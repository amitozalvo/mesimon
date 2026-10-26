#!/usr/bin/env python3
"""The demo board (T-1032): a second mesimon board, on a repository of its
own, that builds a small app under a crown while a tester steers it in a
person's words, watches it and scores it.

    python3 -B ci/demo/board.py lay [--repo R] [--bin B]   a repo, a daemon, a crown waiting
    python3 -B ci/demo/board.py say "<words>"              the person's words to the crown
    python3 -B ci/demo/board.py watch                      what changed since the last watch
    python3 -B ci/demo/board.py wait [--for S]             until the crown idles or a card needs you
    python3 -B ci/demo/board.py trust <KEY>                the person's accept on a trust dialog
    python3 -B ci/demo/board.py score <what> m s w q h "<note>"  one scorecard row (0-2, or -)
    python3 -B ci/demo/board.py stop                       park, shut the daemon down; keep the repo
    python3 -B ci/demo/board.py reset                      stop, then wipe the repo and the board

It is not a rig test: nothing here asserts, it only lays, speaks and reads.
It writes to the demo board only through the person's own commands, as
`Local`, over the demo daemon's `orch.sock` (`ci/rig/wire.py`); it reads the
snapshot, the feed, the daemon journal and the transcripts. It writes
nothing under the machine layer: every tier is a board tier.

The merge train is armed per connection and a closed board is a stopped
train (`SetAutomation`), so `lay` leaves a small holder process beside the
daemon (`hold`), standing in for the person's open TUI; `stop` ends it.
The episode's own files (the watch offsets, the scorecard) live in the
demo board's state dir under `demo/`.
"""

import argparse
import datetime
import json
import os
import shutil
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
sys.path.insert(0, os.path.join(HERE, "..", "rig"))
from wire import Subscription, Wire, WireError, board_paths, read_feed, ulid  # noqa: E402
import rig  # noqa: E402

DEFAULT_REPO = os.path.expanduser("~/Documents/code/demo-ledger")
# The board tiers (T-1032): the crown's seat alone on Opus, the workers on
# Sonnet by default and Haiku for the small ones. Their descriptions are the
# person's words a crown reads in list_board.
TIERS = (
    ("crown", "claude-opus-5-5", "medium", "the crown's seat alone, never a worker"),
    ("coder", "claude-sonnet-5-5", "medium", "features, refactors and anything that needs a plan"),
    ("small", "claude-haiku-5-5", "low", "a small bug fix or a one-file change"),
)
DEFAULT_TIER = "coder"
CROWN_TITLE = "Crown"
README = """# ledger

A personal expense tracker for the command line.
"""
PYPROJECT = """[project]
name = "ledger"
version = "0.0.1"
requires-python = ">=3.11"

[tool.pytest.ini_options]
testpaths = ["tests"]
"""
# Feed commands that say a person's or the crown's words did not land.
LOST_WORDS = ("prompt_submit_refused", "prompt_submit_unreceived", "prompt_submit_not_ready",
              "prompt_resent")


def say(text=""):
    print(text, flush=True)


def die(text):
    say(f"demo: {text}")
    sys.exit(1)


def now_ms():
    return int(time.time() * 1000)


def clock(ms=None):
    t = time.localtime((ms or now_ms()) / 1000)
    return time.strftime("%H:%M:%S", t)


