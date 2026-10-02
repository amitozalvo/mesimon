#!/usr/bin/env python3
"""T-573: drive the spike mod against the real Claude Code under a private tmux.

Each scenario starts `claude --plugin-dir <mod>` (Haiku, `--setting-sources ""`,
a fresh scratch cwd, its trust dialog answered by key) in a pane of a tmux
server of its own, sends prompts the way mesimon does (bracketed paste, then
a separate Enter), drops commands into the mod's spool, and waits on the
mod's event log. Nothing here touches the user's tmux, mesimon's sockets or
the daemon. Output: <out>/<scenario>/ with the log, screens and summary.json.

  python3 drive.py --mod ~/.local/state/mesimon/<proj16>/mod-spike \
      --out <dir> coverage submit ask permission gate plan_result plan_allow relay load tools
"""
import argparse
import glob
import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
COMPOSER_RULE = "─" * 8
PARITY_EVENTS = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop",
                 "SubagentStart", "SubagentStop", "PreCompact", "PostCompact", "SessionEnd",
                 "Notification", "PermissionRequest", "PostToolUseFailure", "TaskCreated", "TaskCompleted"]


def parity_hook(args):
    """`drive.py parity-hook --log F --event E`: a command hook that appends its stdin."""
    payload = sys.stdin.read()
    with open(args.log, "a") as f:
        f.write(json.dumps({"t": time.time() * 1000, "event": args.event, "payload": json.loads(payload or "{}")}) + "\n")


