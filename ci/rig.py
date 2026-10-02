#!/usr/bin/env python3
"""The rig (T-588): a mesimon board inside this ticket worktree drives the
real Claude Code on the mod road, one test at a time, through its own crown.

    python3 -B ci/rig.py            build, start or find the rig's daemon, lay
                                    the board, file the tests, run them all
    python3 -B ci/rig.py --lay      the same up to the crown filed and crowned;
                                    nothing starts, nothing costs
    python3 -B ci/rig.py --only R3  run the named tests (a comma list)
    python3 -B ci/rig.py --reset    park and archive the rig's tickets, stop its
                                    daemon and its private tmux
    --no-build                      skip `cargo build -p mesimon`

Watch it with the command it prints: `cd <worktree> && target/debug/mesimon`.

The rig is a second board on this worktree's own proj16 (its own state dir,
sockets and private tmux); it never touches the author's main board, and it
writes nothing under the machine layer (`tiers.toml`, `prefs.json`,
`usage.json`): its tier is a board tier, in the worktree's `.mesimon/`.
It drives the daemon over the wire (`ci/rig/wire.py`) and reads the feed and
the transcripts, never a screen. Its crown, a real Claude Code session on
Sonnet, is the subject under test: each test's words go to the crown, and
the crown starts, answers, asks, parks and wakes the workers through its
MCP tools. Tests and their exact words are `ci/rig/tests.toml`.

Each test's verdict is a note on its ticket. The crown moves a passed test's
ticket to DONE when the rig tells it to; a failed one stays in REVIEW. The
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
import subprocess
import sys
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
SWEEP_WAIT = 4.0  # the shadow calls a lone frame after 2 s
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
        env = terminal_env({"MESIMON_DETACHED": "1", **(extra_env or {})})
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
            if same:
                say(f"  found the rig's daemon: pid {hello['daemon_pid']}, this build")
                return
            say(f"  the rig's daemon (pid {hello['daemon_pid']}) runs another build: restarting it")
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
            if all(f"{k}={v}" in self.env_of(hello["daemon_pid"]) for k, v in (extra_env or {}).items()):
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
        text = text.replace("let bridgeOn = false\n", "let bridgeOn = false\nlet rigDropped = false\n", 1)
        os.remove(os.path.join(dst, "hooks", "register.test.ts"))
        with open(path, "w") as f:
            f.write(text)
        out = subprocess.run([self.claude, "plugin", "validate", dst], capture_output=True, text=True)
        if out.returncode != 0:
            die(f"P4's mod does not validate:\n{out.stdout}{out.stderr}")
        return dst

    def env_of(self, pid):
        out = subprocess.run(["ps", "-wwE", "-o", "command=", "-p", str(pid)],
                             capture_output=True, text=True)
        return out.stdout.split()

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
        for test in tests:
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
        tid = self.wire.create_ticket("TODO", title)
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

    def crown_settled(self, after_ms=None):
        """The crown is idle and has been for `QUIET` seconds; with
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
        return time.time() - self.quiet_since >= QUIET

    def watch(self):
        """Read the feed and the board, and print what changed: the rig's
        live view, in words."""
        lines, self.feed_off = read_feed(self.paths, self.feed_off)
        self.feed_lines.extend(lines)
        board = self.board()
        crown = self.agent_of(board, self.crown)
        if crown:
            self.note_word("crown", crown)
        test = self.current
        workers = []
        for tid, key, _ in (self.test_tickets(test) if test else []):
            rec = self.agent_of(board, tid)
            if rec:
                self.note_word(key, rec)
                workers.append((key, rec))
        tickets = {self.crown} | {tid for tid, _, _ in (self.test_tickets(test) if test else [])}
        for l in lines:
            kind = l.get("kind")
            mine = next((key for key, rec in workers if of_session(l, rec["id"])), None)
            if kind == "hook" and mine:
                self.log(f"{mine} frame {l['event']}" + (f" ({l['reason']})" if l.get("reason") else ""))
            elif kind == "road_disagree":
                self.log(f"!! road_disagree {l.get('cmd')} {l.get('outcome')} ×{l.get('count')} "
                         f"session {str(l.get('session'))[:8]}")
            elif kind == "board" and l.get("ticket") in tickets and l.get("actor") == "agent":
                self.log(f"crown {l.get('cmd')}" + (f" → {l['outcome']}" if l.get("outcome") else ""))
            elif kind == "board" and l.get("ticket") in tickets and l.get("cmd") in TURN_ROAD_WORDS:
                self.log(f"{l['cmd']}" + (f" → {l['outcome']}" if l.get("outcome") else ""))
            elif kind == "board" and str(l.get("cmd", "")).startswith("claude_road"):
                self.log(f"{l['cmd']}: {l.get('outcome')}")
            elif kind == "crown_wake":
                self.log(f"the board wakes the crown: {l.get('cause')}")

    def note_word(self, who, rec):
        w = word_of(rec["state"])
        state = rec["state"]
        detail = state.get("reason") or state.get("stop_reason") or ""
        key = (w, detail)
        if self.words.get(who) != key:
            self.words[who] = key
            self.history.setdefault(who, []).append(w)
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
        words = step["crown"].format(**self.keys_of(test))
        say(f"  → crown: {words}")
        mark = len(self.feed_lines)
        self.quiet_since = None
        sent_ms = int(time.time() * 1000)
        self.wire.prompt(self.crown, words, queued=False)
        self.restarted = False
        until = step.get("until", {})
        tickets = [t for t, _, _ in self.test_tickets(test)] if until.get("all") else [test["ticket"]]

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
        after = wake["at_ms"] if wake else sent_ms
        settled = lambda: self.crown_settled(after_ms=after)  # noqa: E731
        if not self.wait_for("crown", settled, STEP_TIMEOUT, self.watch):
            record["failures"].append("the crown did not settle")
            return False
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
            self.wait_for("crown", lambda: self.crown_settled(after_ms=after) if wake
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
        if what == "restart_plain":
            ok = self.restart(want_probe="the mod validated")
            self.restarted = True
            if not ok:
                record["failures"].append("the daemon did not come back on the mod")
            return ok
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
        words = squeeze(self.history.get(test["key"], []))
        disagree = [l for l in lines if l.get("kind") == "road_disagree"
                    and any((r := self.agent_of(self.board(), t)) and of_session(l, r["id"])
                            for t, _, _ in tickets)]
        timing = self.timing(mine)
        return {
            "id": test["id"], "key": test["key"], "expect": test["expect"],
            "results": results, "failures": record["failures"], "seconds": seconds,
            "timing": timing, "hooks": hooks, "words": words,
            "pass": all(ok for _, ok, _ in results) and not record["failures"],
            "model": model_of(rows),
            "wakes": [l.get("cause") for l in lines if l.get("kind") == "crown_wake"
                      and l.get("worker") == test["ticket"]],
            "disagree": [f"{l.get('cmd')} {l.get('outcome')}×{l.get('count')}" for l in disagree],
            "alive": record.get("alive"),
        }

    def check_ticket(self, test, tid, title, lines, record):
        board = self.board()
        rec = self.agent_of(board, tid)
        key = self.ticket(board, tid)["short_key"]
        rows = transcript(rec) if rec else []
        mine = [l for l in lines if rec and of_session(l, rec["id"])]
        hooks = [l["event"] for l in mine if l.get("kind") == "hook"]
        # The card's words as the rig's polls of the snapshot saw them (a
        # park writes no `session_state` line, so the feed alone misses it).
        words = squeeze(self.history.get(key, []))
        disagree = [l for l in mine if l.get("kind") == "road_disagree"]
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
            elif name == "no_disagree":
                said = ", ".join(f"{l.get('cmd')} {l.get('outcome')}×{l.get('count')}" for l in disagree)
                check(c, not disagree and rec.get("road") == "mod",
                      f"0 road_disagree over {sum(1 for e in hooks if e != 'PaneDied')} frames"
                      if not disagree else said)
            elif name == "claude_road_mod":
                seen = [l for l in feed_all(self.paths) if l.get("cmd") == "claude_road:mod"]
                check(c, seen, f"claude_road:mod ({seen[-1].get('outcome')})" if seen else "never written")
            elif name == "crown":
                check(c, arg in crown_cmds, f"{arg}×{crown_cmds.count(arg)}")
            elif name == "fed":
                check(c, arg in fed, f"{arg}×{fed.count(arg)}")
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
                hit = is_subsequence(want, [next((w for w in want if w in r.lower()), None) for r in said])
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
        note += ["", f"**Frames the daemon ingested** (hook road, in order): {', '.join(v['hooks']) or 'none'}.",
                 f"**Card words seen:** {' → '.join(v['words']) or 'none'}.",
                 f"**The board woke the crown for it:** {', '.join(v['wakes']) or 'never'}.",
                 f"**road_disagree for its session:** {', '.join(v['disagree']) or 'none'}.",
                 f"**Its mod at the end:** {v['alive'] or 'no pane to ask'}."]
        self.wire.write_note(test["ticket"], "\n".join(note))
        for tid, key, _ in self.test_tickets(test):
            rec = self.agent_of(self.board(), tid)
            if rec and word_of(rec["state"]) not in ("sleeping", "exited", "failed"):
                try:
                    self.wire.sleep(rec["id"])
                except WireError as e:
                    say(f"  could not park {key}: {e}")
        # The crown moves a finished rig ticket to DONE (the author's rule);
        # a failure stays where automove left it, in REVIEW, for a person.
        v["column"] = self.ticket(self.board(), test["ticket"])["column"]
        if v["pass"]:
            keys = [k for _, k, _ in self.test_tickets(test)]
            each = ", then ".join(f"for {k}" for k in keys)
            words = (f"{test['id']} passed; its verdict is a note on {test['key']}. Call move_ticket "
                     f"{each} to DONE, then end your turn with the single line: "
                     f"{test['id']} done.")
            say(f"  → crown: {words}")
            mark = max((l.get("at_ms", 0) for l in self.feed_lines), default=0)
            self.quiet_since = None
            self.wire.prompt(self.crown, words, queued=False)
            self.wait_for("the crown's move", lambda: self.crown_settled(after_ms=mark),
                          STEP_TIMEOUT, self.watch)
            v["column"] = self.ticket(self.board(), test["ticket"])["column"]
            self.log(f"{test['key']} is in {v['column']}")
        self.verdicts.append(v)

    def table(self):
        disagree = [l for l in self.feed_lines if l.get("kind") == "road_disagree"]
        total = sum(int(l.get("count") or 1) for l in disagree)
        out = [f"# The rig's verdicts, run {self.run_id}", "",
               f"Build: {self.build}. Crown model: {self.crown_model}. "
               f"road_disagree over the run: {len(disagree)} lines, {total} frames.", "",
               "| test | expected | observed | build |", "|---|---|---|---|"]
        for v in self.verdicts:
            failed = [f"✗ {n}: {o}" for n, ok, o in v["results"] if not ok] + [f"✗ {f}" for f in v["failures"]]
            observed = ("PASS ∙ " + "; ".join(o for _, _, o in v["results"])) if v["pass"] else (
                "FAIL ∙ " + "; ".join(failed))
            if v["disagree"]:
                observed += f" ∙ road_disagree: {', '.join(v['disagree'])}"
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
        self.verdicts = []
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
    ap.add_argument("--only", help="a comma list of test ids, as R1,R3")
    ap.add_argument("--reset", action="store_true", help="park and archive everything, stop the daemon and tmux")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    repo = git(os.getcwd(), "rev-parse", "--show-toplevel") or git(HERE, "rev-parse", "--show-toplevel")
    if not repo:
        die("not inside a git checkout")
    repo = os.path.realpath(repo)
    branch = git(repo, "rev-parse", "--abbrev-ref", "HEAD") or ""
    if branch == "main" or not branch.startswith("msmn/"):
        die(f"the rig runs only in a ticket worktree (a msmn/… branch); this checkout is on {branch!r}")
    rig = Rig(repo, args)
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
        tests = [t for t in tests if t["id"] in want]
        if not tests:
            die(f"no test named {args.only}")

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