class Demo:
    def __init__(self, repo):
        self.repo = os.path.realpath(repo) if os.path.exists(repo) else os.path.abspath(repo)
        self.paths = board_paths(self.repo) if os.path.exists(repo) else None
        self.dir = os.path.join(self.paths.state_dir, "demo") if self.paths else None
        self.trusted_crown = False

    # ---- the episode's own files

    def load(self):
        try:
            with open(os.path.join(self.dir, "episode.json")) as f:
                return json.load(f)
        except (OSError, ValueError, TypeError):
            return {}

    def save(self, ep):
        os.makedirs(self.dir, exist_ok=True)
        tmp = os.path.join(self.dir, "episode.json.tmp")
        with open(tmp, "w") as f:
            json.dump(ep, f, indent=1)
        os.replace(tmp, os.path.join(self.dir, "episode.json"))

    def wire(self, timeout=15.0, connect_for=5.0):
        return Wire(self.paths.orch_sock, client="demo", timeout=timeout, connect_for=connect_for)

    def alive(self):
        try:
            self.wire(connect_for=0.2).close()
            return True
        except OSError:
            return False

    # ---- lay

    def init_repo(self):
        if os.path.exists(os.path.join(self.repo, ".git")):
            say(f"  repo {self.repo} exists: kept")
            return
        os.makedirs(os.path.join(self.repo, "tests"), exist_ok=True)
        for name, text in (("README.md", README), ("pyproject.toml", PYPROJECT),
                           ("tests/.gitkeep", ""), (".gitignore", "__pycache__/\n*.db\n")):
            with open(os.path.join(self.repo, name), "w") as f:
                f.write(text)
        for args in (("init", "-q", "-b", "main"), ("add", "-A"),
                     ("commit", "-q", "-m", "an empty ledger")):
            out = subprocess.run(["git", "-C", self.repo, *args], capture_output=True, text=True,
                                 env={k: v for k, v in os.environ.items() if not k.startswith("GIT_")})
            if out.returncode != 0:
                die(f"git {args[0]}: {out.stderr.strip()}")
        say(f"  repo {self.repo}: README, pyproject.toml, tests/, one commit on main")

    def start_daemon(self, binary):
        os.makedirs(self.paths.state_dir, mode=0o700, exist_ok=True)
        log = open(self.paths.daemon_log, "ab")
        env = rig.terminal_env({"MESIMON_DETACHED": "1"})
        subprocess.Popen([binary, "daemon", "--repo", self.repo], stdin=subprocess.DEVNULL,
                         stdout=log, stderr=log, env=env, start_new_session=True, cwd=self.repo)
        log.close()
        with self.wire(connect_for=30) as w:
            return w.hello

    def start_holder(self):
        """The train's connection, in a process of its own (see the module's
        words): `hold` re-arms after a daemon restart and ends with `stop`."""
        log = open(os.path.join(self.dir, "hold.log"), "ab")
        p = subprocess.Popen([sys.executable, "-B", os.path.abspath(__file__), "hold",
                              "--repo", self.repo], stdin=subprocess.DEVNULL, stdout=log,
                             stderr=log, start_new_session=True)
        log.close()
        with open(os.path.join(self.dir, "hold.pid"), "w") as f:
            f.write(str(p.pid))
        return p.pid

    def lay(self, binary):
        t0 = time.time()
        say(f"▶ laying the demo board ({clock()})")
        self.init_repo()
        self.paths = board_paths(self.repo)
        self.dir = os.path.join(self.paths.state_dir, "demo")
        os.makedirs(self.dir, exist_ok=True)
        if self.alive():
            die("a demo daemon already runs on this repo: `stop` it first, or `reset`")
        hello = self.start_daemon(binary)
        say(f"  daemon pid {hello['daemon_pid']}, build {hello.get('build')}, {binary}")
        w = self.wire()
        snap = w.snapshot()
        board = snap["board"]
        # Setup's answers as a new user gives them: the kanban template's
        # columns, branches into main, a worktree per ticket, every tool.
        w.request({"cmd": "apply_setup", "template": "kanban", "noun": "PR",
                   "merge_into": "main", "workspace": "worktree", "agent_tools": "full"})
        ids = {}
        # A name the machine layer holds is saved under the machine tier's
        # id: the board's override of it, still on the board layer.
        known = {t["name"]: t for t in (snap.get("machine_tiers") or {}).get("tiers", [])}
        known.update({t["name"]: t for t in board.get("tiers", [])})
        for name, model, effort, words in TIERS:
            spec = {"id": known[name]["id"] if name in known else ulid(), "name": name,
                    "provider": "claude_code", "model": model, "effort": effort,
                    "description": words}
            w.save_tier("board", spec)
            ids[name] = spec["id"]
        w.set_default_tier("board", ids[DEFAULT_TIER])
        w.set_crown_mode("autonomous")
        w.set_crown_watches(True)
        w.request({"cmd": "set_crown_samples", "on": True})
        holder = self.start_holder()
        crown = w.create_ticket("TODO", CROWN_TITLE)
        w.request({"cmd": "set_ticket_tier", "id": crown, "tier": ids["crown"]})
        w.crown(crown)
        key = next(t["short_key"] for t in w.board()["tickets"] if t["id"] == crown)
        cols = [c["name"] for c in w.board()["columns"]]
        say(f"  columns {' · '.join(cols)}; tiers {', '.join(f'{n} ({m} {e})' for n, m, e, _ in TIERS)}; "
            f"default {DEFAULT_TIER}; crown autonomous, watches and samples on; train holder pid {holder}")
        # A launch before the mod's probe lands takes the hook set (rig.py's
        # start_crown): wait for the verdict first.
        road = self.wait_road(120)
        say(f"  road.json: {road}")
        r = w.spawn(crown, "claude", submit_prompt=True)
        say(f"  {key} crowned and started on the board's brief: {r.get('resp')}")
        sid = self.wait_crown_idle(w, crown, 300)
        took = time.time() - t0
        self.save({"repo": self.repo, "bin": binary, "build": hello.get("build"),
                   "crown": crown, "crown_key": key, "crown_session": sid, "laid_ms": now_ms(),
                   "lay_seconds": round(took), "feed_off": read_feed(self.paths)[1],
                   "log_off": os.path.getsize(self.paths.daemon_log), "rows_seen": 0,
                   "said": [], "trusted": [{"at_ms": now_ms(), "key": key}] if self.trusted_crown else []})
        say(f"\n  laid in {took:.0f} s ∙ the crown waits for words" if sid
            else f"\n  laid in {took:.0f} s ∙ the crown has not ended its first turn")
        say(f"  watch it: cd {self.repo} && {binary}")

    def wait_road(self, timeout):
        deadline, line = time.time() + timeout, None
        while time.time() < deadline:
            try:
                with open(os.path.join(self.paths.mod_root, "road.json")) as f:
                    v = json.load(f)
                line = f"{v.get('road')} ∙ {v.get('probe')}"
                if "validated" in line or "failed" in line:
                    return line
            except (OSError, ValueError):
                pass
            time.sleep(0.5)
        return line

    def wait_crown_idle(self, w, crown, timeout):
        sub = Subscription(self.paths.orch_sock)
        deadline = time.time() + timeout
        try:
            while time.time() < deadline:
                rec = agent_of(w.board(), crown)
                if rec and rig.word_of(rec["state"]) == "idle" and rec.get("claude_session_id"):
                    return rec["id"]
                # The person's first press on a new repo: Claude Code's folder
                # trust, answered as `trust` answers it.
                if rec and rec["state"].get("reason") == "trust" and not self.trusted_crown:
                    w.request({"cmd": "accept_trust", "ticket": crown})
                    self.trusted_crown = True
                    say(f"  {clock()} the crown's folder-trust dialog accepted (the person's press)")
                sub.wait(1.0)
        finally:
            sub.close()
        return None

    # ---- hold

    def hold(self):
        """Arm the train as the person's open board does, and keep the
        connection; a daemon gone and back is armed again."""
        while True:
            try:
                w = Wire(self.paths.orch_sock, client="demo-train", connect_for=60)
                w.request({"cmd": "set_automation", "merge_train": True, "merge_notice": True})
                w.request({"cmd": "set_usage_wants", "claude": True, "codex": False})
                w.request({"cmd": "subscribe"})
                say(f"{clock()} train armed")
                w.sock.settimeout(None)
                while True:
                    w._line()
            except (OSError, ConnectionError, WireError, ValueError) as e:
                say(f"{clock()} connection lost: {e}")
                time.sleep(2)

    # ---- say

    def say_words(self, words):
        ep = self.load()
        if not ep:
            die("no episode here: `lay` first")
        with self.wire() as w:
            rec = agent_of(w.board(), ep["crown"])
            busy = rec and rig.word_of(rec["state"]) == "working"
            # The ask field's Enter on a working crown queues; on an idle one
            # it sends now.
            w.prompt(ep["crown"], words, queued=bool(busy))
        ep["said"].append({"at_ms": now_ms(), "words": words, "queued": bool(busy)})
        self.save(ep)
        say(f"{clock()} said{' (queued, the crown is working)' if busy else ''}: {words}")

    # ---- wait

    def wait(self, timeout):
        """Until a crown turn ends after this call (idle for a few seconds), or any card
        needs the person, or `timeout` seconds pass: the tester's pause
        between watches, on the board's pushes, never a poll of the pane."""
        ep = self.load()
        t0, quiet, start_ms = time.time(), None, now_ms()
        sub = Subscription(self.paths.orch_sock)
        try:
            with self.wire() as w:
                while time.time() - t0 < timeout:
                    board = w.board()
                    rec = agent_of(board, ep["crown"])
                    needs = [rec_key(board, s["ticket"]) for s in board["sessions"]
                             if rig.word_of(s["state"]) == "needs_you"]
                    if needs:
                        say(f"{clock()} needs the person: {' '.join(needs)} ({time.time() - t0:.0f} s)")
                        return
                    if rec and rig.word_of(rec["state"]) == "idle" and \
                            rec.get("state_changed_at", 0) >= start_ms:
                        quiet = quiet or time.time()
                        if time.time() - quiet >= 5:
                            say(f"{clock()} the crown is idle ({time.time() - t0:.0f} s)")
                            return
                    else:
                        quiet = None
                    sub.wait(1.0)
        finally:
            sub.close()
        say(f"{clock()} still going after {timeout} s")

    # ---- trust

    def trust(self, key):
        """The person's accept on Claude Code's folder-trust dialog (the
        card's needs-you key): Claude Code asks it once per worktree path,
        and neither the crown nor the board may answer it. Each use is a
        gap row: the person had to come to the desk."""
        ep = self.load()
        with self.wire() as w:
            t = next((t for t in w.board()["tickets"] if t["short_key"] == key), None)
            if not t:
                die(f"no ticket {key}")
            w.request({"cmd": "accept_trust", "ticket": t["id"]})
        ep.setdefault("trusted", []).append({"at_ms": now_ms(), "key": key})
        self.save(ep)
        say(f"{clock()} trust accepted on {key} (a person's press: score it as a gap)")

    # ---- watch

    def watch(self):
        ep = self.load()
        if not ep:
            die("no episode here: `lay` first")
        with self.wire() as w:
            snap = w.snapshot()
        board = snap["board"]
        lines, ep["feed_off"] = read_feed(self.paths, ep.get("feed_off", 0))
        journal, ep["log_off"] = read_from(self.paths.daemon_log, ep.get("log_off", 0))
        minutes = (now_ms() - ep["laid_ms"]) / 60000
        say(f"── {clock()} ∙ {minutes:.0f} min into the episode ∙ build {ep.get('build')}")
        # A machine tier the board did not override is still offered to its
        # crown; it is marked, since the demo means to run on its own.
        tiers = {t["id"]: f"{t['name']}(m)"
                 for t in (snap.get("machine_tiers") or {}).get("tiers", [])}
        tiers.update({t["id"]: t["name"] for t in board.get("tiers", [])})
        by_ticket = {}
        for s in board["sessions"]:
            by_ticket.setdefault(s["ticket"], []).append(s)
        for col in board["columns"]:
            tickets = [t for t in board["tickets"] if t["column"] == col["name"] and not t.get("archived")]
            if not tickets:
                continue
            say(f"  {col['name']}")
            for t in tickets:
                recs = [s for s in by_ticket.get(t["id"], []) if s["kind"] == "claude"]
                rec = recs[-1] if recs else None
                word = rig.word_of(rec["state"]) if rec else "no agent"
                if rec and rec["state"].get("reason"):
                    word += f" ({rec['state']['reason']})"
                tier = tiers.get((rec or {}).get("tier") or t.get("tier"), "")
                tags = ",".join(g["name"] for g in t.get("tags", []))
                say(f"    {t['short_key']:>5} {word:<14} {tier:<7} {tags:<12} {t['title'][:60]}")
        states = {}
        for s in board["sessions"]:
            states[rig.word_of(s["state"])] = states.get(rig.word_of(s["state"]), 0) + 1
        say("  sessions: " + ", ".join(f"{n} {w}" for w, n in sorted(states.items())))
        train = snap.get("automation") or {}
        say(f"  train {'armed' if train.get('merge_train') else 'OFF'}"
            + (f", suspended {len(train['train_suspended'])}" if train.get("train_suspended") else "")
            + (f", holding {len(train['holding'])}" if train.get("holding") else ""))
        self.crown_turns(board, ep)
        keys = {t["id"]: t["short_key"] for t in board["tickets"]}
        events = feed_events(lines, keys)
        if events:
            say("  feed:")
            for e in events[-25:]:
                say(f"    {e}")
            if len(events) > 25:
                say(f"    ({len(events) - 25} earlier lines)")
        slow = [l for l in journal.splitlines() if "slow turn" in l or "stopping" in l
                or "started" in l or "panic" in l]
        for l in slow[-10:]:
            say(f"  journal: {l[:160]}")
        usage = (snap.get("usage") or {})
        quota = usage_line(usage)
        if quota:
            say(f"  quota: {quota}")
        self.save(ep)

    def crown_turns(self, board, ep):
        rec = agent_of(board, ep["crown"])
        if not rec:
            say("  crown: no session")
            return
        ep["crown_session"] = rec["id"]
        rows = rig.transcript(rec)
        ep["rows_seen"] = len(rows)
        since = ep.get("turns_ms", 0)
        ep["turns_ms"] = now_ms()
        state = rig.word_of(rec["state"])
        say(f"  crown {rec_key(board, ep['crown'])}: {state}, {len(rows)} transcript rows")
        for turn in turns_of(rows):
            if (turn["end"] or now_ms()) < since:
                continue
            uses = unique_calls(turn["rows"])
            calls = [n.rsplit("__", 1)[-1] for n, _ in uses]
            results = [r for _, r in uses if r]
            sizes = sum(len(r["text"].encode()) for r in results)
            gets = [len(r["text"].encode()) for (n, r) in uses
                    if r and (n or "").endswith("get_ticket")]
            ctx = turn["context"]
            say(f"    turn {clock(turn['start'])} {turn['seconds']:.0f} s"
                f"{' (open)' if turn['open'] else ''}{' (cont.)' if turn['start'] < since else ''}"
                f" ∙ {turn['prompt'][:70]!r}")
            say(f"      {len(calls)} calls, results {sizes} B"
                + (f", get_ticket {'/'.join(map(str, gets))} B" if gets else "")
                + (f", context {ctx // 1000}k" if ctx else "")
                + (f", {sum(1 for r in results if r['is_error'])} refused"
                   if any(r["is_error"] for r in results) else ""))
            if calls:
                say(f"      {' '.join(squash(calls))}")
            if turn["reply"]:
                say(f"      said: {rig.flat(turn['reply'])[:220]}")

    # ---- score

    def score(self, what, marks, note):
        ep = self.load()
        if not ep:
            die("no episode here: `lay` first")
        path = os.path.join(self.dir, "scorecard.md")
        if not os.path.exists(path):
            with open(path, "w") as f:
                f.write(f"# Episode, build {ep.get('build')}, laid "
                        f"{datetime.datetime.fromtimestamp(ep['laid_ms'] / 1000):%Y-%m-%d %H:%M}\n\n"
                        "| at | what | move | speed | words | quality | harness | note |\n"
                        "|---|---|---|---|---|---|---|---|\n")
        with open(path, "a") as f:
            f.write(f"| {clock()} | {what} | {' | '.join(marks)} | {note} |\n")
        total, of = 0, 0
        with open(path) as f:
            for line in f:
                cells = [c.strip() for c in line.strip().strip("|").split("|")]
                if len(cells) == 8 and cells[0] not in ("at", "---"):
                    for c in cells[2:7]:
                        if c.isdigit():
                            total += int(c)
                            of += 2
        say(f"{clock()} scored {what}: {' '.join(marks)} ∙ running total {total}/{of} ∙ {path}")

    # ---- stop and reset

    def stop(self):
        if self.paths and self.alive():
            with self.wire() as w:
                board = w.board()
                live = [s for s in board["sessions"] if s["kind"] == "claude"
                        and rig.word_of(s["state"]) not in ("sleeping", "exited", "failed")]
                for s in live:
                    try:
                        w.sleep(s["id"])
                    except WireError as e:
                        say(f"  {s['id'][:8]} would not park ({e}): ending it")
                        try:
                            w.request({"cmd": "kill_session", "id": s["id"]})
                        except WireError:
                            pass
                deadline = time.time() + 60
                while live and time.time() < deadline:
                    live = [s for s in w.board()["sessions"] if s["kind"] == "claude"
                            and rig.word_of(s["state"]) not in ("sleeping", "exited", "failed")]
                    time.sleep(0.5)
                say(f"  sessions parked{'' if not live else f', {len(live)} still awake'}")
                pid = w.hello["daemon_pid"]
                w.shutdown()
            stopped(pid, "daemon")
        else:
            say("  no demo daemon runs")
        self.stop_holder()
        if self.paths and os.path.exists(self.paths.tmux_sock):
            out = subprocess.run(["tmux", "-S", self.paths.tmux_sock, "kill-server"],
                                 capture_output=True, text=True)
            say("  the demo's tmux server stopped" if out.returncode == 0
                else f"  tmux kill-server: {out.stderr.strip() or 'no server'}")

    def stop_holder(self):
        try:
            with open(os.path.join(self.dir, "hold.pid")) as f:
                pid = int(f.read())
        except (OSError, ValueError, TypeError):
            return
        try:
            os.kill(pid, signal.SIGTERM)
            say(f"  train holder {pid} stopped")
        except ProcessLookupError:
            pass
        os.remove(os.path.join(self.dir, "hold.pid"))

    def reset(self):
        self.stop()
        if not self.paths:
            return
        # Episodes keep their scorecards: they are the soak's record.
        keep = os.path.join(os.path.dirname(self.paths.state_dir), "demo-scorecards")
        card = os.path.join(self.dir, "scorecard.md")
        if os.path.exists(card):
            os.makedirs(keep, exist_ok=True)
            dst = os.path.join(keep, f"{time.strftime('%Y%m%d-%H%M%S')}.md")
            shutil.copy(card, dst)
            say(f"  scorecard kept at {dst}")
        for d in (self.repo, self.paths.state_dir, self.paths.rt_dir):
            shutil.rmtree(d, ignore_errors=True)
        say(f"  wiped {self.repo} and its board")