class Session:
    def __init__(self, ctx, name, mode="default", argv=(), env=None, parity=False):
        self.ctx, self.name, self.mode = ctx, name, mode
        self.dir = ctx.out / ctx.scenario / name
        self.log = self.dir / "log"
        self.spool = self.dir / "spool"
        self.cwd = self.dir / "cwd"
        for d in (self.log, self.spool, self.cwd):
            d.mkdir(parents=True, exist_ok=True)
        # sun_path is 104 bytes on macOS: the sockets live under a short dir.
        short = Path(f"/tmp/msmn-spike-{os.getuid()}")
        short.mkdir(mode=0o700, exist_ok=True)
        self.sock = str(short / f"{ctx.scenario}-{name}-{os.getpid()}.sock")
        if os.path.exists(self.sock):
            os.unlink(self.sock)
        self.sid = str(uuid.uuid4())
        self.screens = 0
        self.seen = 0
        self.extra_argv = list(argv)
        self.env = {"CLAUDE_CODE_PLUGIN_DIR_WATCH": "0", "MESIMON_MOD_LOG": str(self.log),
                    "MESIMON_MOD_SPOOL": str(self.spool), "MESIMON_MOD_SESSION": self.sid, **(env or {})}
        self.settings = None
        if parity:
            self.parity_log = self.dir / "parity.jsonl"
            hooks = {ev: [{"hooks": [{"type": "command", "command": shlex.join(
                [sys.executable, str(HERE / "drive.py"), "parity-hook", "--log", str(self.parity_log), "--event", ev]), "timeout": 5}]}] for ev in PARITY_EVENTS}
            self.settings = self.dir / "settings.json"
            self.settings.write_text(json.dumps({"hooks": hooks}))

    def tmux(self, *args, timeout=5):
        return subprocess.run(["tmux", "-S", self.sock, *args], capture_output=True, text=True, timeout=timeout)

    def start(self):
        argv = ["claude", "--plugin-dir", str(self.ctx.mod), "--model", "haiku", "--setting-sources", "",
                "--session-id", self.sid, "--permission-mode", self.mode]
        if self.settings:
            argv += ["--settings", str(self.settings)]
        argv += self.extra_argv
        env_args = []
        for k, v in self.env.items():
            env_args += ["-e", f"{k}={v}"]
        r = subprocess.run(["tmux", "-S", self.sock, "-f", "/dev/null", "new-session", "-d", "-s", self.name,
                            "-x", "120", "-y", "50", "-c", str(self.cwd), *env_args, shlex.join(argv)],
                           capture_output=True, text=True, timeout=10)
        if r.returncode != 0:
            raise RuntimeError(f"tmux new-session: {r.stderr}")
        self.t0 = time.time()
        (self.dir / "argv.txt").write_text(shlex.join(argv) + "\n")
        return self

    def alive(self):
        return self.tmux("has-session", "-t", self.name).returncode == 0

    def screen(self, tag=None):
        text = self.tmux("capture-pane", "-p", "-t", self.name).stdout
        if tag:
            self.screens += 1
            (self.dir / f"screen-{self.screens:02d}-{tag}.txt").write_text(text)
        return text

    def composer(self, text):
        lines = text.split("\n")
        for i in range(len(lines) - 1, 0, -1):
            body = lines[i].lstrip()
            if not body.startswith("❯"):
                continue
            if COMPOSER_RULE not in lines[i - 1]:
                continue
            if any(COMPOSER_RULE in l for l in lines[i + 1:]):
                rest = body[1:].strip()
                # The placeholder is nothing: `❯ Try "edit <filepath> to..."`.
                return "holding" if rest and not rest.startswith('Try "') else "empty"
        return None

    def wait_composer(self, timeout=40):
        """Answer the trust dialog if it shows, then wait for an empty composer."""
        end = time.time() + timeout
        trusted_at = 0
        while time.time() < end:
            s = self.screen()
            if "trust this folder" in s and time.time() - trusted_at > 1.0:
                sel = [l for l in s.split("\n") if l.lstrip().startswith("❯")]
                if sel and "Yes" in sel[0]:
                    self.tmux("send-keys", "-t", self.name, "Enter")
                    self.note("trust_dialog_answered")
                else:
                    self.tmux("send-keys", "-t", self.name, "Up")
                trusted_at = time.time()
                time.sleep(0.5)
                continue
            if self.composer(s) == "empty":
                return True
            time.sleep(0.25)
        self.screen("no-composer")
        raise TimeoutError(f"{self.name}: no composer within {timeout}s")

    def prompt(self, text):
        """mesimon's road: a bracketed paste, then a SEPARATE Enter."""
        self.wait_composer()
        f = self.dir / "paste.txt"
        f.write_text(text)
        self.tmux("load-buffer", "-b", "spike", str(f))
        self.tmux("paste-buffer", "-p", "-d", "-b", "spike", "-t", self.name)
        time.sleep(0.4)
        self.tmux("send-keys", "-t", self.name, "Enter")
        self.note(f"prompt_sent: {text[:60]}")

    def keys(self, *keys):
        self.tmux("send-keys", "-t", self.name, *keys)
        self.note(f"keys: {keys}")

    def command(self, text):
        self.wait_composer()
        self.tmux("send-keys", "-t", self.name, "-l", text)
        time.sleep(0.6)
        self.tmux("send-keys", "-t", self.name, "Enter")
        time.sleep(0.6)
        s = self.screen()
        if self.composer(s) == "holding":
            self.tmux("send-keys", "-t", self.name, "Enter")
        self.note(f"command: {text}")

    def spool_cmd(self, cmd):
        n = len(list(self.spool.glob("*.json"))) + 1
        tmp = self.spool / f".{n:03d}.tmp"
        tmp.write_text(json.dumps(cmd))
        tmp.rename(self.spool / f"{n:03d}.json")
        self.note(f"spool: {json.dumps(cmd)[:80]}")
        return time.time() * 1000

    def events(self):
        out = []
        for p in sorted(self.log.glob("*.json")):
            try:
                out.append(json.loads(p.read_text()))
            except ValueError:
                pass
        return out

    def wait_event(self, name, timeout=60, where=None, after_seq=None):
        end = time.time() + timeout
        floor = self.seen if after_seq is None else after_seq
        while time.time() < end:
            for ev in self.events():
                if ev["seq"] <= floor or ev["event"] != name:
                    continue
                if where and not where(ev["data"]):
                    continue
                self.seen = max(self.seen, ev["seq"])
                return ev
            time.sleep(0.2)
        self.screen(f"timeout-{name}")
        raise TimeoutError(f"{self.name}: no {name} within {timeout}s")

    def mark(self):
        """Every event so far is old: the next wait starts after it."""
        evs = self.events()
        self.seen = max([e["seq"] for e in evs] + [self.seen])

    def note(self, text):
        with open(self.dir / "notes.txt", "a") as f:
            f.write(f"{time.time() * 1000:.0f} {text}\n")

    def transcript(self):
        for ev in self.events():
            if ev["event"] == "classic.SessionStart":
                path = ev["data"].get("transcript_path")
                if path and os.path.exists(path):
                    return [json.loads(l) for l in open(path) if l.strip()]
        return []

    def end(self, timeout=15):
        if not self.alive():
            return
        self.screen("before-exit")
        self.tmux("send-keys", "-t", self.name, "C-d")
        end = time.time() + timeout
        while time.time() < end and self.alive():
            time.sleep(0.3)
        if self.alive():
            self.screen("exit-stuck")
            self.tmux("send-keys", "-t", self.name, "C-c")
            time.sleep(0.3)
            self.tmux("send-keys", "-t", self.name, "C-c")
            time.sleep(1.5)
        self.tmux("kill-server")
        time.sleep(0.5)
        shutil.copy(self.dir / "notes.txt", self.dir / "notes.copy.txt") if (self.dir / "notes.txt").exists() else None


