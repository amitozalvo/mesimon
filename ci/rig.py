#!/usr/bin/env python3
"""The rig (T-588): a mesimon board inside this ticket worktree drives the
real Claude Code on the mod road, one test at a time, through its own crown.

    python3 -B ci/rig.py            build, start or find the rig's daemon, lay
                                    the board, file the tests, run them all
    python3 -B ci/rig.py --lay      the same up to the crown filed and crowned;
                                    nothing starts, nothing costs
    python3 -B ci/rig.py --only R3  run the named tests (a comma list); a
                                    letter alone names its group (R, P, D, T, C)
    python3 -B ci/rig.py --failed   run again only what the last run's
                                    verdicts.md lists as FAIL
    python3 -B ci/rig.py --reset    park and archive the rig's tickets, stop its
                                    daemon and its private tmux
    --no-build                      skip `cargo build -p mesimon`
    --flags-off                     the rig's daemon launches every Claude Code
                                    with its flags service off (the seam
                                    MESIMON_RIG_NO_FLAGS=1, which sets
                                    DISABLE_GROWTHBOOK=1), so each flag reads
                                    its built-in default: the run's acceptance
                                    while Claude Code's remote flag has mods
                                    off (T-598). Every other flag is pinned to
                                    its default with it. The rig's alone.

Watch it with the command it prints: `cd <worktree> && target/debug/mesimon`.

The rig is a second board on this worktree's own proj16 (its own state dir,
sockets and private tmux); it never touches the author's main board, and it
writes nothing under the machine layer (`tiers.toml`, `prefs.json`): its
tier is a board tier, in the worktree's `.mesimon/`. Its sessions' mods do
refresh the machine's `usage.json` with the account's own windows (T-581),
as any board's do.
It drives the daemon over the wire (`ci/rig/wire.py`) and reads the feed and
the transcripts, never a screen. Its crown, a real Claude Code session on
Sonnet, is the subject under test: each test's words go to the crown, and
the crown starts, answers, asks, parks and wakes the workers through its
MCP tools. Tests and their exact words are `ci/rig/tests.toml`.

Each test's verdict is a note on its ticket. The crown moves a passed test's
ticket to DONE when the rig tells it to, at the head of the next words it
sends (the next test's first crown step, or the run's last words), so a
close-out costs no turn of its own; a failed one stays in REVIEW. The
run's table is printed and written to `target/rig/verdicts.md`. Nothing is
torn down at the end, and nothing on the rig's board is archived but by
`--reset` (the author's rule: the crown never archives there).
"""

import argparse
import glob
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "rig"))
from wire import Subscription, Wire, WireError, board_paths, feed_all, read_feed, ulid  # noqa: E402

TIER_NAME = "rig"
TIER_MODEL = "sonnet"  # Claude Code's alias for Sonnet 5.5 on 2.1.287
TIER_MODEL_FULL = "claude-sonnet-5-5"
TIER_EFFORT = "low"
TIER_DESCRIPTION = "every rig test and the rig's crown"
STEP_TIMEOUT = 600
WAKE_GRACE = 60
QUIET = 6.0
SWEEP_WAIT = 1.0  # the last frames' feed lines, after the worker's end
# The column the permission tests' workers start in (T-581): Claude Code in
# manual mode, so a Bash command asks.
MANUAL_COLUMN = "RIG MANUAL"
# Where the rig stands in for the phone (T-581): the socket the phone mod's
# `mesimon approve` dials, short for sun_path. The phone holds each request
# this long, so the dialog is up and the card says so before it is answered.
PHONE_SOCK = "/tmp/msmn-rig-phone.sock"
PHONE_HOLD = 4.0
# What a terminal hands a program it starts: the rig's daemon is started from
# this, never from the environment of the agent running the rig, which
# carries another board's MESIMON_*, a tmux pane's TMUX and Claude Code's own.
TERMINAL_ENV = (
    "HOME", "USER", "LOGNAME", "SHELL", "PATH", "LANG", "LC_ALL", "LC_CTYPE",
    "TERM", "COLORTERM", "TERM_PROGRAM", "TMPDIR", "SSH_AUTH_SOCK",
    "__CF_USER_TEXT_ENCODING",
)

LOG = None
# The turn roads' feed words (T-575, T-576), printed as they land.
TURN_ROAD_WORDS = (
    "prompt_by_mod", "prompt_by_paste", "prompt_submit_refused", "prompt_submit_unreceived",
    "prompt_resent", "prompt_submit_not_ready", "answer_by_mod",
)


def say(text=""):
    print(text, flush=True)
    if LOG:
        LOG.write(text + "\n")
        LOG.flush()


def die(text):
    say(f"rig: {text}")
    sys.exit(1)


def git(repo, *args):
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    out = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, env=env)
    return out.stdout.strip() if out.returncode == 0 else None


def terminal_env(extra=None):
    env = {k: os.environ[k] for k in TERMINAL_ENV if k in os.environ}
    env.update(extra or {})
    return env


# ---------------------------------------------------------------- the board


def word_of(state):
    """A session state as the card says it."""
    if not state:
        return "unknown"
    tag = state.get("state")
    if tag in ("spawning", "running", "throttled"):
        return "working"
    if tag == "idle":
        return "working" if state.get("stop_reason") == "background" else "idle"
    if tag == "requires_action":
        return "needs_you"
    return tag  # sleeping, exited, failed, unknown


def feed_word(line):
    """A `session_state` feed line's `to`, as the card says it."""
    to, reason = line.get("to"), line.get("reason")
    if to in ("spawning", "running", "throttled"):
        return "working"
    if to == "idle":
        return "working" if reason == "background" else "idle"
    if to == "requires_action":
        return "needs_you"
    return to


def is_subsequence(want, seen):
    it = iter(seen)
    return all(any(w == s for s in it) for w in want)


def squeeze(words):
    out = []
    for w in words:
        if w != "unknown" and (not out or out[-1] != w):
            out.append(w)
    return out


def transcript(rec):
    sid = rec.get("claude_session_id") or rec["id"]
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{sid}.jsonl"))
    if not hits:
        return []
    rows = []
    with open(max(hits, key=os.path.getmtime), encoding="utf-8") as f:
        for raw in f:
            try:
                rows.append(json.loads(raw))
            except ValueError:
                pass
    return rows