# ---- reading helpers

def agent_of(board, tid):
    recs = [s for s in board["sessions"] if s["ticket"] == tid and s["kind"] == "claude"]
    return recs[-1] if recs else None


def rec_key(board, tid):
    return next((t["short_key"] for t in board["tickets"] if t["id"] == tid), "?")


def read_from(path, off):
    try:
        size = os.path.getsize(path)
    except OSError:
        return "", 0
    if size < off:
        off = 0
    with open(path, "rb") as f:
        f.seek(off)
        data = f.read()
    return data.decode(errors="replace"), off + len(data)


def stopped(pid, what):
    deadline = time.time() + 30
    while time.time() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            say(f"  {what} {pid} stopped")
            return True
        time.sleep(0.2)
    say(f"  {what} {pid} still runs 30 s after shutdown")
    return False


def unique_calls(rows):
    """(name, result) per tool use, each id once: Claude Code writes an MCP
    tool_use row twice under one id."""
    seen, out = set(), []
    calls = rig.tool_calls(rows)
    ids = [b.get("id") for r in rows for b in ((r.get("message") or {}).get("content") or [])
           if isinstance(b, dict) and b.get("type") == "tool_use"]
    for i, call in zip(ids, calls):
        if i not in seen:
            seen.add(i)
            out.append(call)
    return out