class Ctx:
    def __init__(self, args):
        self.mod = Path(args.mod).resolve()
        self.out = Path(args.out).resolve()
        self.scenario = ""
        self.summary = {}

    def say(self, text):
        print(f"[{self.scenario}] {text}", flush=True)

    def done(self, **kw):
        self.summary.update(kw)
        (self.out / self.scenario / "summary.json").write_text(json.dumps(self.summary, indent=1, default=str))
        self.summary = {}


def answer_of(ev):
    return (ev["data"].get("answer") or "")[:200]


def user_rows(transcript):
    return [r for r in transcript if r.get("type") == "user" and isinstance(r.get("message"), dict)]


def content_text(row):
    c = row["message"].get("content")
    if isinstance(c, str):
        return c
    return "\n".join(b.get("text", "") for b in c if isinstance(b, dict) and b.get("type") == "text")


# ---------------------------------------------------------------- scenarios

def sc_coverage(ctx):
    s = Session(ctx, "main", mode="default", argv=["--allowedTools", "Bash", "Agent"], parity=True).start()
    s.prompt("Reply with exactly SPIKE_DONE and nothing else. Do not use any tool.")
    t1 = s.wait_event("turn.complete")
    ctx.say(f"turn 1: {answer_of(t1)}")
    s.prompt("Using the Bash tool with run_in_background set to true, run this command: sleep 4; echo bg_done. Then reply BG_STARTED and nothing else.")
    t2 = s.wait_event("turn.complete")
    ctx.say(f"turn 2: {answer_of(t2)}")
    try:
        n = s.wait_event("prompt.submit", timeout=30, where=lambda d: d.get("origin", {}).get("kind") == "task-notification")
        ctx.say(f"task notification arrived as prompt.submit origin={n['data']['origin']}")
        s.wait_event("turn.complete", timeout=60)
    except TimeoutError as e:
        ctx.say(str(e))
    s.prompt('Use the Agent tool with subagent_type "Explore" and the prompt "Reply with the single word hello and nothing else." Then reply SUB_DONE and nothing else.')
    t3 = s.wait_event("turn.complete", timeout=120, where=lambda d: not d.get("agentId"))
    ctx.say(f"turn 3: {answer_of(t3)}")
    s.command("/compact")
    try:
        c = s.wait_event("session.compact.result", timeout=90)
        ctx.say(f"compact: {c['data']}")
    except TimeoutError as e:
        ctx.say(str(e))
    s.spool_cmd({"kind": "tools"})
    s.spool_cmd({"kind": "usage"})
    s.wait_event("session.usage", timeout=20)
    s.end()
    s.wait_event("session.end", timeout=10)
    ctx.done(events=[e["event"] for e in s.events()])


def brief_text(tag, kb=10):
    head = f"Reply with exactly BRIEF_OK_{tag} and nothing else. Do not use any tool. Everything after this line is filler to make this brief {kb} KB long; ignore it.\n"
    filler = ""
    i = 0
    while len(head.encode()) + len(filler.encode()) < kb * 1024:
        filler += f"filler line {i}: the quick brown fox jumps over the lazy dog, again and again.\n"
        i += 1
    return head + filler + f"End of filler. Reply with exactly BRIEF_OK_{tag} and nothing else."