def texts(row):
    content = (row.get("message") or {}).get("content")
    if isinstance(content, str):
        return [content]
    if isinstance(content, list):
        return [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
    return []


def first_prompt(rows):
    for row in rows:
        if row.get("type") == "user" and not row.get("isMeta"):
            t = "\n".join(texts(row)).strip()
            if t:
                return t
    return ""


def last_reply(rows):
    for row in reversed(rows):
        if row.get("type") == "assistant":
            t = "\n".join(texts(row)).strip()
            if t:
                return t
    return ""


def model_of(rows):
    for row in reversed(rows):
        if row.get("type") == "assistant":
            m = (row.get("message") or {}).get("model")
            if m and m != "<synthetic>":
                return m
    return None


def flat(text):
    return " ".join(text.split())


def filler(size=10_000):
    """P1's filler: numbered paragraphs of plain prose, to `size` bytes."""
    out, n = [], 0
    while sum(len(p) + 2 for p in out) < size:
        n += 1
        out.append(f"{n}. A paragraph of filler for the long-brief test, with \"quotes\", "
                   f"an apostrophe's turn, a `backtick`, $HOME written as text, and the "
                   f"number {n} so no two lines read alike.")
    return "\n\n".join(out)


def tool_calls(rows):
    """(tool name, its result) for every tool use in a transcript, in order;
    a result is `{is_error, text}`, or None while none came."""
    results = {}
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_result":
                body = b.get("content")
                text = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
                results[b.get("tool_use_id")] = {"is_error": bool(b.get("is_error")), "text": text}
    out = []
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_use":
                out.append((b.get("name"), results.get(b.get("id"))))
    return out


def user_prompts(rows, at=False):
    """The person's prompts in a transcript, in order: user rows with text,
    not a background task's notification (the engine's, not the person's).
    With `at`, each with its timestamp."""
    out = []
    for row in rows:
        if row.get("type") == "user" and not row.get("isMeta"):
            t = "\n".join(texts(row)).strip()
            if t and not t.startswith("<task-notification>"):
                out.append((t, row.get("timestamp")) if at else t)
    return out


def iso_ms(stamp):
    """A transcript's ISO timestamp as epoch ms."""
    from datetime import datetime
    try:
        return int(datetime.fromisoformat(stamp.replace("Z", "+00:00")).timestamp() * 1000)
    except (AttributeError, ValueError):
        return None


def replies(rows):
    """The assistant's text replies, in order."""
    out = []
    for row in rows:
        if row.get("type") == "assistant":
            t = "\n".join(texts(row)).strip()
            if t:
                out.append(t)
    return out


def sid16(session):
    """The tmux session name: a pane-died frame names its session by it."""
    return session.replace("-", "")[:16]


def of_session(line, session):
    return line.get("session") in (session, sid16(session))


def last_failed(path):
    """The test ids a verdicts table lists as FAIL, or None without one."""
    try:
        with open(path) as f:
            rows = f.read().splitlines()
    except OSError:
        return None
    out = []
    for row in rows:
        cells = [c.strip() for c in row.split("|")]
        if len(cells) > 3 and cells[3].startswith("FAIL"):
            out.append(cells[1].split()[0])
    return out


class Phone:
    """The phone and the daemon's wait, stood in for at the socket the phone
    mod's `mesimon approve` dials (T-581): each request (a RemotePermission
    header line and the PermissionRequest payload) is held `PHONE_HOLD`
    seconds and answered as the daemon answers it, `"allow"` or `"deny"`
    (`mesophon::PermissionDecision`), then the stream closes. Every request is
    logged with the session and the answer."""

    def __init__(self):
        self.answer = "allow"
        self.log = []
        if os.path.exists(PHONE_SOCK):
            os.remove(PHONE_SOCK)
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(PHONE_SOCK)
        os.chmod(PHONE_SOCK, 0o600)
        self.sock.listen(8)
        threading.Thread(target=self.serve, daemon=True).start()

    def serve(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            threading.Thread(target=self.one, args=(conn,), daemon=True).start()

    def one(self, conn):
        with conn:
            buf = b""
            conn.settimeout(5)
            try:
                while buf.count(b"\n") < 2:
                    chunk = conn.recv(1 << 16)
                    if not chunk:
                        break
                    buf += chunk
                header, payload = (json.loads(x) for x in buf.split(b"\n")[:2])
            except (OSError, ValueError):
                return
            answer = self.answer
            time.sleep(PHONE_HOLD)
            try:
                conn.sendall(json.dumps(answer).encode())
            except OSError:
                answer = f"{answer} (unsent)"
            self.log.append({"session": header.get("session"), "tool": payload.get("tool_name"),
                             "answer": answer, "at_ms": int(time.time() * 1000)})


class Rig:
    def __init__(self, repo, args):
        self.repo = repo
        self.args = args
        self.paths = board_paths(repo)
        self.bin = os.path.join(repo, "target/debug/mesimon")
        self.out = os.path.join(repo, "target/rig")
        os.makedirs(self.out, exist_ok=True)
        self.wire = None
        self.sub = None
        self.claude = shutil.which("claude")
        self.claude_version = None
        self.build = None
        self.branch = None
        # The rig's seams on every start of its daemon (`--flags-off`).
        self.seams = {"MESIMON_RIG_NO_FLAGS": "1"} if getattr(args, "flags_off", False) else {}
        self.phone = None

    # ---- the daemon

    def connect(self):
        if self.wire:
            self.wire.close()
        if self.sub:
            self.sub.close()
        self.wire = Wire(self.paths.orch_sock, client="rig", connect_for=30)
        self.sub = Subscription(self.paths.orch_sock)
        return self.wire.hello

    def alive(self):
        try:
            Wire(self.paths.orch_sock, client="rig-probe", connect_for=0.2).close()
            return True
        except OSError:
            return False

    def start_daemon(self, extra_env=None):
        os.makedirs(self.paths.state_dir, mode=0o700, exist_ok=True)
        log = open(self.paths.daemon_log, "ab")
        env = terminal_env({"MESIMON_DETACHED": "1", **self.seams, **(extra_env or {})})
        subprocess.Popen(
            [self.bin, "daemon", "--repo", self.repo],
            stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=env,
            start_new_session=True, cwd=self.repo,
        )
        log.close()
        hello = self.connect()
        say(f"  daemon up: pid {hello['daemon_pid']}, build {hello.get('build')}")
        return hello

    def stop_daemon(self):
        pid = Wire(self.paths.orch_sock, client="rig-stop").hello["daemon_pid"]
        os.kill(pid, signal.SIGTERM)
        deadline = time.time() + 30
        while time.time() < deadline:
            # A daemon the rig started is its child: reaped here, or it
            # stays a zombie that `kill(pid, 0)` still finds.
            try:
                os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                pass
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.1)
        else:
            die(f"the daemon (pid {pid}) did not exit within 30 s of SIGTERM")
        say(f"  daemon {pid} stopped (SIGTERM)")
        return pid

    def ensure_daemon(self):
        if self.alive():
            hello = self.connect()
            stamp = hello.get("exe_stamp") or {}
            st = os.stat(self.bin)
            same = stamp.get("len") == st.st_size and abs(
                stamp.get("mtime_ms", 0) - st.st_mtime_ns // 1_000_000
            ) <= 1
            env = self.env_of(hello["daemon_pid"])
            seamed = all(f"{k}={v}" in env for k, v in self.seams.items())
            if same and seamed:
                say(f"  found the rig's daemon: pid {hello['daemon_pid']}, this build")
                return
            why = "another build" if not same else "without --flags-off's seam"
            say(f"  the rig's daemon (pid {hello['daemon_pid']}) runs {why}: restarting it")
            self.stop_daemon()
        self.start_daemon()

    def restart(self, extra_env=None, want_probe=None):
        """SIGTERM and start again. A board TUI watching the rig respawns a
        daemon the moment its socket goes, without the rig's seam, and often
        wins the lock: the winner's environment (`ps -wwE`) says whose it
        is, and one that lacks the seam is stopped and the race run again.
        Then `want_probe` names the words `road.json` must come to say."""
        for attempt in range(1, 11):
            self.stop_daemon()
            hello = self.start_daemon(extra_env)
            want = {**self.seams, **(extra_env or {})}
            if all(f"{k}={v}" in self.env_of(hello["daemon_pid"]) for k, v in want.items()):
                break
            say(f"  pid {hello['daemon_pid']} lacks the rig's seam: another client respawned "
                f"it first; again ({attempt}/10)")
        else:
            return False
        if not want_probe:
            return True
        line = self.wait_probe(want_probe)
        say(f"  road.json: {line}")
        return bool(line and want_probe in line)

    def tmux(self, *args):
        """The rig's private tmux server, by the build its daemon runs."""
        tmux = os.environ.get("MESIMON_TMUX_BIN") or (
            os.path.join(self.repo, "target/debug/mesimon-tmux")
            if os.path.exists(os.path.join(self.repo, "target/debug/mesimon-tmux")) else "tmux")
        return subprocess.run([tmux, "-S", self.paths.tmux_sock, *args],
                              capture_output=True, text=True)

    def drop_mod(self):
        """P4's mod: the laid sources, with the first submit of each session
        reported dropped. Under the rig's state dir: every load writes
        Claude Code's types and tsconfig into the folder, never a checkout."""
        src = os.path.join(self.repo, "crates/mesimon-daemon/mod")
        dst = os.path.join(self.paths.state_dir, "rig", "mod-drop")
        if os.path.exists(dst):
            shutil.rmtree(dst)
        shutil.copytree(src, dst)
        path = os.path.join(dst, "hooks", "register.ts")
        with open(path) as f:
            text = f.read()
        head = "async function submitPrompt($: any, id: string, text: string) {\n"
        if head not in text:
            die("the mod's submitPrompt moved: P4's drop cannot be patched in")
        text = text.replace(head, head + (
            "  if (!rigDropped) {\n"
            "    rigDropped = true\n"
            "    await relay($, 'ModSubmit', id, { outcome: 'dropped', reason: 'the rig dropped it (P4)' }, false)\n"
            "    return\n"
            "  }\n"), 1)
        anchor = "const seen: string[] = []\n"
        if anchor not in text:
            die("the mod's `seen` list moved: P4's flag cannot be declared")
        text = text.replace(anchor, anchor + "let rigDropped = false\n", 1)
        os.remove(os.path.join(dst, "hooks", "register.test.ts"))
        with open(path, "w") as f:
            f.write(text)
        out = subprocess.run([self.claude, "plugin", "validate", dst], capture_output=True, text=True)
        if out.returncode != 0:
            die(f"P4's mod does not validate:\n{out.stdout}{out.stderr}")
        return dst

    def phone_mod(self):
        """C1's and C2's mod: the laid sources, with `mesimon approve` dialing
        the rig's phone instead of the board's hook.sock. Nothing else moves:
        the hold, the relay of PermissionRequest and the decision's return
        are the mod's own."""
        src = os.path.join(self.repo, "crates/mesimon-daemon/mod")
        dst = os.path.join(self.paths.state_dir, "rig", "mod-phone")
        if os.path.exists(dst):
            shutil.rmtree(dst)
        shutil.copytree(src, dst)
        path = os.path.join(dst, "hooks", "register.ts")
        with open(path) as f:
            text = f.read()
        line = "const argv = [c.bin, 'approve', '--sock', c.hookSock, '--session', c.session]"
        if line not in text:
            die("the mod's approve moved: the phone cannot be patched in")
        text = text.replace(line, line.replace("c.hookSock", repr(PHONE_SOCK)), 1)
        os.remove(os.path.join(dst, "hooks", "register.test.ts"))
        with open(path, "w") as f:
            f.write(text)
        out = subprocess.run([self.claude, "plugin", "validate", dst], capture_output=True, text=True)
        if out.returncode != 0:
            die(f"the phone mod does not validate:\n{out.stdout}{out.stderr}")
        return dst

    def env_of(self, pid):
        out = subprocess.run(["ps", "-wwE", "-o", "command=", "-p", str(pid)],
                             capture_output=True, text=True)
        return out.stdout.split()

    def mod_digest(self):
        """The laid mod's digest as `modroad::digest` computes it, from the
        sources `modroad::FILES` names, in its order."""
        import hashlib
        src = os.path.join(self.repo, "crates/mesimon-daemon")
        with open(os.path.join(src, "src/modroad.rs")) as f:
            files = re.findall(r'\("([^"]+)", include_str!\("\.\./mod/([^"]+)"\)\)', f.read())
        h = hashlib.sha256()
        for rel, path in files:
            with open(os.path.join(src, "mod", path), "rb") as f:
                text = f.read()
            h.update(rel.encode() + b"\0" + text + b"\0")
        return h.hexdigest()

    def wait_this_mod(self, timeout=120):
        """The daemon's probe of this build's mod: a new build's mod has a
        new digest and is probed again, and a launch before that probe
        lands takes the hook set."""
        want = self.mod_digest()
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                with open(os.path.join(self.paths.mod_root, "probe.json")) as f:
                    p = json.load(f)
                if p.get("key", {}).get("digest") == want and p.get("verdict") == "passed":
                    return True
            except (OSError, ValueError):
                pass
            time.sleep(0.5)
        return False

    def wait_probe(self, words, timeout=120):
        deadline = time.time() + timeout
        line = None
        while time.time() < deadline:
            try:
                with open(os.path.join(self.paths.mod_root, "road.json")) as f:
                    v = json.load(f)
                line = f"{v.get('road')} ∙ {v.get('probe')}"
                if words in line:
                    return line
            except (OSError, ValueError):
                pass
            time.sleep(0.5)
        return line

    # ---- reads

    def snapshot(self):
        return self.wire.snapshot()

    def board(self):
        return self.wire.board()

    def ticket(self, board, tid):
        return next((t for t in board["tickets"] if t["id"] == tid), None)

    def agent_of(self, board, tid):
        recs = [s for s in board["sessions"] if s["ticket"] == tid and s["kind"] == "claude"]
        return recs[-1] if recs else None

    # ---- laying the board

    def lay(self):
        board = self.board()
        names = [c["name"] for c in board["columns"]]
        for want in ("TODO", "IN PROGRESS", "REVIEW", "DONE"):
            if want not in names:
                die(f"the rig's board has no {want} column (it has {names}); --reset and run again")
        tier = next((t for t in board.get("tiers", []) if t["name"] == TIER_NAME), None)
        spec = {
            "id": tier["id"] if tier else ulid(),
            "name": TIER_NAME,
            "provider": "claude_code",
            "model": tier.get("model", TIER_MODEL) if tier else TIER_MODEL,
            "effort": TIER_EFFORT,
            "description": TIER_DESCRIPTION,
        }
        if tier != spec:
            self.wire.save_tier("board", spec)
        if board.get("default_tier") != spec["id"]:
            self.wire.set_default_tier("board", spec["id"])
        self.tier = spec
        # The permission tests' column (T-581): Claude Code in manual mode.
        col = next((c for c in board["columns"] if c["name"] == MANUAL_COLUMN), None)
        if col is None:
            self.wire.request({"cmd": "add_column", "name": MANUAL_COLUMN, "after": "TODO"})
            col = next(c for c in self.board()["columns"] if c["name"] == MANUAL_COLUMN)
        if col.get("claude_mode") != "manual":
            settings = {k: v for k, v in col.items() if k not in ("name", "order")}
            settings["claude_mode"] = "manual"
            self.wire.request({"cmd": "set_column_settings", "name": MANUAL_COLUMN,
                               "settings": settings})
        if not board.get("crown_answers", True):
            self.wire.set_crown_answers(True)
        # The crown's asks go straight to the workers it started (T-550);
        # otherwise each waits on its card for a person's ^y.
        if not board.get("crown_sends"):
            self.wire.set_crown_sends(True)
        say(f"  tier {TIER_NAME} ({spec['model']}, effort {TIER_EFFORT}) is the board's default; "
            f"the crown answers and sends")

    def clear_previous(self):
        """A run is repeatable. An earlier run's tickets stay where they are
        (only `--reset` archives on this board); their agents are parked,
        and an agent parked on the checkout is ended (its conversation kept
        on disk), since a parked agent there holds the checkout and the next
        R2 would be refused it (T-583)."""
        board = self.board()
        self.park_all(board)
        held = [s for s in self.board()["sessions"] if s["kind"] == "claude"
                and word_of(s["state"]) == "sleeping" and os.path.realpath(s["cwd"]) == self.repo]
        for s in held:
            try:
                self.wire.request({"cmd": "kill_session", "id": s["id"]})
            except WireError as e:
                say(f"  could not end {s['id'][:8]}: {e}")
        if held:
            say(f"  ended {len(held)} earlier agents parked on the checkout")

    def park_all(self, board):
        live = [s for s in board["sessions"]
                if s["kind"] == "claude" and word_of(s["state"]) not in ("sleeping", "exited")]
        for s in live:
            try:
                self.wire.sleep(s["id"])
            except WireError as e:
                # Only an idle agent sleeps; one held on a dialog is ended
                # (its conversation stays on disk).
                say(f"  could not park {s['id'][:8]} ({e}): ending it")
                try:
                    self.wire.request({"cmd": "kill_session", "id": s["id"]})
                except WireError as e2:
                    say(f"  could not end {s['id'][:8]}: {e2}")
        deadline = time.time() + 90
        while live and time.time() < deadline:
            board = self.board()
            live = [s for s in board["sessions"] if s["kind"] == "claude"
                    and word_of(s["state"]) not in ("sleeping", "exited", "failed")]
            time.sleep(0.5)
        for s in board["sessions"]:
            if s["kind"] == "bash" and word_of(s["state"]) != "exited":
                try:
                    self.wire.request({"cmd": "kill_session", "id": s["id"]})
                except WireError:
                    pass

    def file(self, tests):
        # Every rig board files into the same repository, whose branches
        # every worktree shares, and each board counts its keys from T-1: two
        # boards' `T-9 · R1` would cut the same `msmn/T-9-r1-…` branch, and
        # the second's worktree is refused. The worktree's own ticket heads
        # each title (T-577 found it), so its branches are its own.
        owner = re.match(r"msmn/(T-\d+)", self.branch or "")
        for test in tests:
            if owner and not test["title"].startswith(owner.group(1)):
                test["title"] = f"{owner.group(1)} {test['title']}"
                test["siblings"] = [f"{owner.group(1)} {t}" for t in test.get("siblings", [])]
            if test.get("long_brief"):
                test["brief"] = test["brief"] + "\n" + filler()
            tid = self.file_one(test, test["title"])
            test["ticket"] = tid
            test["key"] = self.ticket(self.board(), tid)["short_key"]
            # Siblings (P5): tickets the test starts beside its own, each
            # with the same brief, every check run on each.
            test["others"] = []
            for title in test.get("siblings", []):
                sid = self.file_one(test, title)
                test["others"].append((sid, self.ticket(self.board(), sid)["short_key"], title))
            keys = " ".join([test["key"]] + [k for _, k, _ in test["others"]])
            say(f"  filed {keys}: {test['title']}")
        # The title is the person's own words and the brief is pasted, so the
        # title asks for it (see tests.toml on <pasted_content>).
        crown = self.wire.create_ticket("TODO", f"Rig crown · run {self.run_id}: follow the brief below")
        self.wire.write_note(crown, self.crown_brief(tests))
        self.wire.set_manual_merge(crown, True)
        self.wire.set_workspace(crown, "shared_checkout")
        self.wire.crown(crown)
        self.crown = crown
        self.crown_key = self.ticket(self.board(), crown)["short_key"]
        say(f"  filed and crowned {self.crown_key}: the rig's crown, on the checkout")

    def file_one(self, test, title):
        tid = self.wire.create_ticket(test.get("column", "TODO"), title)
        self.wire.write_note(tid, test["brief"])
        self.wire.write_note(tid, self.test_card(test))
        # Off the merge train: a rebase ask would be a prompt the test did
        # not send.
        self.wire.set_manual_merge(tid, True)
        return tid

    def test_tickets(self, test):
        """(ticket, key, title) for the test's own ticket and its siblings."""
        return [(test["ticket"], test["key"], test["title"])] + test.get("others", [])

    def keys_of(self, test):
        keys = {"key": test["key"]}
        for i, (_, k, _) in enumerate(test.get("others", []), start=2):
            keys[f"key{i}"] = k
        return keys

    def test_card(self, test):
        lines = [f"# {test['id']}: what the rig checks", "", f"**Expected.** {test['expect']}", ""]
        lines.append("**Steps.**")
        for step in test["steps"]:
            if "crown" in step:
                said = step["crown"].format(key="this ticket", key2="the second", key3="the third")
                lines.append(f"- the crown is told: “{said}”")
            else:
                lines.append(f"- the rig: `{step['rig']}`")
        lines += ["", "**Checks.** " + ", ".join(f"`{c}`" for c in test["checks"])]
        return "\n".join(lines)

    def crown_brief(self, tests):
        order = "\n".join(f"- {t['id']} ({t['key']}): "
                          f"{t['title'].split(' · ', 1)[-1].removesuffix(': do the steps below')}. "
                          f"{t['expect']}" for t in tests)
        return f"""You wear the crown of the rig: a mesimon board inside a ticket worktree, run by `ci/rig.py`, that tests mesimon's Claude Code mod road on the real Claude Code. You are the subject under test: each test proves a crown can drive a worker through the board while the mod relays every frame, with nothing read off a screen.

How a run goes. The rig sends you one step at a time, as a prompt that names one test ticket and the exact tool calls to make. Make exactly those calls, in that order, and nothing else: read no file, run no command, write no note, file no ticket and raise no hand; the rig reads the board itself and writes every verdict. Move a ticket only when the rig tells you to: a test that passed goes to DONE with move_ticket. Never archive a ticket: on this board only `ci/rig.py --reset` archives. Take a ticket's seen stamp from get_ticket with its key right before a call that needs one. When the board wakes you for a worker, do what the step said for that moment; if it said nothing, end your turn with one line naming the test and the worker's state word.

The tests, in order, and what each needs:
{order}

Reply with the single word ready and end your turn."""

    # ---- running

    def wait_for(self, what, pred, timeout, on_tick=None):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                got = pred()
                if got:
                    return got
                if on_tick:
                    on_tick()
                self.sub.wait(0.5)
            except (OSError, ConnectionError) as e:
                # A daemon that went away under a read: someone restarted it.
                say(f"  ({what}: {e}; reconnecting)")
                time.sleep(1)
                self.connect()
        say(f"  gave up waiting for {what} after {timeout} s")
        return None

    def start_crown(self):
        # A fresh board probes its Claude Code when the shell environment
        # lands (T-588); a crown started before the verdict launches on the
        # hook set, and every step would then reach it by paste.
        line = self.wait_probe("the mod validated", 120)
        say(f"\n  road.json: {line}")
        if not self.wait_this_mod():
            say("  no probe of this build's mod passed in 120 s: launches take the hook set")
        say(f"\n▶ starting the crown {self.crown_key} (its brief is its first prompt)")
        r = self.wire.spawn(self.crown, "claude", submit_prompt=True)
        say(f"  spawn: {r.get('resp')}")
        t0 = time.time()
        ok = self.wait_for("crown ready", lambda: self.crown_settled(after_ms=0), STEP_TIMEOUT,
                           self.watch)
        if not ok:
            die("the crown did not finish its first turn")
        rec = self.agent_of(self.board(), self.crown)
        rows = transcript(rec)
        model = model_of(rows)
        say(f"  crown ready in {time.time() - t0:.0f} s ∙ road {rec.get('road', 'hooks')} ∙ "
            f"model {model} ∙ said {last_reply(rows)[:40]!r}")
        if model and not model.startswith(TIER_MODEL_FULL) and self.tier["model"] == TIER_MODEL:
            say(f"  `{TIER_MODEL}` resolved to {model} on this build: the tier takes "
                f"{TIER_MODEL_FULL} and the crown starts again")
            self.wire.save_tier("board", {**self.tier, "model": TIER_MODEL_FULL})
            self.tier["model"] = TIER_MODEL_FULL
            self.wire.sleep(rec["id"])
            self.wait_for("crown parked", lambda: word_of(
                self.agent_of(self.board(), self.crown)["state"]) == "sleeping", 60)
            return self.restart_crown()
        self.crown_model = model

    def restart_crown(self):
        rec = self.agent_of(self.board(), self.crown)
        self.wire.request({"cmd": "resume_session", "id": rec["id"], "confirm": False})
        self.wait_for("crown woken", self.crown_settled, 120, self.watch)
        self.crown_model = model_of(transcript(self.agent_of(self.board(), self.crown)))

    def crown_settled(self, after_ms=None, quiet=QUIET):
        """The crown is idle and has been for `quiet` seconds; with
        `after_ms`, only once a turn of its ended (its `Stop`) after then."""
        rec = self.agent_of(self.board(), self.crown)
        if not rec:
            return False
        w = word_of(rec["state"])
        if w not in ("idle",) and not (w == "unknown" and self.restarted):
            self.quiet_since = None
            return False
        if after_ms is not None and not any(
            l.get("kind") == "hook" and of_session(l, rec["id"]) and l.get("event") == "Stop"
            and l.get("at_ms", 0) > after_ms for l in self.feed_lines
        ):
            return False
        if self.quiet_since is None:
            self.quiet_since = time.time()
        return time.time() - self.quiet_since >= quiet

    def watch(self):
        """Read the feed and the board, and print what changed: the rig's
        live view, in words."""
        lines, self.feed_off = read_feed(self.paths, self.feed_off)
        self.feed_lines.extend(lines)
        polled_ms = int(time.time() * 1000)
        board = self.board()
        crown = self.agent_of(board, self.crown)
        if crown:
            self.note_word("crown", crown, polled_ms)
        test = self.current
        workers = []
        for tid, key, _ in (self.test_tickets(test) if test else []):
            rec = self.agent_of(board, tid)
            if rec:
                workers.append((key, rec))
                # The card's words from the feed first: a 1.5 s `working`
                # between two polls is still a word the card said (D1).
                for l in lines:
                    if l.get("kind") == "session_state" and of_session(l, rec["id"]):
                        self.history.setdefault(key, []).append((l.get("at_ms", polled_ms), feed_word(l)))
                self.note_word(key, rec, polled_ms)
        tickets = {self.crown} | {tid for tid, _, _ in (self.test_tickets(test) if test else [])}
        for l in lines:
            kind = l.get("kind")
            mine = next((key for key, rec in workers if of_session(l, rec["id"])), None)
            if kind == "hook" and mine:
                self.log(f"{mine} frame {l['event']}" + (f" ({l['reason']})" if l.get("reason") else "")
                         + ("" if l.get("road") == "mod" or l.get("event") == "PaneDied" else " ∙ NOT by the mod"))
            elif kind == "board" and l.get("ticket") in tickets and l.get("actor") == "agent":
                self.log(f"crown {l.get('cmd')}" + (f" → {l['outcome']}" if l.get("outcome") else ""))
            elif kind == "board" and l.get("ticket") in tickets and l.get("cmd") in TURN_ROAD_WORDS:
                self.log(f"{l['cmd']}" + (f" → {l['outcome']}" if l.get("outcome") else ""))
            elif kind == "board" and str(l.get("cmd", "")).startswith("claude_road"):
                self.log(f"{l['cmd']}: {l.get('outcome')}")
            elif kind == "crown_wake":
                self.log(f"the board wakes the crown: {l.get('cause')}")

    def note_word(self, who, rec, at_ms):
        """A poll's word: printed when it changed, and kept with its time
        beside the feed's (a park writes no `session_state` line, so the
        polls are what see it)."""
        w = word_of(rec["state"])
        state = rec["state"]
        detail = state.get("reason") or state.get("stop_reason") or ""
        key = (w, detail)
        if self.words.get(who) != key:
            self.words[who] = key
            self.history.setdefault(who, []).append((at_ms, w))
            road = rec.get("road", "hooks")
            self.log(f"{who} {w.replace('_', ' ')}" + (f" ({detail})" if detail else "")
                     + (f" ∙ road {road}" if who != "crown" else ""))

    def log(self, text):
        say(f"  {time.time() - self.t0:7.1f}s  {text}")

    def worker_lines(self, sid, since=0):
        return [l for l in self.feed_lines[since:] if of_session(l, sid)]

    def run_step(self, test, step, record):
        if "rig" in step:
            return self.rig_step(test, step["rig"], record, step)
        words = self.with_close_out(step["crown"].format(**self.keys_of(test)))
        say(f"  → crown: {words}")
        mark = len(self.feed_lines)
        self.quiet_since = None
        sent_ms = int(time.time() * 1000)
        self.wire.prompt(self.crown, words, queued=False)
        self.restarted = False
        until = step.get("until", {})
        tickets = [t for t, _, _ in self.test_tickets(test)] if until.get("all") else [test["ticket"]]
        if until.get("on"):
            # One sibling's turn (R9's second worker): `on` names its key.
            on = self.keys_of(test)[until["on"]]
            tickets = [t for t, k, _ in self.test_tickets(test) if k == on]

        def one_done(tid):
            rec = self.agent_of(self.board(), tid)
            if not rec:
                return False
            mine = self.worker_lines(rec["id"], mark)
            stops = sum(1 for l in mine if l.get("kind") == "hook" and l.get("event") == "Stop")
            if stops < until.get("stops", 0):
                return False
            for e in until.get("events", []):
                if not any(l.get("kind") == "hook" and l.get("event") == e for l in mine):
                    return False
            if until.get("fed") and not any(
                l.get("kind") == "board" and l.get("cmd") == until["fed"] and l.get("ticket") == tid
                for l in self.feed_lines[mark:]
            ):
                return False
            return word_of(rec["state"]) == until.get("state", "idle")

        def worker_done():
            return all(one_done(t) for t in tickets)

        if not self.wait_for("worker", worker_done, STEP_TIMEOUT, self.watch):
            record["failures"].append(f"timed out waiting for {test['key']}: {until}")
            return False
        # A step the next one must follow at once (P2's person, while the
        # worker's turn runs) does not wait for the crown: its turn ends on
        # its own, and a prompt sent to it meanwhile waits for that end.
        if step.get("settle") is False:
            return True
        # Where the board wakes the crown for its worker (`wake`), the step
        # ends when the crown's turn on that wake ended (its Stop) and it has
        # been quiet; elsewhere, when it has been quiet. A wake the step did
        # not expect still resets the quiet, since the crown works on it.
        recs = [self.agent_of(self.board(), t) for t in tickets]
        final_at = max((l["at_ms"] for rec in recs for l in self.worker_lines(rec["id"], mark)
                        if l.get("kind") == "session_state"), default=0)
        wake = None
        if step.get("wake"):
            wake = self.wait_for("the board's wake for the crown", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") in tickets and l.get("at_ms", 0) >= final_at - 500),
                None), WAKE_GRACE, self.watch)
            record["wakes"].append(wake.get("cause") if wake else "none came")
        self.quiet_since = None
        # Every crown step is a turn of the crown's on the step's words: it
        # has settled once a Stop of its came after them (or after the wake).
        # A turn on the board's wake is the step's last, so its Stop is the
        # end with no quiet after it; elsewhere the crown must be quiet.
        after = wake["at_ms"] if wake else sent_ms
        settled = lambda: self.crown_settled(after_ms=after, quiet=0 if wake else QUIET)  # noqa: E731
        if not self.wait_for("crown", settled, STEP_TIMEOUT, self.watch):
            record["failures"].append("the crown did not settle")
            return False
        self.closed()
        return True

    def rig_step(self, test, what, record, step=None):
        step = step or {}
        say(f"  → rig: {what}")
        if what == "prompt":
            # The person's words to the worker while its turn runs (P2).
            rec = self.agent_of(self.board(), test["ticket"])
            began = next((l["at_ms"] for l in reversed(self.feed_lines) if l.get("kind") == "hook"
                          and of_session(l, rec["id"]) and l.get("event") == "UserPromptSubmit"), None)
            wait = max(0.0, step.get("after", 0) - (time.time() - (began or 0) / 1000))
            time.sleep(min(wait, step.get("after", 0)))
            rec = self.agent_of(self.board(), test["ticket"])
            if word_of(rec["state"]) != "working":
                record["failures"].append(f"the worker was {word_of(rec['state'])}, not working, "
                                          "when the person's prompt was due")
                return False
            try:
                r = self.wire.prompt(test["ticket"], step["text"], queued=False)
            except WireError as e:
                record["failures"].append(f"the person's prompt: {e}")
                return False
            self.log(f"the person's prompt to a working {test['key']}: {r.get('resp')}")
            return True
        if what == "press_enter":
            # The person answers the worker's dialog in its pane (D4): Enter
            # on the row the cursor rests on, the first. The worker's turn
            # then ends, the board wakes the crown, and the crown settles.
            rec = self.agent_of(self.board(), test["ticket"])
            mark = len(self.feed_lines)
            now_ms = int(time.time() * 1000)
            out = self.tmux("send-keys", "-t", sid16(rec["id"]), "Enter")
            self.log(f"the person pressed Enter in {test['key']}'s pane ({out.returncode})")

            def finished():
                r = self.agent_of(self.board(), test["ticket"])
                stopped = any(l.get("kind") == "hook" and of_session(l, r["id"])
                              and l.get("event") == "Stop" for l in self.feed_lines[mark:])
                return stopped and word_of(r["state"]) == "idle"

            if not self.wait_for("the worker's turn on the person's answer", finished,
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the worker never finished on the person's answer")
                return False
            wake = self.wait_for("the board's wake for the crown", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == test["ticket"]), None), WAKE_GRACE, self.watch)
            self.quiet_since = None
            after = wake["at_ms"] if wake else now_ms
            self.wait_for("crown", lambda: self.crown_settled(after_ms=after, quiet=0) if wake
                          else self.crown_settled(), STEP_TIMEOUT, self.watch)
            return True
        if what == "resend":
            # The person's Shift+Enter over the seat's unsent words (P4).
            try:
                r = self.wire.request({"cmd": "prompt_session", "ticket": test["ticket"],
                                       "text": "", "queued": False, "resend": True})
            except WireError as e:
                record["failures"].append(f"resend: {e}")
                return False
            self.log(f"the person's resend for {test['key']}: {r.get('resp')}")
            return True
        if what == "restart_drop_mod":
            folder = self.drop_mod()
            ok = self.restart({"MESIMON_MOD_DIR": folder}, want_probe="the mod validated")
            self.restarted = True
            if not ok:
                record["failures"].append("the daemon never came up with P4's mod")
            return ok
        if what in ("phone_allow", "phone_deny"):
            # The phone stands in at the socket the phone mod's approve dials
            # (T-581); the daemon comes up with that mod once.
            self.phone = self.phone or Phone()
            self.phone.answer = "allow" if what == "phone_allow" else "deny"
            if self.on_phone_mod:
                return True
            ok = self.restart({"MESIMON_MOD_DIR": self.phone_mod()}, want_probe="the mod validated")
            self.restarted = True
            self.on_phone_mod = ok
            if not ok:
                record["failures"].append("the daemon never came up with the phone mod")
            return ok
        if what == "still_waiting":
            # The dialog stands with nobody to answer it (C3): `seconds`
            # later the card still says needs you and the command has not run.
            time.sleep(step.get("seconds", 20))
            rec = self.agent_of(self.board(), test["ticket"])
            path = os.path.join(rec["cwd"], step.get("file", ""))
            word = word_of(rec["state"])
            ran = os.path.exists(path)
            record["still_waiting"] = (word == "needs_you" and not ran,
                                       f"after {step.get('seconds', 20)} s: {word}, "
                                       f"{step.get('file')} {'exists' if ran else 'absent'}")
            self.log(f"still waiting: {record['still_waiting'][1]}")
            return True
        if what == "ping":
            rec = self.agent_of(self.board(), test["ticket"])
            for attempt in (1, 2, 3):
                try:
                    ms = self.wire.mod_ping(rec["id"])
                    record["pings"].append(f"{ms} ms")
                    self.log(f"pong in {ms} ms")
                    return True
                except (WireError, OSError) as e:
                    self.log(f"ping {attempt}: {e}")
                    time.sleep(2)
            record["pings"].append("no pong")
            record["failures"].append("ping: no pong")
            return False
        if what == "restart":
            self.restart()
            self.restarted = True
            return True
        if what == "restart_old_claude":
            wrapper = os.path.join(self.out, "claude-2.1.286")
            with open(wrapper, "w") as f:
                f.write("#!/bin/sh\n# The rig's R7 (T-588): a Claude Code that says it is 2.1.286.\n"
                        'if [ "$1" = "--version" ]; then echo "2.1.286 (Claude Code)"; exit 0; fi\n'
                        f'exec "{self.claude}" "$@"\n')
            os.chmod(wrapper, 0o755)
            ok = self.restart({"MESIMON_CLAUDE_BIN": wrapper}, want_probe="older than 2.1.287")
            self.restarted = True
            if not ok:
                record["failures"].append("the daemon never came up with the old Claude Code")
            return ok
        if what == "restart_mods_off":
            # R9 (T-598): a Claude Code whose mods are off. It runs the real
            # one, which validates the mod and passes the load probe, and
            # starts a session with no `--plugin-dir`, so the mod never
            # loads: what 2.1.288's remote flag did to the mod road, whether
            # or not that flag is off today.
            wrapper = os.path.join(self.out, "claude-mods-off")
            with open(wrapper, "w") as f:
                f.write("#!/bin/sh\n# The rig's R9 (T-598): a Claude Code whose mods are off.\n"
                        'case "$1" in --version|plugin) exec "' + self.claude + '" "$@" ;; esac\n'
                        "skip=0\n"
                        "for a do\n"
                        "  shift\n"
                        '  if [ "$skip" = 1 ]; then skip=0; continue; fi\n'
                        '  if [ "$a" = "--plugin-dir" ]; then skip=1; continue; fi\n'
                        '  set -- "$@" "$a"\n'
                        "done\n"
                        f'exec "{self.claude}" "$@"\n')
            os.chmod(wrapper, 0o755)
            ok = self.restart({"MESIMON_CLAUDE_BIN": wrapper}, want_probe="the mod validated")
            self.restarted = True
            if not ok:
                record["failures"].append("the daemon never came up on the mods-off Claude Code")
            return ok
        if what == "restart_plain":
            ok = self.restart(want_probe="the mod validated")
            self.restarted = True
            self.on_phone_mod = False
            if not ok:
                record["failures"].append("the daemon did not come back on the mod")
            return ok
        if what == "doctor_mcp":
            out = subprocess.run([self.bin, "doctor", "--mcp"], cwd=self.repo, capture_output=True,
                                 text=True, env=terminal_env(), timeout=120)
            record["doctor_mcp"] = out.stdout
            self.log(f"doctor --mcp: {len(out.stdout.splitlines())} lines")
            return True
        if what == "doctor":
            out = subprocess.run([self.bin, "doctor"], cwd=self.repo, capture_output=True,
                                 text=True, env=terminal_env(), timeout=120)
            line = next((l.strip() for l in out.stdout.splitlines() if "claude road" in l), "")
            record["doctor"] = line
            self.log(f"doctor: {line}")
            return True
        die(f"unknown rig step {what}")

    def run_test(self, test):
        self.current = test
        self.words.pop(test["key"], None)
        self.history.pop(test["key"], None)
        start_mark = len(self.feed_lines)
        t0 = time.time()
        say(f"\n▶ {test['id']} {test['key']}: {test['title']}")
        record = {"failures": [], "pings": [], "doctor": None, "wakes": []}
        ok = True
        for step in test["steps"]:
            if ok:
                ok = self.run_step(test, step, record)
            elif step.get("rig") == "restart_plain":
                # A failed R7 still gives the board back its own Claude Code.
                self.run_step(test, step, record)
        time.sleep(SWEEP_WAIT)
        self.watch()
        # Evidence, not a gate: is this worker's mod alive end to end now?
        rec = self.agent_of(self.board(), test["ticket"])
        if rec and rec.get("road") == "mod" and word_of(rec["state"]) not in ("sleeping", "exited"):
            try:
                record["alive"] = f"pong in {self.wire.mod_ping(rec['id'])} ms"
            except (WireError, OSError) as e:
                record["alive"] = f"no pong: {e}"
            self.log(f"{test['key']} mod: {record['alive']}")
        verdict = self.judge(test, start_mark, record, time.time() - t0)
        if not ok:
            verdict["pass"] = False
        self.record(test, verdict)
        self.current = None
        return verdict

    # ---- verdicts

    def judge(self, test, mark, record, seconds):
        lines = self.feed_lines[mark:]
        results = []
        tickets = self.test_tickets(test)
        for tid, key, title in tickets:
            prefix = f"{key} " if len(tickets) > 1 else ""
            for name, ok, observed in self.check_ticket(test, tid, title, lines, record):
                results.append((prefix + name, ok, observed))
        rec = self.agent_of(self.board(), test["ticket"])
        rows = transcript(rec) if rec else []
        mine = [l for l in lines if rec and of_session(l, rec["id"])]
        hooks = [l["event"] for l in mine if l.get("kind") == "hook"]
        words = self.card_words(test["key"])
        timing = self.timing(mine)
        return {
            "id": test["id"], "key": test["key"], "expect": test["expect"],
            "results": results, "failures": record["failures"], "seconds": seconds,
            "timing": timing, "hooks": hooks, "words": words,
            "pass": all(ok for _, ok, _ in results) and not record["failures"],
            "model": model_of(rows),
            "wakes": [l.get("cause") for l in lines if l.get("kind") == "crown_wake"
                      and l.get("worker") == test["ticket"]],
            "alive": record.get("alive"),
        }

    def check_ticket(self, test, tid, title, lines, record):
        board = self.board()
        rec = self.agent_of(board, tid)
        key = self.ticket(board, tid)["short_key"]
        rows = transcript(rec) if rec else []
        mine = [l for l in lines if rec and of_session(l, rec["id"])]
        hooks = [l["event"] for l in mine if l.get("kind") == "hook"]
        # The card's words: the feed's `session_state` lines and the rig's
        # polls of the snapshot (a park writes no line), in time order.
        words = self.card_words(key)
        crown_cmds = [l.get("cmd") for l in lines if l.get("kind") == "board"
                      and l.get("actor") == "agent" and l.get("ticket") == tid]
        fed = [l.get("cmd") for l in lines if l.get("kind") == "board" and l.get("ticket") == tid]
        results = []

        def check(name, ok, observed):
            results.append((name, bool(ok), observed))

        for c in test["checks"]:
            name, _, arg = c.partition(":")
            if rec is None:
                check(c, False, "no agent record on the ticket")
            elif name == "road":
                check(c, rec.get("road", "hooks") == arg, f"road {rec.get('road', 'hooks')}")
            elif name == "plugin_dir":
                check(c, "--plugin-dir" in rec["argv"], "--plugin-dir in argv"
                      if "--plugin-dir" in rec["argv"] else "no --plugin-dir in argv")
            elif name == "no_plugin_dir":
                check(c, "--plugin-dir" not in rec["argv"], "no --plugin-dir in argv"
                      if "--plugin-dir" not in rec["argv"] else "--plugin-dir in argv")
            elif name == "started_by_crown":
                check(c, rec.get("started_by") == self.crown, f"started by {rec.get('started_by')}")
            elif name == "cwd":
                at_root = os.path.realpath(rec["cwd"]) == self.repo
                ok = at_root if arg == "checkout" else (not at_root and "/worktrees/" in rec["cwd"])
                check(c, ok, "the checkout" if at_root else rec["cwd"].replace(os.environ["HOME"], "~"))
            elif name == "first_prompt":
                first = flat(first_prompt(rows))
                ok = flat(test["brief"]) in first and title in first
                check(c, ok, f"first prompt {len(first)} chars, title and brief "
                      + ("whole" if ok else f"missing: {first[:80]!r}"))
            elif name == "first_prompt_exact":
                first = first_prompt(rows)
                want = f"{title}\n\n{test['brief'].rstrip()}"
                ok = first == want and "<pasted_content" not in first
                said = (f"{len(first)} chars, byte for byte" if ok else
                        f"{len(first)} chars against {len(want)}: "
                        f"{next((i for i, (a, b) in enumerate(zip(first, want)) if a != b), min(len(first), len(want)))} "
                        f"is the first byte that differs; head {first[:60]!r}")
                check(c, ok, said)
            elif name == "words":
                want = arg.split(",")
                check(c, is_subsequence(want, words), " → ".join(words) or "none")
            elif name == "hooks":
                want = arg.split(",")
                missing = [e for e in want if e not in hooks]
                counts = ", ".join(f"{e}×{hooks.count(e)}" for e in want if e in hooks)
                check(c, not missing, counts + (f"; missing {', '.join(missing)}" if missing else ""))
            elif name == "mod_alone":
                # T-577: the mod carries the session alone. No hook set, no
                # MCP server and no allow rule on argv; every frame the
                # daemon took for it came by the mod (tmux's pane-died aside).
                flags = [f for f in ("--settings", "--mcp-config", "--allowedTools") if f in rec["argv"]]
                frames = [l for l in mine if l.get("kind") == "hook" and l.get("event") != "PaneDied"]
                other = [l["event"] for l in frames if l.get("road") != "mod"]
                ok = rec.get("road") == "mod" and not flags and not other and frames
                check(c, ok, f"{len(frames)} frames, all by the mod; no hook set or MCP flag on argv"
                      if ok else f"flags {flags}; frames not by the mod: {other}; road {rec.get('road')}")
            elif name == "no_shim":
                out = subprocess.run(["ps", "-axww", "-o", "command="], capture_output=True, text=True)
                shims = [l for l in out.stdout.splitlines() if " mcp " in f" {l} "
                         and f"--session {rec['id']}" in l and "--call" not in l]
                check(c, not shims, "no `mesimon mcp` server for the session" if not shims
                      else f"{len(shims)} running: {shims[0][:80]}")
            elif name == "tools":
                calls = tool_calls(rows)
                missing, failed = [], []
                for tool in arg.split(","):
                    got = [r for n, r in calls if n == f"mcp__mesimon__{tool}"]
                    if not got:
                        missing.append(tool)
                    elif not any(r and not r.get("is_error") for r in got):
                        failed.append(f"{tool}: {(got[-1] or {}).get('text', 'no result')[:60]!r}")
                check(c, not missing and not failed,
                      f"{len(arg.split(','))} served with no error" if not missing and not failed
                      else f"missing {missing}; refused {failed}")
            elif name == "never":
                check(c, arg not in words, " → ".join(words) or "none")
            elif name == "plan_mode":
                argv = rec["argv"]
                mode = next((argv[i + 1] for i, a in enumerate(argv[:-1]) if a == "--permission-mode"), None)
                check(c, mode == "plan", f"--permission-mode {mode}")
            elif name == "doctor_mcp":
                text = record.get("doctor_mcp") or ""
                names = ["get_ticket", "list_board", "read_note", "write_note", "tag_ticket",
                         "create_ticket", "move_ticket", "raise_hand"]
                absent = [n for n in names if f"mcp__mesimon__{n}" not in text]
                says = "The mod registers the same tools" in " ".join(text.split())
                check(c, says and not absent, "lists the tools and says the mod registers them"
                      if says and not absent else f"absent {absent}; mod paragraph {'seen' if says else 'missing'}")
            elif name == "gate_refused":
                path = os.path.join(rec["cwd"], test["gate_file"])
                writes = [r for n, r in tool_calls(rows) if n == "Write"]
                refused = [r for r in writes if r and r.get("is_error")
                           and "mesimon owns .mesimon/" in r.get("text", "")]
                ok = bool(refused) and not os.path.exists(path)
                check(c, ok, f"{len(writes)} Write, refused in mesimon gate's words; "
                      f"{test['gate_file']} not created" if ok else
                      f"writes {[(r or {}).get('text', '')[:60] for r in writes]}; exists {os.path.exists(path)}")
            elif name == "claude_road_mod":
                seen = [l for l in feed_all(self.paths) if l.get("cmd") == "claude_road:mod"]
                check(c, seen, f"claude_road:mod ({seen[-1].get('outcome')})" if seen else "never written")
            elif name == "crown":
                check(c, arg in crown_cmds, f"{arg}×{crown_cmds.count(arg)}")
            elif name == "fed":
                check(c, arg in fed, f"{arg}×{fed.count(arg)}")
            elif name == "board_fed":
                # A board line about no ticket (the road's verdict), since
                # the test began.
                said = [l for l in lines if l.get("kind") == "board" and l.get("cmd") == arg]
                check(c, said, f"{arg}: {said[-1].get('outcome')}" if said else f"no {arg} line")
            elif name == "relaunched":
                # T-598: the test's own ticket was relaunched on the hook set
                # once, within `arg` seconds of the crown's start_agent; a
                # sibling started after it took the hook set at once and was
                # never relaunched.
                again = [l for l in lines if l.get("cmd") == "claude_road_relaunch"
                         and l.get("ticket") == tid]
                started = next((l["at_ms"] for l in lines if l.get("cmd") == "start_agent"
                                and l.get("ticket") == tid and l.get("actor") == "agent"), None)
                if tid == test["ticket"]:
                    secs = (again[0]["at_ms"] - started) / 1000 if again and started else None
                    ok = len(again) == 1 and secs is not None and secs <= float(arg)
                    check(c, ok, f"relaunched {len(again)}× , {secs:.1f} s after start_agent"
                          if secs is not None else f"relaunched {len(again)}×; start_agent at {started}")
                else:
                    check(c, not again, "never relaunched: the hook set at once"
                          if not again else f"relaunched {len(again)}×")
            elif name == "not_fed":
                check(c, arg not in fed, f"{arg}×{fed.count(arg)}")
            elif name == "mod_report":
                event, _, reason = arg.partition(":")
                seen = [l for l in mine if l.get("kind") == "hook" and l.get("event") == event
                        and (not reason or l.get("reason") == reason)]
                check(c, seen, f"{event}" + (f" ({reason})" if reason else "") + f"×{len(seen)}")
            elif name == "prompts":
                got = user_prompts(rows)
                check(c, len(got) == int(arg), f"{len(got)} prompts: "
                      + "; ".join(repr(p[:30]) for p in got))
            elif name == "held":
                # The second prompt entered after the first turn's Stop: the
                # engine held it for its own turn. The Stop is the daemon's
                # clock, the hook's exec after the turn's end (40-116 ms,
                # T-573), so a second's slack; a prompt delivered into the
                # turn would sit the whole sleep before it.
                got = user_prompts(rows, at=True)
                stops = [l["at_ms"] for l in mine if l.get("kind") == "hook" and l.get("event") == "Stop"]
                second = iso_ms(got[1][1]) if len(got) > 1 else None
                ok = bool(second and stops and stops[0] - 1000 <= second)
                check(c, ok, f"first Stop at {stops[0] if stops else None}, second prompt entered "
                      f"at {second}" + (f" ({(second - stops[0]) / 1000:+.1f} s)" if ok else ""))
            elif name == "replies":
                want = arg.split(",")
                said = replies(rows)
                bare = [r.strip().strip(".!").strip().lower() for r in said]
                hit = is_subsequence(want, [b if b in want else None for b in bare])
                check(c, hit, " → ".join(repr(r[:24]) for r in said) or "none")
            elif name == "file":
                path = os.path.join(rec["cwd"], arg)
                try:
                    with open(path) as f:
                        held = f.read()
                except OSError:
                    held = None
                ok = held is not None and test.get("brief_file", "") in held
                check(c, ok, f"{arg}: {held.strip()[:40]!r}" if held is not None else f"no {arg}")
            elif name == "exists" or name == "absent":
                there = os.path.exists(os.path.join(rec["cwd"], arg))
                check(c, there == (name == "exists"), f"{arg} {'exists' if there else 'absent'}")
            elif name == "phone":
                got = [p for p in (self.phone.log if self.phone else []) if p["session"] == rec["id"]]
                ok = len(got) == 1 and got[0]["answer"] == arg
                check(c, ok, "; ".join(f"{p['tool']} → {p['answer']}" for p in got) or "never asked")
            elif name == "denied_verbatim":
                # The tool result the transcript records is what the model
                # read; no plugin's name rides it (T-573 row 4: a tool.check
                # deny would say "denied by plugin mesimon").
                bash = [r for n, r in tool_calls(rows) if n == "Bash"]
                text = (bash[-1] or {}).get("text", "") if bash else ""
                said = last_reply(rows)
                ok = bool(bash) and (bash[-1] or {}).get("is_error") and "plugin" not in text.lower()
                check(c, ok, f"the model read {text[:160]!r}; replied {said[:80]!r}")
            elif name == "still_waiting":
                ok, said = record.get("still_waiting", (False, "not measured"))
                check(c, ok, said)
            elif name == "cost_by_mod":
                try:
                    with open(os.path.join(self.paths.state_dir, "costs.json")) as f:
                        ledger = json.load(f)
                except (OSError, ValueError):
                    ledger = {}
                t = ledger.get("tickets", {}).get(tid, {})
                chk = t.get("check", {})
                by_mod, by_tail = chk.get("mod", 0), chk.get("tail", 0)
                hours = sum(sum(v.values()) for m in t.get("hours", {}).values() for v in m.values())
                ok = by_mod > 0 and hours > 0 and abs(by_mod - by_tail) * 50 <= max(by_mod, by_tail)
                check(c, ok, f"mod {by_mod}, transcript {by_tail} tokens past the fence; "
                      f"{hours} on the ticket")
            elif name == "quota_by_mod":
                reading = (self.snapshot().get("usage", {}).get("claude") or {}).get("reading") or {}
                at = reading.get("read_at_ms", 0)
                # The windows are the account's: the last report of any of
                # the board's sessions (the crown's turns report too) wrote it.
                reports = [l["at_ms"] for l in lines if l.get("kind") == "hook"
                           and l.get("event") == "ModUsage"]
                kinds = [w.get("label") for w in reading.get("windows", [])]
                ok = any(abs(at - r) <= 1500 for r in reports) and "5h" in kinds
                check(c, ok, f"read at {at}, at a ModUsage of the board's ({len(reports)} in the test); windows {kinds}"
                      if ok else f"read at {at}, {len(reports)} ModUsage; windows {kinds}")
            elif name == "answered":
                said = [l for l in lines if l.get("cmd") == "answer_agent" and l.get("ticket") == tid]
                ok = any(l.get("outcome") == "answered" and l.get("answer", "").lower() == arg
                         for l in said)
                check(c, ok, "; ".join(f"{l.get('outcome')} {l.get('answer')!r}" for l in said)
                      or "no answer_agent line")
            elif name == "refused":
                said = [l for l in lines if l.get("cmd") == "answer_agent" and l.get("ticket") == tid]
                ok = not any(l.get("outcome") == "answered" for l in said)
                check(c, ok, "; ".join(f"{l.get('outcome')} ({l.get('reason') or ''})" for l in said)
                      or "refused: no answer_agent line")
            elif name == "pings":
                ok = record["pings"] and all(p.endswith(" ms") for p in record["pings"])
                check(c, ok, "pong " + ", then ".join(record["pings"]))
            elif name == "reply":
                said = last_reply(rows)
                check(c, arg.lower() in said.lower(), f"said {said[:60]!r}")
            elif name == "doctor":
                check(c, arg in (record["doctor"] or ""), record["doctor"] or "no line")
            elif name == "final":
                check(c, word_of(rec["state"]) == arg, word_of(rec["state"]))
            else:
                check(c, False, "unknown check")
        return results

    def card_words(self, key):
        seen = sorted(self.history.get(key, []), key=lambda p: p[0])
        return squeeze([w for _, w in seen])

    def timing(self, mine):
        at = [(feed_word(l), l["at_ms"]) for l in mine if l.get("kind") == "session_state"]
        parts = []
        first = next((t for w, t in at if w == "working"), None)
        idle = next((t for w, t in at if w == "idle" and first and t > first), None)
        if first and idle:
            parts.append(f"first turn {((idle - first) / 1000):.1f} s")
        frames = [l["at_ms"] for l in mine if l.get("kind") == "hook"]
        if frames:
            parts.append(f"{len(frames)} frames over {(frames[-1] - frames[0]) / 1000:.1f} s")
        return ", ".join(parts)

    def record(self, test, v):
        mark = "PASS" if v["pass"] else "FAIL"
        say(f"  {mark} {test['id']} in {v['seconds']:.0f} s ∙ {v['timing']}")
        for name, ok, observed in v["results"]:
            say(f"    {'✓' if ok else '✗'} {name}: {observed}")
        for f in v["failures"]:
            say(f"    ✗ {f}")
        note = [f"# {test['id']} verdict: {mark}", "",
                f"Run {self.run_id} ∙ {self.build} ∙ worker model {v['model']} ∙ {v['seconds']:.0f} s"
                + (f" ∙ {v['timing']}" if v["timing"] else ""), "",
                f"**Expected.** {test['expect']}", "",
                "| check | | observed |", "|---|---|---|"]
        note += [f"| `{n}` | {'✓' if ok else '✗'} | {o} |" for n, ok, o in v["results"]]
        note += [f"| step | ✗ | {f} |" for f in v["failures"]]
        note += ["", f"**Frames the daemon ingested** (in order): {', '.join(v['hooks']) or 'none'}.",
                 f"**Card words seen:** {' → '.join(v['words']) or 'none'}.",
                 f"**The board woke the crown for it:** {', '.join(v['wakes']) or 'never'}.",
                 f"**Its mod at the end:** {v['alive'] or 'no pane to ask'}."]
        self.wire.write_note(test["ticket"], "\n".join(note))
        for tid, key, _ in self.test_tickets(test):
            rec = self.agent_of(self.board(), tid)
            if not rec:
                continue
            # An agent on the board's checkout is ended, not parked: a parked
            # one holds the checkout, and the next test that starts there is
            # refused it (T-583; T-577's T3 met R2's). Its conversation stays.
            if os.path.realpath(rec["cwd"]) == self.repo and word_of(rec["state"]) != "exited":
                try:
                    self.wire.request({"cmd": "kill_session", "id": rec["id"]})
                except WireError as e:
                    say(f"  could not end {key}: {e}")
            elif word_of(rec["state"]) not in ("sleeping", "exited", "failed"):
                try:
                    self.wire.sleep(rec["id"])
                except WireError as e:
                    say(f"  could not park {key}: {e}")
        # The crown moves a finished rig ticket to DONE (the author's rule),
        # at the head of the next words the rig sends it rather than in a
        # turn of its own; a failure stays where automove left it, in
        # REVIEW, for a person.
        v["column"] = self.ticket(self.board(), test["ticket"])["column"]
        if v["pass"]:
            self.close_out.append(test)
        self.verdicts.append(v)

    def with_close_out(self, words):
        """The step's words, headed by the moves owed for the tests that
        passed since the crown was last told anything."""
        if not self.close_out:
            return words
        owed = []
        for test in self.close_out:
            keys = [k for _, k, _ in self.test_tickets(test)]
            each = ", then ".join(f"for {k}" for k in keys)
            owed.append(f"{test['id']} passed; its verdict is a note on {test['key']}: call "
                        f"move_ticket {each} to DONE.")
        self.closing = self.close_out
        self.close_out = []
        return " ".join(owed) + " Then " + words

    def closed(self):
        """Read back the moves the crown was told to make: each ticket's
        column, as the table says it."""
        board = self.board()
        for test in self.closing:
            col = self.ticket(board, test["ticket"])["column"]
            for v in self.verdicts:
                if v["id"] == test["id"]:
                    v["column"] = col
            self.log(f"{test['key']} is in {col}")
        self.closing = []

    def finish(self):
        """The last test's move, with the run's last words."""
        if not self.close_out:
            self.closed()
            return
        words = self.with_close_out("end your turn with the single line: run done.")
        say(f"\n  → crown: {words}")
        mark = max((l.get("at_ms", 0) for l in self.feed_lines), default=0)
        self.quiet_since = None
        self.wire.prompt(self.crown, words, queued=False)
        self.wait_for("the crown's moves", lambda: self.crown_settled(after_ms=mark),
                      STEP_TIMEOUT, self.watch)
        self.closed()

    def table(self):
        out = [f"# The rig's verdicts, run {self.run_id}", "",
               f"Build: {self.build}. Crown model: {self.crown_model}. "
               f"Run time: {(time.time() - self.t0) / 60:.1f} min.", "",
               "| test | expected | observed | build |", "|---|---|---|---|"]
        for v in self.verdicts:
            failed = [f"✗ {n}: {o}" for n, ok, o in v["results"] if not ok] + [f"✗ {f}" for f in v["failures"]]
            observed = ("PASS ∙ " + "; ".join(o for _, _, o in v["results"])) if v["pass"] else (
                "FAIL ∙ " + "; ".join(failed))
            observed += f" ∙ ticket in {v.get('column')}"
            out.append(f"| {v['id']} {v['key']} | {v['expect']} | {observed} | {self.build} |")
        text = "\n".join(out) + "\n"
        for path in (os.path.join(self.out, "verdicts.md"),
                     os.path.join(self.out, f"verdicts-{self.run_id}.md")):
            with open(path, "w") as f:
                f.write(text)
        say("\n" + text)
        say(f"written to {os.path.relpath(os.path.join(self.out, 'verdicts.md'), self.repo)}")

    # ---- the whole run

    def run(self, tests, lay_only):
        self.run_id = time.strftime("%Y%m%d-%H%M%S")
        self.feed_lines, self.feed_off = [], 0
        _, self.feed_off = read_feed(self.paths)
        self.words, self.current, self.quiet_since, self.restarted = {}, None, None, False
        self.history = {}
        self.on_phone_mod = False
        self.verdicts = []
        self.close_out, self.closing = [], []
        self.t0 = time.time()
        say("\n▶ laying the board")
        self.lay()
        self.clear_previous()
        self.file(tests)
        if lay_only:
            say("\nlaid; nothing started. Run again without --lay to run the tests.")
            return
        self.start_crown()
        for test in tests:
            self.run_test(test)
        self.finish()
        self.table()

    def reset(self):
        if not self.alive():
            say("  no rig daemon is running")
        else:
            self.connect()
            board = self.board()
            say(f"  parking {sum(1 for s in board['sessions'] if word_of(s['state']) not in ('sleeping', 'exited'))} sessions")
            self.park_all(board)
            n = 0
            for t in self.board()["tickets"]:
                if not t.get("archived"):
                    try:
                        self.wire.archive(t["id"])
                        n += 1
                    except WireError as e:
                        say(f"  could not archive {t['short_key']}: {e}")
            say(f"  archived {n} tickets")
            self.stop_daemon()
        tmux = os.environ.get("MESIMON_TMUX_BIN") or (
            os.path.join(self.repo, "target/debug/mesimon-tmux")
            if os.path.exists(os.path.join(self.repo, "target/debug/mesimon-tmux")) else "tmux")
        if os.path.exists(self.paths.tmux_sock):
            out = subprocess.run([tmux, "-S", self.paths.tmux_sock, "kill-server"],
                                 capture_output=True, text=True)
            say("  the rig's tmux server stopped" if out.returncode == 0
                else f"  tmux kill-server: {out.stderr.strip() or 'no server'}")