def squash(names):
    """`list_board get_ticket get_ticket` as `list_board get_ticket×2`."""
    out = []
    for n in names:
        if out and out[-1][0] == n:
            out[-1][1] += 1
        else:
            out.append([n, 1])
    return [n if c == 1 else f"{n}×{c}" for n, c in out]


def turns_of(rows):
    """The crown's turns in transcript rows: each starts at a prompt that is
    not a tool result, and carries its length, its last reply and the last
    context it read (the API's input tokens, cache included)."""
    turns = []
    for r in rows:
        msg = r.get("message") or {}
        content = msg.get("content")
        is_prompt = r.get("type") == "user" and not r.get("isMeta") and (
            isinstance(content, str) or (isinstance(content, list) and not any(
                isinstance(b, dict) and b.get("type") == "tool_result" for b in content)))
        if is_prompt:
            text = "\n".join(rig.texts(r)).strip()
            turns.append({"start": rig.row_ms(r) or now_ms(), "prompt": rig.flat(text),
                          "rows": [], "end": None, "context": 0})
        if not turns:
            continue
        t = turns[-1]
        t["rows"].append(r)
        if r.get("type") == "assistant":
            t["end"] = rig.row_ms(r) or t["end"]
            u = msg.get("usage") or {}
            ctx = u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0) + \
                u.get("cache_creation_input_tokens", 0)
            t["context"] = ctx or t["context"]
    for t in turns:
        t["reply"] = rig.last_reply(t["rows"])
        t["open"] = t["end"] is None
        t["seconds"] = ((t["end"] or now_ms()) - t["start"]) / 1000
    return turns