def sc_submit(ctx):
    brief = ctx.out / ctx.scenario / "brief.txt"
    brief.parent.mkdir(parents=True, exist_ok=True)
    text = brief_text("LAUNCH")
    brief.write_text(text)
    s = Session(ctx, "main", mode="default", argv=["--allowedTools", "Bash"], env={"MESIMON_MOD_BRIEF_FILE": str(brief)}).start()
    s.wait_composer()  # the trust dialog comes before session.start
    r = s.wait_event("prompt.submit.resolved", timeout=60)
    ctx.say(f"launch brief resolved in {r['data']['ms']} ms, origin={r['data'].get('origin')}")
    t1 = s.wait_event("turn.complete", timeout=90)
    ctx.say(f"turn 1: {answer_of(t1)}")
    s.screen("after-launch-brief")
    # Framed: without asUser.
    s.spool_cmd({"kind": "submit", "text": "Reply with exactly FRAMED_OK and nothing else.", "asUser": False})
    t2 = s.wait_event("turn.complete", timeout=90)
    ctx.say(f"turn 2 (framed): {answer_of(t2)}")
    s.screen("after-framed")
    # Mid-turn: a submit while Bash sleeps.
    s.prompt("Use the Bash tool to run exactly: sleep 6. Then reply SLEPT and nothing else.")
    s.wait_event("classic.PreToolUse", timeout=60, where=lambda d: d.get("tool") == "Bash")
    at = s.spool_cmd({"kind": "submit", "text": "Reply with exactly QUEUED_OK and nothing else."})
    ps = s.wait_event("prompt.submit", timeout=60, where=lambda d: d.get("head", "").startswith("Reply with exactly QUEUED_OK"))
    ctx.say(f"mid-turn submit: prompt.submit fired {ps['t'] - at:.0f} ms after the spool, turnId={ps['data'].get('turnId')}")
    r2 = s.wait_event("prompt.submit.resolved", timeout=60, where=lambda d: d.get("why") == "spool")
    ctx.say(f"mid-turn submit resolved after {r2['data']['ms']} ms")
    s.wait_event("turn.complete", timeout=90, where=lambda d: "QUEUED_OK" in (d.get("answer") or ""))
    s.spool_cmd({"kind": "usage"})
    s.wait_event("session.usage", timeout=20)
    s.end()
    tr = s.transcript()
    rows = user_rows(tr)
    first = content_text(rows[0]) if rows else ""
    ctx.done(
        launch_brief_equal=(first == text),
        launch_brief_chars=len(first),
        first_user_row_keys=sorted(rows[0].keys()) if rows else [],
        first_user_row_meta={k: rows[0].get(k) for k in ("isMeta", "origin", "userType", "promptOrigin") if k in rows[0]} if rows else {},
        plugin_named_in_first_row=("mesimon-spike" in json.dumps(rows[0])) if rows else None,
        framed_row_text=[content_text(r)[:200] for r in rows if "FRAMED_OK" in content_text(r)],
        queued_row_text=[content_text(r)[:200] for r in rows if "QUEUED_OK" in content_text(r)],
        all_user_rows=[content_text(r)[:100] for r in rows],
    )


def sc_submit_midturn(ctx):
    """Row 2: a prompt submitted while a turn runs, and the hold's timing."""
    s = Session(ctx, "main", mode="default", argv=["--allowedTools", "Bash"]).start()
    s.prompt("Use the Bash tool to run exactly: sleep 8. Then reply SLEPT and nothing else.")
    s.wait_event("classic.PreToolUse", timeout=60, where=lambda d: d.get("tool") == "Bash")
    at = s.spool_cmd({"kind": "submit", "text": "Reply with exactly QUEUED_OK and nothing else."})
    time.sleep(1.5)
    s.screen("while-held")
    r2 = s.wait_event("prompt.submit.resolved", timeout=60, where=lambda d: d.get("why") == "spool")
    ctx.say(f"mid-turn submit resolved {r2['t'] - at:.0f} ms after the spool (call->resolve {r2['data']['ms']} ms)")
    t1 = s.wait_event("turn.complete", timeout=90, where=lambda d: "SLEPT" in (d.get("answer") or ""))
    t2 = s.wait_event("turn.complete", timeout=90, where=lambda d: "QUEUED_OK" in (d.get("answer") or ""))
    ctx.say(f"SLEPT turn ended at +{t1['t'] - at:.0f} ms, QUEUED_OK turn ended at +{t2['t'] - at:.0f} ms")
    s.screen("after-both")
    s.end()
    tr = s.transcript()
    ctx.done(resolved_after_spool_ms=r2["t"] - at, slept_done_ms=t1["t"] - at, queued_done_ms=t2["t"] - at,
             turn_starts=[e["data"].get("text", "")[:60] for e in s.events() if e["event"] == "turn.start"],
             queue_ops=[json.dumps(r)[:200] for r in tr if r.get("type") == "queue-operation"],
             user_rows=[(r.get("origin"), content_text(r)[:80]) for r in user_rows(tr)])