def main():
    global LOG
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--lay", action="store_true", help="lay the board and file the tests; start nothing")
    ap.add_argument("--only", help="a comma list of test ids, as R1,R3; a letter alone is its group")
    ap.add_argument("--failed", action="store_true",
                    help="only the tests the last run's verdicts.md lists as FAIL")
    ap.add_argument("--reset", action="store_true", help="park and archive everything, stop the daemon and tmux")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--flags-off", action="store_true",
                    help="Claude Code's flags service off for the rig's sessions (T-598's seam)")
    args = ap.parse_args()

    repo = git(os.getcwd(), "rev-parse", "--show-toplevel") or git(HERE, "rev-parse", "--show-toplevel")
    if not repo:
        die("not inside a git checkout")
    repo = os.path.realpath(repo)
    branch = git(repo, "rev-parse", "--abbrev-ref", "HEAD") or ""
    if branch == "main" or not branch.startswith("msmn/"):
        die(f"the rig runs only in a ticket worktree (a msmn/… branch); this checkout is on {branch!r}")
    rig = Rig(repo, args)
    rig.branch = branch
    os.makedirs(rig.out, exist_ok=True)
    LOG = open(os.path.join(rig.out, "rig.log"), "a")
    say(f"\n=== rig {time.strftime('%Y-%m-%d %H:%M:%S')} ∙ {branch} ∙ proj16 {rig.paths.proj16}")

    if args.reset:
        rig.reset()
        return

    with open(os.path.join(HERE, "rig", "tests.toml"), "rb") as f:
        tests = tomllib.load(f)["test"]
    if args.only:
        want = [x.strip() for x in args.only.split(",")]
        tests = [t for t in tests if t["id"] in want or t["id"][0] in want]
        if not tests:
            die(f"no test named {args.only}")
    if args.failed:
        failed = last_failed(os.path.join(rig.out, "verdicts.md"))
        if failed is None:
            die("no target/rig/verdicts.md to read the failures from")
        tests = [t for t in tests if t["id"] in failed]
        if not tests:
            die("the last run's verdicts list no FAIL")
        say(f"  --failed: {', '.join(t['id'] for t in tests)}")

    if not args.no_build:
        say("▶ cargo build -p mesimon")
        t = time.time()
        b = subprocess.run(["cargo", "build", "-p", "mesimon"], cwd=repo)
        if b.returncode != 0:
            die("the build failed")
        say(f"  built in {time.time() - t:.0f} s")
    if not os.path.exists(rig.bin):
        die(f"no {rig.bin}; build first")
    if not rig.claude:
        die("no claude on PATH")
    rig.claude_version = subprocess.run([rig.claude, "--version"], capture_output=True,
                                        text=True).stdout.split()[0]
    rig.build = f"claude {rig.claude_version} ∙ {branch}@{git(repo, 'rev-parse', '--short', 'HEAD')}"
    say("▶ the rig's daemon")
    rig.ensure_daemon()
    say(f"\nWatch the rig with:\n\n    cd {repo} && target/debug/mesimon\n")
    rig.run(tests, args.lay)


if __name__ == "__main__":
    main()