def feed_events(lines, keys):
    """The feed lines a person scoring the board cares about, in words."""
    out = []
    for l in lines:
        kind, at = l.get("kind"), clock(l.get("at_ms"))
        key = keys.get(l.get("ticket"), "")
        cmd = str(l.get("cmd", ""))
        outcome = f" → {l['outcome']}" if l.get("outcome") else ""
        if kind == "board" and l.get("actor") == "agent":
            out.append(f"{at} agent {cmd} {key}{outcome}"[:200])
        elif kind == "board" and cmd in LOST_WORDS:
            out.append(f"{at} LOST? {cmd} {key}{outcome}"[:200])
        elif kind == "board" and ("merge" in cmd or "train" in cmd or "rebase" in cmd):
            out.append(f"{at} {cmd} {key}{outcome}"[:200])
        elif kind == "crown_wake":
            out.append(f"{at} wakes the crown: {l.get('cause')}"[:200])
        elif kind == "session_state" and l.get("to") in ("requires_action", "failed", "exited"):
            out.append(f"{at} {key} {l.get('to')}{' (' + l['reason'] + ')' if l.get('reason') else ''}")
        elif kind == "board" and cmd in ("raise_hand", "agent_raise_hand"):
            out.append(f"{at} hand raised {key}{outcome}")
    return out