def sc_load(ctx):
    sessions = []
    # `yes` directly, never `sh -c "yes > /dev/null"`: kill() reaches the
    # direct child only, and a `yes` behind a shell outlived the run as a
    # launchd orphan at 100% of a core, six per run (T-585).
    burners = [subprocess.Popen(["yes"], stdout=subprocess.DEVNULL) for _ in range(6)]
    try:
        for i in range(3):
            brief = ctx.out / ctx.scenario / f"brief-{i}.txt"
            brief.parent.mkdir(parents=True, exist_ok=True)
            brief.write_text(brief_text(f"S{i}"))
            sessions.append(Session(ctx, f"s{i}", mode="default", env={"MESIMON_MOD_BRIEF_FILE": str(brief)}).start())
        results = {}
        for s in sessions:
            s.wait_composer()  # the trust dialog comes before session.start
        for i, s in enumerate(sessions):
            r = s.wait_event("prompt.submit.resolved", timeout=90)
            t = s.wait_event("turn.complete", timeout=120)
            tr_rows = []
            results[s.name] = {"resolved_ms": r["data"]["ms"], "answer": answer_of(t), "submit_after_start_ms": r["t"] - s.t0 * 1000}
    finally:
        for b in burners:
            b.kill()
        for b in burners:
            b.wait()
    for s in sessions:
        s.end()
        rows = user_rows(s.transcript())
        results[s.name]["brief_equal"] = bool(rows) and content_text(rows[0]) == (ctx.out / ctx.scenario / f"brief-{s.name[1:]}.txt").read_text()
    ctx.done(sessions=results)


ASK_ONE = 'Use the AskUserQuestion tool to ask me exactly one question: "Which color?" with the header "Color" and the options "Red" and "Blue". After I answer, reply with COLOR=<my answer> and nothing else.'
ASK_BATCH = ('Call the AskUserQuestion tool once with two questions. Question 1: "Which size?", header "Size", options "Small" and "Large", multiSelect false. '
             'Question 2: "Which toppings?", header "Toppings", options "Cheese", "Olives" and "Ham", multiSelect true. '
             'After I answer, reply with SIZE=<answer 1>; TOPPINGS=<answer 2> and nothing else.')


def sc_ask(ctx):
    s = Session(ctx, "main", mode="default").start()
    # Round 1: the spool answers first.
    s.prompt(ASK_ONE)
    s.wait_event("ask.call", timeout=60)
    time.sleep(1.5)
    s.screen("r1-dialog")
    at = s.spool_cmd({"kind": "answer", "answers": {"Which color?": "Blue"}})
    a = s.wait_event("ask.answered_by_spool", timeout=20)
    time.sleep(1.0)
    s.screen("r1-after-answer")
    t1 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"r1 answered by spool after {a['t'] - at:.0f} ms; model said: {answer_of(t1)}")
    native_after = [e for e in s.events() if e["event"] == "ask.native_after_spool"]
    # Round 2: the person answers first (Down, Enter = Blue), then the spool is late.
    s.prompt(ASK_ONE)
    s.wait_event("ask.call", timeout=60)
    time.sleep(1.5)
    s.screen("r2-dialog")
    s.keys("Down")
    time.sleep(0.4)
    s.keys("Enter")
    n = s.wait_event("ask.native", timeout=20)
    s.spool_cmd({"kind": "answer", "answers": {"Which color?": "Red"}})
    s.wait_event("answer.nothing_held", timeout=10)
    t2 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"r2 native: {json.dumps(n['data'])[:300]}; model said: {answer_of(t2)}")
    # Round 3: a batch with a multi-select, answered whole by the spool.
    s.prompt(ASK_BATCH)
    s.wait_event("ask.call", timeout=60)
    time.sleep(1.5)
    s.screen("r3-dialog")
    s.spool_cmd({"kind": "answer", "answers": {"Which size?": "Large", "Which toppings?": "Cheese, Ham"}})
    s.wait_event("ask.answered_by_spool", timeout=20)
    time.sleep(1.0)
    s.screen("r3-after-answer")
    t3 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"r3 model said: {answer_of(t3)}")
    # Row 7 in default mode: the registered tool.
    s.spool_cmd({"kind": "tools"})
    tl = s.wait_event("tool.list", timeout=20)
    s.prompt('Call the spike_ping tool with note "hi" and reply with its result verbatim and nothing else.')
    t4 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"spike_ping: {answer_of(t4)}")
    s.screen("after-ping")
    s.end()
    tr = s.transcript()
    results = [r for r in tr if r.get("type") == "user" and isinstance(r.get("message", {}).get("content"), list)
               and any(b.get("type") == "tool_result" for b in r["message"]["content"])]
    ctx.done(
        r1=answer_of(t1), r2=answer_of(t2), r3=answer_of(t3), ping=answer_of(t4),
        native_after_spool=[e["data"] for e in native_after],
        tool_results=[(b.get("tool_use_id"), str(b.get("content"))[:300]) for r in results for b in r["message"]["content"] if b.get("type") == "tool_result"],
        tool_result_records=[str(r.get("toolUseResult"))[:400] for r in results],
        tools=[t for t in tl["data"] if "spike" in t],
        checks=[e["data"] for e in s.events() if e["event"] == "tool.check"],
    )


def sc_permission(ctx):
    # `printf` is allowed by Claude Code's own read-only classifier (measured:
    # core said allow, no dialog), so the prompting command is a write.
    s = Session(ctx, "main", mode="default", parity=True).start()
    s.prompt("Use the Bash tool to run exactly: touch one.txt. Then reply TOUCHED and nothing else.")
    c = s.wait_event("tool.check", timeout=60, where=lambda d: d["tool"] == "Bash")
    ctx.say(f"tool.check core: {c['data']['core']}")
    try:
        pr = s.wait_event("classic.PermissionRequest", timeout=15)
        ctx.say(f"PermissionRequest after tool.check: {pr['t'] - c['t']:.0f} ms; suggestions={pr['data'].get('permission_suggestions')}")
    except TimeoutError as e:
        ctx.say(str(e))
    time.sleep(1.0)
    s.screen("native-dialog")
    s.keys("Enter")
    t1 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"turn 1: {answer_of(t1)}")
    s.spool_cmd({"kind": "check", "tool": "Bash", "decision": "allow"})
    s.prompt("Use the Bash tool to run exactly: touch two.txt. Then reply TOUCHED and nothing else.")
    s.mark()
    t2 = s.wait_event("turn.complete", timeout=60)
    allowed_events = [e["event"] for e in s.events() if e["seq"] > t1["seq"] and e["seq"] <= t2["seq"]]
    ctx.say(f"turn 2 (allow): {answer_of(t2)}; events: {allowed_events}")
    s.spool_cmd({"kind": "check", "tool": "Bash", "decision": "deny"})
    s.prompt("Use the Bash tool to run exactly: touch three.txt. Then reply with the exact error text the tool returned and nothing else.")
    t3 = s.wait_event("turn.complete", timeout=60)
    denied_events = [e["event"] for e in s.events() if e["seq"] > t2["seq"] and e["seq"] <= t3["seq"]]
    ctx.say(f"turn 3 (deny): {answer_of(t3)}; events: {denied_events}")
    s.end()
    ctx.done(turn1=answer_of(t1), turn2=answer_of(t2), turn3=answer_of(t3), allow_events=allowed_events, deny_events=denied_events,
             files=sorted(p.name for p in s.cwd.iterdir()),
             checks=[e["data"] for e in s.events() if e["event"] == "tool.check"],
             permission_requests=[{k: e["data"].get(k) for k in ("tool_name", "permission_suggestions")} for e in s.events() if e["event"] == "classic.PermissionRequest"],
             notifications=[e["data"].get("notification_type") for e in s.events() if e["event"] == "classic.Notification"])


def sc_permit(ctx):
    """Row 4: the one-shot allow beside an open dialog, from classic.PermissionRequest."""
    s = Session(ctx, "main", mode="default", parity=True).start()
    s.wait_composer()
    permits = s.spool / "permits"
    permits.mkdir(exist_ok=True)
    s.spool_cmd({"kind": "hold_permits", "on": True})
    time.sleep(0.8)

    def permit(n, decision):
        tmp = permits / f".{n}.tmp"
        tmp.write_text(json.dumps(decision))
        tmp.rename(permits / f"{n}.json")
        s.note(f"permit {n}: {decision}")
        return time.time() * 1000

    # Round 1: the daemon's allow while the dialog is up.
    s.prompt("Use the Bash tool to run exactly: touch one.txt. Then reply TOUCHED and nothing else.")
    h = s.wait_event("permit.hold", timeout=60)
    time.sleep(1.5)
    s.screen("r1-dialog-while-held")
    at = permit(1, {"behavior": "allow"})
    a = s.wait_event("permit.answered", timeout=20)
    time.sleep(1.0)
    s.screen("r1-after-allow")
    t1 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"r1 allow answered {a['t'] - at:.0f} ms after the file; model: {answer_of(t1)}")
    # Round 2: the person answers first (Enter on Yes) while the hook holds.
    s.prompt("Use the Bash tool to run exactly: touch two.txt. Then reply TOUCHED and nothing else.")
    s.wait_event("permit.hold", timeout=60)
    time.sleep(1.5)
    s.keys("Enter")
    t2 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"r2 person first; model: {answer_of(t2)}")
    time.sleep(1.0)
    hook_after = [e["event"] for e in s.events() if e["event"].startswith("permit.") and e["seq"] > t2["seq"] - 20]
    ctx.say(f"r2 hook events so far: {hook_after}")
    permit(2, {"behavior": "allow"})  # releases the wait, if it still runs
    try:
        w = s.wait_event("permit.wait_done", timeout=10)
        ctx.say(f"r2 wait_done: {w['data']}")
    except TimeoutError as e:
        ctx.say(str(e))
    try:
        w = s.wait_event("permit.wait_failed", timeout=3)
        ctx.say(f"r2 wait_failed: {w['data']}")
    except TimeoutError:
        pass
    # Round 3: the daemon's deny.
    s.prompt("Use the Bash tool to run exactly: touch three.txt. Then reply with the exact error text the tool returned and nothing else.")
    s.wait_event("permit.hold", timeout=60)
    time.sleep(1.0)
    permit(3, {"behavior": "deny", "message": "mesimon spike: the phone said no"})
    t3 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"r3 deny; model: {answer_of(t3)}")
    s.screen("r3-after-deny")
    s.end()
    ctx.done(files=sorted(p.name for p in s.cwd.iterdir()), r1=answer_of(t1), r2=answer_of(t2), r3=answer_of(t3),
             permit_events=[(e["event"], e["data"]) for e in s.events() if e["event"].startswith("permit.")],
             parity=[json.loads(l)["event"] for l in open(s.parity_log)])