def usage_line(usage):
    windows = (((usage or {}).get("claude") or {}).get("reading") or {}).get("windows") or []
    return ", ".join(f"{w.get('label')} {w.get('percent'):.0f}%" for w in windows
                     if isinstance(w, dict) and isinstance(w.get("percent"), (int, float)))


def default_bin():
    """The binary this board's daemon runs: the checkout's journal names the
    daemon's pid, and `ps` its argv."""
    common = rig.git(HERE, "rev-parse", "--path-format=absolute", "--git-common-dir")
    checkout = os.path.dirname(common) if common else None
    if not checkout:
        return None
    log = board_paths(checkout).daemon_log
    try:
        with open(log) as f:
            starts = [l for l in f if " started pid " in l]
    except OSError:
        starts = []
    if starts:
        pid = starts[-1].split(" started pid ", 1)[1].split()[0]
        out = subprocess.run(["ps", "-o", "command=", "-p", pid], capture_output=True, text=True)
        argv = out.stdout.split()
        if argv and argv[1:2] == ["daemon"] and os.path.exists(argv[0]):
            return argv[0]
    fallback = os.path.join(checkout, "target/debug/mesimon")
    return fallback if os.path.exists(fallback) else None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", default=DEFAULT_REPO)
    sub = ap.add_subparsers(dest="cmd", required=True)
    lay = sub.add_parser("lay")
    lay.add_argument("--bin", help="the mesimon binary the demo daemon runs")
    s = sub.add_parser("say")
    s.add_argument("words")
    sub.add_parser("watch")
    wt = sub.add_parser("wait")
    wt.add_argument("--for", dest="seconds", type=int, default=600)
    tr = sub.add_parser("trust")
    tr.add_argument("key")
    sc = sub.add_parser("score")
    sc.add_argument("what", help="a crown move or a delivered ticket, in a few words")
    for mark in ("move", "speed", "words", "quality", "harness"):
        sc.add_argument(mark, choices=("0", "1", "2", "-"))
    sc.add_argument("note")
    sub.add_parser("stop")
    sub.add_parser("reset")
    sub.add_parser("hold")
    # `--repo` is accepted after the subcommand too.
    argv = sys.argv[1:]
    if "--repo" in argv[1:]:
        i = argv.index("--repo", 1)
        argv = ["--repo", argv[i + 1]] + argv[:i] + argv[i + 2:]
    a = ap.parse_args(argv)
    demo = Demo(a.repo)
    if a.cmd == "lay":
        binary = a.bin or default_bin()
        if not binary or not os.path.exists(binary):
            die("no mesimon binary: name one with --bin")
        demo.lay(os.path.realpath(binary))
        return
    if not demo.paths:
        die(f"no repo at {a.repo}: `lay` first")
    if a.cmd == "say":
        if "\n" in a.words.strip():
            die("a person's words here are one line")
        demo.say_words(a.words.strip())
    elif a.cmd == "wait":
        demo.wait(a.seconds)
    elif a.cmd == "trust":
        demo.trust(a.key)
    elif a.cmd == "watch":
        demo.watch()
    elif a.cmd == "score":
        demo.score(a.what, [a.move, a.speed, a.words, a.quality, a.harness], a.note)
    elif a.cmd == "stop":
        demo.stop()
    elif a.cmd == "reset":
        demo.reset()
    elif a.cmd == "hold":
        demo.hold()


if __name__ == "__main__":
    main()