def sc_gate(ctx):
    s = Session(ctx, "main", mode="acceptEdits").start()
    s.prompt("Use the Write tool to create the file .mesimon/spike.txt (relative to the current directory) with the content hi. Then reply with the exact error or result text the tool returned and nothing else.")
    g = s.wait_event("gate.deny", timeout=60)
    t1 = s.wait_event("turn.complete", timeout=60)
    ctx.say(f"gate: {g['data']}; model said: {answer_of(t1)}")
    s.prompt("Use the Write tool to create the file ok.txt with the content hi. Then reply WROTE and nothing else.")
    t2 = s.wait_event("turn.complete", timeout=60)
    s.end()
    tr = s.transcript()
    results = [b for r in tr if r.get("type") == "user" and isinstance(r.get("message", {}).get("content"), list)
               for b in r["message"]["content"] if b.get("type") == "tool_result"]
    ctx.done(denied_file_exists=(s.cwd / ".mesimon" / "spike.txt").exists(), ok_file_exists=(s.cwd / "ok.txt").exists(),
             turn1=answer_of(t1), turn2=answer_of(t2), tool_results=[str(b.get("content"))[:300] for b in results],
             checks=[e["data"] for e in s.events() if e["event"] == "tool.check"])


def plan_scenario(ctx, mode):
    s = Session(ctx, "main", mode="plan").start()
    s.spool_cmd({"kind": "plan", "mode": mode})
    time.sleep(1.0)
    s.prompt("Make a short plan to create a file named a.txt containing the word hi. Then call the ExitPlanMode tool.")
    p = s.wait_event("plan.call", timeout=90)
    ctx.say(f"plan.call e keys: {sorted(p['data']['e'].keys())}")
    time.sleep(1.5)
    s.screen("plan-dialog-or-not")
    t1 = s.wait_event("turn.complete", timeout=90)
    time.sleep(1.0)
    s.screen("after-plan")
    ctx.say(f"turn 1: {answer_of(t1)}")
    s.prompt("Now create a.txt with the Write tool exactly as planned, then reply WROTE and nothing else.")
    t2 = s.wait_event("turn.complete", timeout=90)
    s.screen("after-write")
    ctx.say(f"turn 2: {answer_of(t2)}")
    s.end()
    ctx.done(mode=mode, plan_events=[e["event"] for e in s.events() if e["event"].startswith("plan.") or e["event"] == "tool.check"],
             checks=[e["data"] for e in s.events() if e["event"] == "tool.check"],
             plan_result=[e["data"] for e in s.events() if e["event"] in ("plan.answered_by_mod", "plan.native")],
             a_txt_exists=(s.cwd / "a.txt").exists(), turn1=answer_of(t1), turn2=answer_of(t2),
             classic_after_plan=[e["event"] for e in s.events() if e["seq"] > p["seq"] and e["event"].startswith("classic.")])


def sc_plan_result(ctx):
    plan_scenario(ctx, "result")


def sc_plan_allow(ctx):
    plan_scenario(ctx, "allow")


def sc_plan_native(ctx):
    s = Session(ctx, "main", mode="plan").start()
    s.prompt("Make a short plan to create a file named a.txt containing the word hi. Then call the ExitPlanMode tool.")
    p = s.wait_event("plan.call", timeout=90)
    time.sleep(1.5)
    s.screen("plan-dialog")
    s.keys("Enter")
    t1 = s.wait_event("turn.complete", timeout=90)
    s.end()
    ctx.done(plan_result=[e["data"] for e in s.events() if e["event"] == "plan.native"], turn1=answer_of(t1),
             checks=[e["data"] for e in s.events() if e["event"] == "tool.check"])


def sc_tools(ctx):
    """Row 7 in plan mode: is the in-process tool admitted?"""
    s = Session(ctx, "main", mode="plan").start()
    s.wait_composer()  # the trust dialog comes before session.start
    s.spool_cmd({"kind": "tools"})
    tl = s.wait_event("tool.list", timeout=20)
    s.prompt('Call the spike_ping tool with note "plan" and reply with its result verbatim and nothing else.')
    t1 = s.wait_event("turn.complete", timeout=60)
    time.sleep(1.0)
    s.screen("after-ping")
    s.end()
    ctx.done(tools=[t for t in tl["data"] if "spike" in t], turn1=answer_of(t1),
             checks=[e["data"] for e in s.events() if e["event"] == "tool.check"],
             ping_calls=[e["data"] for e in s.events() if e["event"] == "spike_ping.call"])


def sc_relay(ctx):
    d = ctx.out / ctx.scenario
    d.mkdir(parents=True, exist_ok=True)
    sock = Path(f"/tmp/msmn-spike-{os.getuid()}") / f"hook-{os.getpid()}.sock"
    sock.parent.mkdir(mode=0o700, exist_ok=True)
    if sock.exists():
        sock.unlink()
    received = []
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(str(sock))
    srv.listen(64)

    def serve():
        while True:
            try:
                c, _ = srv.accept()
            except OSError:
                return
            buf = b""
            c.settimeout(3)
            try:
                while True:
                    chunk = c.recv(65536)
                    if not chunk:
                        break
                    buf += chunk
            except OSError:
                pass
            c.close()
            header, _, payload = buf.partition(b"\n")
            try:
                h = json.loads(header)
            except ValueError:
                h = {"raw": header[:100].decode(errors="replace")}
            received.append({"t": time.time() * 1000, "header": h, "payload_bytes": len(payload)})

    threading.Thread(target=serve, daemon=True).start()
    bin_path = ctx.mesimon_bin
    s = Session(ctx, "main", mode="default", env={"MESIMON_MOD_RELAY_SOCK": str(sock), "MESIMON_MOD_RELAY_BIN": str(bin_path)}).start()
    s.prompt("Reply with exactly SPIKE_DONE and nothing else. Do not use any tool.")
    s.wait_event("turn.complete", timeout=60)
    s.wait_event("relay.done", timeout=20, where=lambda dd: dd["event"] == "Stop")
    s.end()
    time.sleep(1.0)
    srv.close()
    relays = [e for e in s.events() if e["event"] in ("relay.done", "relay.failed")]
    ctx.done(received=received, relays=[{"event": e["data"].get("event"), "ms": e["data"].get("ms"), "exit": e["data"].get("exitCode"), "err": e["data"].get("error")} for e in relays],
             classic=[e["event"] for e in s.events() if e["event"].startswith("classic.")])


SCENARIOS = {"coverage": sc_coverage, "submit": sc_submit, "submit_midturn": sc_submit_midturn, "load": sc_load, "ask": sc_ask, "permission": sc_permission,
             "gate": sc_gate, "permit": sc_permit, "plan_result": sc_plan_result, "plan_allow": sc_plan_allow, "plan_native": sc_plan_native,
             "tools": sc_tools, "relay": sc_relay}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd")
    ph = sub.add_parser("parity-hook")
    ph.add_argument("--log", required=True)
    ph.add_argument("--event", required=True)
    run = sub.add_parser("run")
    run.add_argument("--mod", required=True)
    run.add_argument("--out", required=True)
    run.add_argument("--mesimon-bin", default=str(HERE.parents[2] / "target" / "debug" / "mesimon"))
    run.add_argument("scenarios", nargs="+", choices=sorted(SCENARIOS))
    args = ap.parse_args()
    if args.cmd == "parity-hook":
        return parity_hook(args)
    ctx = Ctx(args)
    ctx.mesimon_bin = Path(args.mesimon_bin)
    for name in args.scenarios:
        ctx.scenario = name
        (ctx.out / name).mkdir(parents=True, exist_ok=True)
        ctx.say("start")
        t0 = time.time()
        try:
            SCENARIOS[name](ctx)
            ctx.say(f"done in {time.time() - t0:.0f}s")
        except Exception as e:  # a scenario that fails still leaves its log
            ctx.say(f"FAILED: {e!r}")
            ctx.done(failed=repr(e))
            subprocess.run(["sh", "-c", f"for s in /tmp/msmn-spike-{os.getuid()}/{name}-*-{os.getpid()}.sock; do tmux -S \"$s\" kill-server 2>/dev/null; done"])


if __name__ == "__main__":
    main()
