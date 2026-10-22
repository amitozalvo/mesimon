#!/usr/bin/env python3
"""Real-agent verification: Claude crown/worker tests on the mod road, or
bounded native Codex cases with --provider codex (ci/rig/README.md).

    python3 -B ci/rig.py            build, start or find the rig's daemon, lay
                                    the board, file the tests, run them all
    python3 -B ci/rig.py --lay      the same up to the crown filed and crowned;
                                    nothing starts, nothing costs
    python3 -B ci/rig.py --only R3  run the named tests (a comma list); a
                                    letter alone names its group (R, P, D, T, C, S)
    python3 -B ci/rig.py --failed   run again only what the last run's
                                    verdicts.md lists as FAIL
    python3 -B ci/rig.py --reset    park and archive the rig's tickets, stop its
                                    daemon and its private tmux
    python3 -B ci/rig.py --stale    list the dead rig boards; remove nothing
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

Every run first clears the dead rig boards (T-765): a board keyed on a ticket
worktree of this repository that no longer exists, found by the checkout path
the rig records in `<state>/rig/parent` (or, for an older board, a session's
cwd whose proj16 is the board's). Its daemon and private tmux are stopped, its
worktrees removed, their msmn/ branches deleted and its state and runtime
dirs removed. A board whose ticket worktree still exists is never touched.
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
# A second board tier on the same model (T-808): where L2's crown moves a
# worker stopped on a usage limit. Never the default, and never the built-in
# `claude`, whose model is the account's own and may cost far more.
ALT_TIER_NAME = "rig-alt"
ALT_TIER_DESCRIPTION = "a worker the crown moves off a usage limit (L2)"
STEP_TIMEOUT = 600
# The stub forge CLI a rig test puts first on the daemon's PATH (T-739).
GH_STUB = """#!/bin/sh
case "$1 $2" in
  "auth status") exit 0 ;;
  "pr list") cat "$MSMN_GH_PRS" 2>/dev/null || echo '[]' ;;
  "pr view") cat "$(dirname "$MSMN_GH_PRS")/view.$3.json" 2>/dev/null || { echo "no pull requests found for $3" >&2; exit 1; } ;;
  "pr review"|"pr comment") exit 0 ;;
  "pr diff") echo "diff --git a/README.md b/README.md"; echo "+reviewed" ;;
  "pr checks") echo "All checks were successful" ;;
  api\\ *)
    case "$4" in
      *check-runs*) cat "$MSMN_GH_CHECKS" 2>/dev/null || echo '{"check_runs":[]}' ;;
      *deployments/*/statuses*) echo '[{"state":"success"}]' ;;
      *deployments*) echo '[]' ;;
      *) echo '{}' ;;
    esac ;;
  *) echo "stub gh: $*" >&2; exit 1 ;;
esac
"""
WAKE_GRACE = 60
QUIET = 6.0
SWEEP_WAIT = 1.0  # the last frames' feed lines, after the worker's end
# The column the permission tests' workers start in (T-581): Claude Code in
# manual mode, so a Bash command asks.
MANUAL_COLUMN = "RIG MANUAL"
# The rig stands in for the phone (T-581) at a socket the phone mod's
# `mesimon approve` dials, in each board's runtime dir (`Rig.phone_sock`, T-831).
# The phone holds each request this long, so the dialog is up and the card
# says so before it is answered.
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


def solid_png(color, size=64):
    """A `size`-square PNG of one colour, base64: what a picture test sends,
    so the worker's reply can name what it saw (T-853)."""
    import base64
    import struct
    import zlib
    rgb = {"red": (255, 0, 0), "green": (0, 160, 0), "blue": (0, 0, 255)}[color]
    row = b"\x00" + bytes(rgb) * size
    raw = row * size

    def chunk(kind, body):
        return (struct.pack(">I", len(body)) + kind + body
                + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF))

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
    return base64.b64encode(png).decode()


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


def summary_rows(body):
    """The rows of a note's `Summary` sections as `core/src/summary.rs`
    reads them: (ticked | None for a plain row, text), plus the headings
    the note used, for the verdict of a note that put its list elsewhere."""
    rows, headings, open_level, fenced = [], [], None, False
    for line in body.splitlines():
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            continue
        if fenced:
            continue
        m = re.match(r" {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*#*[ \t]*$", line)
        if m:
            level, text = len(m.group(1)), (m.group(2) or "").strip()
            headings.append(text)
            if open_level is not None and level <= open_level:
                open_level = None
            if open_level is None and text.lower() == "summary":
                open_level = level
            continue
        if open_level is None or not line.strip():
            continue
        item = re.match(r"\s*(?:[-*+]|\d{1,9}[.)]) (.*)$", line)
        if item:
            box = re.match(r"\[( |x|X)\](?: (.*))?$", item.group(1))
            if box:
                rows.append((box.group(1) != " ", (box.group(2) or "").strip()))
                continue
            rows.append((None, item.group(1).strip()))
        else:
            rows.append((None, line.strip()))
    return rows, headings


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


def tool_inputs(rows):
    """(tool name, its input) for every tool use in a transcript, in order."""
    out = []
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_use":
                out.append((b.get("name"), b.get("input") or {}))
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


def tagged(rows, tag):
    """What a transcript's user rows hold inside `<tag>…</tag>`, in order:
    Claude Code records a `!` line and its output as such rows (T-707)."""
    out = []
    for row in rows:
        if row.get("type") != "user":
            continue
        for t in texts(row):
            head, sep, rest = t.partition(f"<{tag}>")
            if sep and not head.strip():
                out.append(rest.split(f"</{tag}>", 1)[0])
    return out


def shell_lines(rows):
    """The `!` lines that ran in the box, as the transcript records them."""
    return tagged(rows, "bash-input")


def shell_outputs(rows):
    """What those lines printed."""
    return tagged(rows, "bash-stdout")


def replies(rows):
    """The assistant's text replies, in order."""
    out = []
    for row in rows:
        if row.get("type") == "assistant":
            t = "\n".join(texts(row)).strip()
            if t:
                out.append(t)
    return out


def slug(title):
    """`mesimon_core::workspace::slug` for the rig's ASCII titles: the branch
    a worktree ticket is cut on is `msmn/<KEY>-<slug>` (U1 holds it)."""
    out, last_dash = [], True
    for ch in title:
        c = ch.lower() if ch.isascii() and (ch.isalnum() or ch in "._") else "-"
        if c == "-":
            if not last_dash:
                out.append("-")
                last_dash = True
        else:
            out.append(c)
            last_dash = False
    s = "".join(out).strip("-.")
    if len(s) > 32:
        cut = 32
        dash = s[:cut + 1].rfind("-")
        if dash >= 8:
            cut = dash
        s = s[:cut].rstrip("-.")
    return s or "t"


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


# ---- dead rig boards (T-765)
#
# A rig board is keyed on its ticket worktree's path, and its state dir, its
# worktrees and their msmn/ branches outlive that worktree: the main board
# archives the ticket and the rig's board stays, a worktree each test. Every
# run clears the boards whose ticket worktree is gone.

def board_repo(state_dir):
    """The checkout a board's state dir is keyed on, or None: the rig's own
    record of it, else a session's cwd whose proj16 is the dir's name (a board
    laid before the record)."""
    cands = []
    try:
        with open(os.path.join(state_dir, "rig", "parent")) as f:
            cands.append(f.read().strip())
    except OSError:
        pass
    try:
        with open(os.path.join(state_dir, "sessions.json")) as f:
            cands += [s.get("cwd") or "" for s in json.load(f).get("sessions", [])]
    except (OSError, ValueError, AttributeError):
        pass
    name = os.path.basename(state_dir)
    return next((c for c in cands if c and board_paths(c).proj16 == name), None)


def gitdir_of(worktree):
    """Where a worktree's `.git` file points, or None."""
    try:
        with open(os.path.join(worktree, ".git")) as f:
            line = f.read().strip()
    except OSError:
        return None
    return line[len("gitdir: "):] if line.startswith("gitdir: ") else None


def pids_of(pattern):
    out = subprocess.run(["pgrep", "-f", "--", pattern], capture_output=True, text=True)
    return [int(p) for p in out.stdout.split()]


def stop_pids(pids, what):
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.time() + 10
    while pids and time.time() < deadline:
        pids = [p for p in pids if _alive(p)]
        time.sleep(0.1)
    if pids:
        say(f"    {what} still running after SIGTERM: {pids}")
    return not pids


def _alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def dead_boards(repo):
    """Each board keyed on a ticket worktree of this repository that is gone,
    as (state dir, its checkout, {worktree: branch})."""
    common = git(repo, "rev-parse", "--path-format=absolute", "--git-common-dir")
    root = os.path.dirname(board_paths(repo).state_dir)
    ticket_worktree = re.compile(re.escape(root) + r"/[0-9a-f]{16}/worktrees/[^/]+")
    listed, path = {}, None
    for line in (git(repo, "worktree", "list", "--porcelain") or "").splitlines():
        if line.startswith("worktree "):
            path = line[len("worktree "):]
            listed[path] = None
        elif line.startswith("branch refs/heads/") and path:
            listed[path] = line[len("branch refs/heads/"):]
    out = []
    for name in sorted(os.listdir(root)):
        d = os.path.join(root, name)
        if not re.fullmatch(r"[0-9a-f]{16}", name) or not os.path.isdir(d):
            continue
        parent = board_repo(d)
        if not parent or not ticket_worktree.fullmatch(parent) or os.path.exists(parent):
            continue
        nested = glob.glob(os.path.join(d, "worktrees", "*"))
        foreign = [w for w in nested if (g := gitdir_of(w)) and not g.startswith(common + "/")]
        if foreign:
            say(f"  {name}: {os.path.basename(parent)} is gone, but {len(foreign)} of its "
                "worktrees belong to another repository: left alone")
            continue
        mine = {p: b for p, b in listed.items() if p.startswith(d + "/worktrees/")}
        out.append((d, parent, mine))
    return out


def sweep(repo, dry):
    """Clear every dead rig board: its daemon and private tmux stopped, its
    worktrees removed, their branches deleted, its state and runtime dirs
    gone. `dry` lists them and touches nothing."""
    boards = dead_boards(repo)
    if not boards:
        if dry:
            say("  no dead rig boards")
        return
    say(f"▶ {len(boards)} dead rig board{'s' if len(boards) != 1 else ''}"
        f"{' (--stale: nothing removed)' if dry else ', cleared'}")
    for d, parent, mine in boards:
        say(f"  {os.path.basename(d)}: {os.path.basename(parent)} is gone; "
            f"{len(mine)} worktree{'s' if len(mine) != 1 else ''}")
        if dry:
            continue
        paths = board_paths(parent)
        stop_pids(pids_of(f"daemon --repo {parent.replace('.', '[.]')}$"), "its daemon")
        stop_pids(pids_of(f"-S {paths.tmux_sock.replace('.', '[.]')} "), "its tmux server")
        for w in mine:
            subprocess.run(["git", "-C", repo, "worktree", "remove", "-f", "-f", w],
                           capture_output=True)
        git(repo, "worktree", "prune")
        branches = [b for b in mine.values() if b]
        if branches:
            git(repo, "branch", "-D", *branches)
        shutil.rmtree(d, ignore_errors=True)
        shutil.rmtree(paths.rt_dir, ignore_errors=True)
        if os.path.exists(d):
            say(f"    {d} could not be removed whole")


class Phone:
    """The phone and the daemon's wait, stood in for at the socket the phone
    mod's `mesimon approve` dials (T-581): each request (a RemotePermission
    header line and the PermissionRequest payload) is held `PHONE_HOLD`
    seconds and answered as the daemon answers it, `"allow"` or `"deny"`
    (`mesophon::PermissionDecision`), then the stream closes. Every request is
    logged with the session and the answer."""

    def __init__(self, path):
        self.answer = "allow"
        self.log = []
        self.path = path
        # The path is this board's own (its runtime dir), so nothing here
        # removes a socket another run owns; a leftover is this board's.
        if os.path.exists(path):
            os.remove(path)
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.bind(path)
        os.chmod(path, 0o600)
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


PEER_NAME = "rig-peer"
# A shell that stands in for Claude on the peer board: it paints Claude's
# composer (the daemon pastes only into one) and keeps every line pasted.
PEER_STUB = (
    "#!/bin/sh\n"
    "printf '\\033[999;1H\\033[3A────────────────────\\n❯ \\n────────────────────\\n"
    "  ? for shortcuts\\033[H'\n"
    "stty -icanon 2>/dev/null\n"
    "while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n"
)


class Peer:
    """A second board on this machine (T-733, X1): `target/rig/rig-peer`, a
    repository of its own with its own daemon from this build, a stub Claude
    as its crown, linked with the rig's board through the machine's
    `links.toml` as a person links one. Its words to the rig's crown are the
    stub's own `ask_board`. Stopped and unlinked when the test ends, so the
    machine's links file is left as it was."""

    def __init__(self, rig):
        self.rig = rig
        self.repo = os.path.join(rig.out, PEER_NAME)
        os.makedirs(self.repo, exist_ok=True)
        if not os.path.isdir(os.path.join(self.repo, ".git")):
            git(self.repo, "init", "-q")
            with open(os.path.join(self.repo, "README.md"), "w") as f:
                f.write("the rig's peer board\n")
            git(self.repo, "add", "README.md")
            git(self.repo, "-c", "user.name=rig", "-c", "user.email=rig@mesimon.dev",
                "commit", "-qm", "the peer board")
        self.paths = board_paths(self.repo)
        self.stub = os.path.join(rig.out, "peer-claude.sh")
        with open(self.stub, "w") as f:
            f.write(PEER_STUB)
        os.chmod(self.stub, 0o700)
        self.got = os.path.join(rig.out, "got.txt")
        if os.path.exists(self.got):
            os.remove(self.got)
        os.makedirs(self.paths.state_dir, mode=0o700, exist_ok=True)
        log = open(self.paths.daemon_log, "ab")
        env = terminal_env({"MESIMON_DETACHED": "1", "MESIMON_CLAUDE_BIN": self.stub,
                            "MESIMON_CLAUDE_ROAD": "hooks", **rig.seams})
        subprocess.Popen([rig.bin, "daemon", "--repo", self.repo], stdin=subprocess.DEVNULL,
                         stdout=log, stderr=log, env=env, start_new_session=True, cwd=self.repo)
        log.close()
        self.wire = Wire(self.paths.orch_sock, client="rig-peer", connect_for=30)
        self.feed_mark = len(feed_all(self.paths))
        self.ticket = self.wire.create_ticket("TODO", "Relay release")
        self.wire.set_workspace(self.ticket, "shared_checkout")
        self.session = self.wire.spawn(self.ticket, "claude", submit_prompt=False)["id"]
        self.wire.crown(self.ticket)
        self.crown_key = next(t["short_key"] for t in self.wire.board()["tickets"]
                              if t["id"] == self.ticket)
        try:
            rig.wire.link_board(self.repo)
        except WireError as e:
            # A run that stopped mid-test left the pair in the file.
            if "linked already" not in str(e):
                raise

    def fed(self):
        """The peer board's feed since it was laid: the commands."""
        return [l.get("cmd") for l in feed_all(self.paths)[self.feed_mark:]
                if l.get("kind") == "board"]

    def reply(self, board, text):
        """The peer's crown speaks to the rig's crown: `ask_board`, an ask
        or the answer to one."""
        return self.wire.ask_board(self.session, board, text)

    def stop(self):
        """Unlink, then stop: the machine's links file is left as it was."""
        try:
            self.rig.wire.unlink_board(self.repo)
        except (WireError, OSError, ConnectionError):
            pass
        self.stop_daemon()

    def stop_daemon(self):
        """The peer's daemon and tmux go; the link stays (X3: a linked
        board that is not open)."""
        try:
            self.wire.shutdown()
        except (WireError, OSError, ConnectionError):
            pass
        deadline = time.time() + 15
        while os.path.exists(self.paths.orch_sock) and time.time() < deadline:
            time.sleep(0.1)
        tmux = os.environ.get("MESIMON_TMUX_BIN") or (
            os.path.join(self.rig.repo, "target/debug/mesimon-tmux")
            if os.path.exists(os.path.join(self.rig.repo, "target/debug/mesimon-tmux")) else "tmux")
        if os.path.exists(self.paths.tmux_sock):
            subprocess.run([tmux, "-S", self.paths.tmux_sock, "kill-server"],
                           capture_output=True, text=True)


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
        # The second board X1 lays (T-733), while it stands, and its feed after.
        self.peer = None
        self.peer_fed = []

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

    def phone_sock(self):
        """The stand-in phone's socket: in this board's runtime dir, beside
        hook.sock, so it fits sun_path as they do and two runs never share it."""
        return os.path.join(self.paths.rt_dir, "phone.sock")

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
        line = "const argv = [c.bin, 'approve', '--sock', c.hookSock, '--session', c.session,"
        if line not in text:
            die("the mod's approve moved: the phone cannot be patched in")
        text = text.replace(line, line.replace("c.hookSock", repr(self.phone_sock())), 1)
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
        alt = next((t for t in board.get("tiers", []) if t["name"] == ALT_TIER_NAME), None)
        alt_spec = {**spec, "id": alt["id"] if alt else ulid(), "name": ALT_TIER_NAME,
                    "description": ALT_TIER_DESCRIPTION}
        if alt != alt_spec:
            self.wire.save_tier("board", alt_spec)
        self.alt_tier = alt_spec
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
        # REVIEW's pipeline gate is H1's alone (T-749): a run cut short
        # inside H1 left it on, and every later test's turn end would wait
        # on a pipeline its ticket never runs.
        self.gate_pipeline(False)
        # The crown's asks go straight to the workers it started and it
        # answers their questions (T-610's autonomous mode, the default);
        # supervised, each waits on its card for a person's ^y.
        if board.get("crown_mode", "autonomous") != "autonomous":
            self.wire.set_crown_mode("autonomous")
        # The crown may watch a ticket a person started (T-712, W1); a board
        # from before T-732 wrote it off.
        if not board.get("crown_watches", True):
            self.wire.set_crown_watches(True)
        say(f"  tier {TIER_NAME} ({spec['model']}, effort {TIER_EFFORT}) is the board's default; "
            f"the crown is autonomous")

    def gate_pipeline(self, on):
        """REVIEW's `entry needs a passed pipeline` (T-739), on or off."""
        col = next(c for c in self.board()["columns"] if c["name"] == "REVIEW")
        if bool(col.get("requires_pipeline")) == on:
            return
        settings = {k: v for k, v in col.items() if k not in ("name", "order")}
        settings["requires_pipeline"] = on
        self.wire.request({"cmd": "set_column_settings", "name": "REVIEW", "settings": settings})

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
                # `true` is P1's 10 KB; a number is that many bytes (B1), and
                # `tail` is the paragraph that ends it (T-896: the one fact a
                # cut brief loses).
                size = test["long_brief"]
                size = size if type(size) is int else 10_000
                test["brief"] = test["brief"] + "\n" + filler(size)
                if test.get("tail"):
                    test["brief"] += "\n\n" + test["tail"]
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
        # `{stub}` is the stub forge's folder (T-859): a pane's PATH is the
        # person's, so a brief that wants the stub `gh` names its path.
        test["brief"] = test["brief"].replace("{stub}", self.forge_stub())
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
        # `{crown}` is the rig's crown's own key (T-908: a note there is
        # another ticket's to a worker).
        keys = {"key": test["key"], "crown": self.crown_key}
        for i, (_, k, _) in enumerate(test.get("others", []), start=2):
            keys[f"key{i}"] = k
        return keys

    def test_card(self, test):
        lines = [f"# {test['id']}: what the rig checks", "", f"**Expected.** {test['expect']}", ""]
        lines.append("**Steps.**")
        for step in test["steps"]:
            if "crown" in step:
                said = step["crown"].format(key="this ticket", key2="the second", key3="the third",
                                            crown="the crown's ticket")
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
            self.wire.save_tier("board", {**self.alt_tier, "model": TIER_MODEL_FULL})
            self.alt_tier["model"] = TIER_MODEL_FULL
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
            if until.get("crown_fed"):
                # A step with no worker (X1): a board line of the crown's
                # own ticket since the step began, `cmd` as named.
                return any(l.get("kind") == "board" and l.get("cmd") == until["crown_fed"]
                           and l.get("ticket") == self.crown for l in self.feed_lines[mark:])
            if until.get("woke"):
                # A start that makes no agent (U1): the board's wake for it.
                return any(l.get("kind") == "crown_wake" and l.get("worker") == tid
                           and l.get("cause") == until["woke"] for l in self.feed_lines[mark:])
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
        final_at = max((l["at_ms"] for rec in recs if rec for l in self.worker_lines(rec["id"], mark)
                        if l.get("kind") == "session_state"), default=0)
        wake = None
        if step.get("wake"):
            wake = self.wait_for("the board's wake for the crown", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") in tickets and l.get("at_ms", 0) >= final_at - 500),
                None), WAKE_GRACE, self.watch)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if wake and until.get("woke"):
                # No worker turn paced this wake (U1): it can land inside the
                # crown's turn on the step's words, and the crown's turn on it
                # starts when it is sent.
                sent = self.wait_for("the wake's delivery", lambda: next((
                    l for l in self.feed_lines[mark:] if l.get("kind") == "board"
                    and l.get("cmd") == "crown_wake_sent"
                    and l.get("at_ms", 0) >= wake["at_ms"]), None), WAKE_GRACE, self.watch)
                wake = sent or wake
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
        if what == "shared_shell":
            # A human-run terminal, shared explicitly. The agent only gets its read tool.
            try:
                self.wire.request({"cmd": "open_terminal", "ticket": test["ticket"]})
                self.wire.request({"cmd": "terminal_end"})
                opened = self.wire.request({"cmd": "adopt_terminal", "ticket": test["ticket"]})
                shell = opened["id"]
                self.wire.request({"cmd": "share_shell", "session": shell, "shared": True})
                out = self.tmux("send-keys", "-t", sid16(shell), "-l", "printf '%s%s\\n' review- evidence-904")
                if out.returncode:
                    raise RuntimeError(out.stderr)
                self.tmux("send-keys", "-t", sid16(shell), "Enter")
                return True
            except (WireError, KeyError, RuntimeError) as error:
                record["failures"].append(f"shared shell fixture: {error}")
                return False
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
        if what == "person_start":
            # The person starts the test's agent themselves (W1, T-734): the
            # composer's Shift+Enter, in the test's workspace, and the step
            # ends at that agent's first turn's end.
            self.wire.set_workspace(test["ticket"], test["workspace"])
            mark = len(self.feed_lines)
            r = self.wire.spawn(test["ticket"], "claude", submit_prompt=True)
            self.log(f"the person started {test['key']}: {r.get('resp')}")

            def first_turn():
                rec = self.agent_of(self.board(), test["ticket"])
                return rec and word_of(rec["state"]) == "idle" and any(
                    l.get("kind") == "hook" and l.get("event") == "Stop"
                    for l in self.worker_lines(rec["id"], mark))

            if not self.wait_for("the person's agent's first turn", first_turn, STEP_TIMEOUT,
                                 self.watch):
                record["failures"].append("the person's agent never finished its first turn")
                return False
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
        if what == "shell":
            # The person's `!` line to the idle worker (T-876): the ask
            # field's words, sent now. The board types them into the box,
            # which runs them in bash mode; no hook fires for the line, so
            # the step ends on the board's own receipt (`shell_sent`) and
            # the turn Claude Code runs on the output is the next step's.
            mark = len(self.feed_lines)
            try:
                r = self.wire.prompt(test["ticket"], step["text"], queued=False)
            except WireError as e:
                record["failures"].append(f"the person's shell line: {e}")
                return False
            self.log(f"the person's shell line to {test['key']}: {r.get('resp')}")
            sent = self.wait_for("the board to type the shell line", lambda: any(
                l.get("kind") == "board" and l.get("cmd") == "shell_sent"
                and l.get("ticket") == test["ticket"] for l in self.feed_lines[mark:]),
                30, self.watch)
            if not sent:
                record["failures"].append("the board never typed the shell line")
                return False
            return True
        if what == "picture_ask":
            # The person's words with a picture (T-853): a solid PNG of
            # `color` staged as the desk's `^v` stages one, then the words,
            # naming it as a note would, sent now to the idle worker.
            color = step.get("color", "red")
            try:
                upload = self.wire.upload_png(solid_png(color))
                words = step["text"].replace("{link}", f"[Image #1](mesimon-attachment:{upload})")
                r = self.wire.prompt(test["ticket"], words, queued=False, uploads=[upload])
            except WireError as e:
                record["failures"].append(f"the person's pictured prompt: {e}")
                return False
            self.log(f"the person's {color} picture and words to {test['key']}: {r.get('resp')}")
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
            self.phone = self.phone or Phone(self.phone_sock())
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
        if what == "limit":
            # L1 (T-802): the account's usage limit, as Claude Code reports
            # it — a `StopFailure` whose class is `rate_limit` — sent through
            # the real hook binary on both roads (the daemon takes the one
            # the session reports by). No real account is made to hit one.
            rec = self.agent_of(self.board(), test["ticket"])
            mark = len(self.feed_lines)
            body = json.dumps({"error": "rate_limit", "last_assistant_message":
                               "You've reached your Sonnet limit. Run /usage-credits to continue "
                               "or switch models with /model."})
            for road in (["--road", "mod"], []):
                subprocess.run([self.bin, "hook", "--sock", os.path.join(self.paths.rt_dir, "hook.sock"),
                                "--session", rec["id"], "--event", "StopFailure",
                                "--reason", "rate_limit", *road],
                               input=body, text=True, capture_output=True, timeout=10)
            wake = self.wait_for("the board's limit wake for the crown", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == test["ticket"]), None), WAKE_GRACE, self.watch)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append("the board never woke the crown for the limit")
                return False
            sent = self.wait_for("the wake's delivery", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "board"
                and l.get("cmd") == "crown_wake_sent" and l.get("at_ms", 0) >= wake["at_ms"]),
                None), WAKE_GRACE, self.watch)
            after = (sent or wake)["at_ms"]
            if not self.wait_for("crown", lambda: self.crown_settled(after_ms=after, quiet=0),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle")
                return False
            self.closed()
            return True
        if what == "peer_board":
            # X1 (T-733): a second board on this machine, crowned by a stub,
            # linked with this one as a person links it.
            self.peer = Peer(self)
            self.log(f"the peer board {PEER_NAME} is up, crowned by {self.peer.crown_key}, "
                     f"and linked with this board")
            return True
        if what == "peer_ask":
            # X2: the peer's crown asks this board's crown, with nothing
            # said to it by the rig: the words reach it through the Board
            # ask template alone, and the step ends when its turn on them
            # ended.
            sent_ms = int(time.time() * 1000)
            words = step.get("text", "which release is tagged on your board? answer with the "
                                     "version, or that you cannot tell, and end your turn")
            try:
                r = self.peer.reply(os.path.basename(self.repo), words)
                self.log(f"the peer's crown asked: {r.get('road')}")
            except WireError as e:
                record["failures"].append(f"the peer's ask: {e}")
                return False
            if not self.wait_for("crown", lambda: self.crown_settled(after_ms=sent_ms, quiet=0),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle on the peer's ask")
                return False
            self.peer_fed = self.peer.fed()
            self.closed()
            return True
        if what == "peer_stop":
            # X3: the peer's daemon goes and the link stays, so the next ask
            # finds a linked board that is not open.
            self.peer.stop_daemon()
            self.log(f"the peer board {PEER_NAME}'s daemon stopped; the link stands")
            return True
        if what == "peer_reply":
            # X1: the peer's crown answers with ask_board; the words reach
            # this board's crown as its next prompt, and the step ends when
            # its turn on them ended. Then the peer goes, and the link.
            sent_ms = int(time.time() * 1000)
            words = step.get("text", "relay 0.1.0-beta.2 is live; /version serves it")
            try:
                r = self.peer.reply(os.path.basename(self.repo), words)
                self.log(f"the peer's crown replied: {r.get('road')}, read as a reply: {r.get('reply')}")
            except WireError as e:
                record["failures"].append(f"the peer's reply: {e}")
                self.peer.stop()
                self.peer = None
                return False
            settled = self.wait_for("crown", lambda: self.crown_settled(after_ms=sent_ms, quiet=0),
                                    STEP_TIMEOUT, self.watch)
            self.peer_fed = self.peer.fed()
            self.peer.stop()
            self.peer = None
            if not settled:
                record["failures"].append("the crown did not settle on the peer's reply")
                return False
            self.closed()
            return True
        if what in ("hold_branch", "free_branch"):
            # U1 (T-777): the ticket's own branch made before its start, so
            # the worktree's precheck refuses it; freed when the test ends.
            branch = f"msmn/{test['key']}-{slug(test['title'])}"
            if what == "hold_branch":
                git(self.repo, "branch", branch, "HEAD")
            else:
                git(self.repo, "branch", "-D", branch)
            self.log(f"{'held' if what == 'hold_branch' else 'freed'} {branch}")
            return True
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
        if what in ("gate_pipeline", "gate_off"):
            # H1 (T-749): REVIEW refuses a ticket whose pipeline has not
            # passed, so a turn's end in IN PROGRESS is held there.
            self.gate_pipeline(what == "gate_pipeline")
            return True
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
        if what == "forge_pr":
            # The forge sees the worker's PR (T-739): a stub `gh` first on the
            # daemon's PATH answers for github.com, the PR open on the
            # ticket's own branch with its pipeline running — the baseline
            # the next step's edge is read against.
            branch = next((w["branch"] for w in self.snapshot().get("worktrees", [])
                           if w["ticket"] == test["ticket"]), None)
            if not branch:
                record["failures"].append("the worker has no worktree branch to open a PR from")
                return False
            d = self.forge_stub()
            self.forge_branch = branch
            self.write_pr(d, branch, "OPEN", "running")
            env = {
                "PATH": d + ":" + os.environ.get("PATH", ""),
                "MSMN_GH_PRS": os.path.join(d, "prs.json"),
                "MSMN_GH_CHECKS": os.path.join(d, "checks.json"),
                "MESIMON_FORGE_EVERY_MS": "2000",
            }
            self.wire.set_merge_into("pull_requests")
            if not self.restart(env):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            bound = self.wait_for("the PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 42
                for p in self.snapshot().get("prs", [])), STEP_TIMEOUT, self.watch)
            if not bound:
                record["failures"].append("the stub forge's PR never bound to the ticket")
                return False
            return True
        if what == "forge_merged_elsewhere":
            # The worker's work merged on the forge as PR 42 at a head its
            # branch is not at (T-778): the branch reads ahead of the local
            # base, which the forge never moved, and nothing folds it merged.
            branch = next((w["branch"] for w in self.snapshot().get("worktrees", [])
                           if w["ticket"] == test["ticket"]), None)
            if not branch:
                record["failures"].append("the worker has no worktree branch to open a PR from")
                return False
            d = self.forge_stub()
            self.forge_branch = branch
            self.write_pr(d, branch, "MERGED", "passed", head="c0ffee42" * 5)
            env = {
                "PATH": d + ":" + os.environ.get("PATH", ""),
                "MSMN_GH_PRS": os.path.join(d, "prs.json"),
                "MSMN_GH_CHECKS": os.path.join(d, "checks.json"),
                "MESIMON_FORGE_EVERY_MS": "2000",
            }
            self.wire.set_merge_into("pull_requests")
            if not self.restart(env):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            bound = self.wait_for("the merged PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 42 and p["state"] == "merged"
                for p in self.snapshot().get("prs", [])) and any(
                w["ticket"] == test["ticket"] and not w.get("merged")
                for w in self.snapshot().get("worktrees", [])), STEP_TIMEOUT, self.watch)
            if not bound:
                record["failures"].append("the merged PR never bound, or the card read merged")
                return False
            return True
        if what == "forge_closed":
            # The worker's PR 42 closed unmerged on the forge at its branch's
            # tip (T-797): the branch reads ahead of the local base, and the
            # closed PR is all the forge holds of the work.
            branch = next((w["branch"] for w in self.snapshot().get("worktrees", [])
                           if w["ticket"] == test["ticket"]), None)
            if not branch:
                record["failures"].append("the worker has no worktree branch to open a PR from")
                return False
            d = self.forge_stub()
            self.forge_branch = branch
            self.write_pr(d, branch, "CLOSED", "passed")
            env = {
                "PATH": d + ":" + os.environ.get("PATH", ""),
                "MSMN_GH_PRS": os.path.join(d, "prs.json"),
                "MSMN_GH_CHECKS": os.path.join(d, "checks.json"),
                "MESIMON_FORGE_EVERY_MS": "2000",
            }
            self.wire.set_merge_into("pull_requests")
            if not self.restart(env):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            bound = self.wait_for("the closed PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 42 and p["state"] == "closed"
                for p in self.snapshot().get("prs", [])), STEP_TIMEOUT, self.watch)
            if not bound:
                record["failures"].append("the closed PR never bound to the ticket")
                return False
            return True
        if what == "forge_empty":
            # The stub forge with no PR yet (T-769): the ticket's set is read
            # empty, so a PR that appears later is newly bound, not a
            # baseline.
            branch = next((w["branch"] for w in self.snapshot().get("worktrees", [])
                           if w["ticket"] == test["ticket"]), None)
            if not branch:
                record["failures"].append("the worker has no worktree branch to open a PR from")
                return False
            d = self.forge_stub()
            self.forge_branch = branch
            path = os.path.join(d, "prs.json")
            with open(path + ".tmp", "w") as f:
                f.write("[]")
            os.replace(path + ".tmp", path)
            env = {
                "PATH": d + ":" + os.environ.get("PATH", ""),
                "MSMN_GH_PRS": path,
                "MSMN_GH_CHECKS": os.path.join(d, "checks.json"),
                "MESIMON_FORGE_EVERY_MS": "2000",
            }
            self.wire.set_merge_into("pull_requests")
            if not self.restart(env):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            read = self.wait_for("the forge's first pass", lambda: (
                self.snapshot().get("forge", {}).get("sampled_at_ms", 0) > 0),
                STEP_TIMEOUT, self.watch)
            if not read:
                record["failures"].append("the stub forge was never read")
                return False
            # A second pass, so the empty read is the ticket's own.
            time.sleep(3)
            return True
        if what == "forge_bound_passed":
            # A PR opened and judged between two reads (T-769): bound with
            # its pipeline already passed, which is its first edge.
            d = self.forge_stub()
            mark = len(self.feed_lines)
            self.write_pr(d, self.forge_branch, "OPEN", "passed")
            wake = self.forge_wake(test, mark)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append("the board never woke the crown for the bound PR")
                return False
            self.quiet_since = None
            if not self.wait_for("the crown's turn on the forge's news",
                                 lambda: self.crown_settled(after_ms=wake.get("at_ms", 0)),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's turn on the forge's news never ended")
                return False
            return True
        if what == "forge_fail":
            # The pipeline fails on the forge: the next read sees the edge,
            # the board wakes the crown, and the crown tells the worker.
            d = self.forge_stub()
            mark = len(self.feed_lines)
            self.write_pr(d, self.forge_branch, "OPEN", "failed")
            wake = self.forge_wake(test, mark)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append("the board never woke the crown for the failed pipeline")
                return False
            told = self.wait_for("the crown to tell the worker", lambda: any(
                l.get("kind") == "board" and l.get("actor") == "agent" and l.get("cmd") == "ask_agent"
                and l.get("ticket") == test["ticket"] for l in self.feed_lines[mark:]),
                STEP_TIMEOUT, self.watch)
            if not told:
                record["failures"].append("the crown never told the worker of its failed pipeline")
                return False
            # The worker's turn on those words ends before the next step
            # restarts the daemon under it.
            rec = self.agent_of(self.board(), test["ticket"])
            done = rec and self.wait_for("the worker's turn on the crown's words", lambda: any(
                l.get("kind") == "hook" and l.get("event") == "Stop"
                for l in self.worker_lines(rec["id"], mark)), STEP_TIMEOUT, self.watch)
            if not done:
                record["failures"].append("the worker's turn on the crown's words never ended")
                return False
            return True
        if what == "forge_pass":
            # The pipeline passes on the forge (T-744): the board wakes the
            # crown with the news, and the step ends when the crown's turn
            # on it did; what it did is the checks'.
            d = self.forge_stub()
            mark = len(self.feed_lines)
            self.write_pr(d, self.forge_branch, "OPEN", "passed")
            wake = self.forge_wake(test, mark)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append("the board never woke the crown for the passed pipeline")
                return False
            self.quiet_since = None
            if not self.wait_for("the crown's turn on the forge's news",
                                 lambda: self.crown_settled(after_ms=wake.get("at_ms", 0)),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's turn on the forge's news never ended")
                return False
            return True
        if what == "forge_merge":
            # PR 42 merges on the forge at the branch's tip (T-757): the
            # forge folds it into the flags, which wake the crown with the
            # merge; the step ends when the crown's turn on it did.
            d = self.forge_stub()
            mark = len(self.feed_lines)
            self.write_pr(d, self.forge_branch, "MERGED", "passed")
            wake = self.wait_for("the crown's merge wake", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == test["ticket"]
                and l.get("cause") in ("merged", "delivered")), None),
                WAKE_GRACE * 2, self.watch)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append("the board never woke the crown for the merge")
                return False
            self.quiet_since = None
            if not self.wait_for("the crown's turn on the merge",
                                 lambda: self.crown_settled(after_ms=wake.get("at_ms", 0)),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's turn on the merge never ended")
                return False
            return True
        if what == "forge_later":
            # A second PR on the merged ticket (T-757), at a commit the
            # worktree's branch does not hold: it opens, which is no news,
            # then merges, which moves no flag, so the forge's wake says it.
            d = self.forge_stub()
            later = {"number": 43, "head": "c0ffee43" * 5}
            self.write_pr(d, self.forge_branch, "MERGED", "passed", later=dict(later, state="OPEN"))
            bound = self.wait_for("the second PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 43
                for p in self.snapshot().get("prs", [])), STEP_TIMEOUT, self.watch)
            if not bound:
                record["failures"].append("the second PR never bound to the ticket")
                return False
            mark = len(self.feed_lines)
            self.write_pr(d, self.forge_branch, "MERGED", "passed", later=dict(later, state="MERGED"))
            wake = self.forge_wake(test, mark)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append("the board never woke the crown for the later merge")
                return False
            self.quiet_since = None
            if not self.wait_for("the crown's turn on the forge's news",
                                 lambda: self.crown_settled(after_ms=wake.get("at_ms", 0)),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's turn on the forge's news never ended")
                return False
            return True
        if what in ("forge_conflict", "forge_clear"):
            # The PR turns conflicting, or clears (T-785): the board wakes
            # the crown with the edge, and the step ends when the crown's
            # turn on it did.
            d = self.forge_stub()
            mark = len(self.feed_lines)
            self.write_pr(d, self.forge_branch, "OPEN", "running", conflict=what == "forge_conflict")
            wake = self.forge_wake(test, mark)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append(f"the board never woke the crown for {what}")
                return False
            self.quiet_since = None
            if not self.wait_for("the crown's turn on the forge's news",
                                 lambda: self.crown_settled(after_ms=wake.get("at_ms", 0)),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's turn on the forge's news never ended")
                return False
            return True
        if what == "forge_restart":
            # The daemon restarts on the same stub forge (T-785), once
            # forge.json holds the conflict said, and the step ends at the
            # first live read after it, which says nothing.
            d = self.forge_stub()
            kept = os.path.join(self.paths.state_dir, "forge.json")
            def said():
                try:
                    with open(kept) as f:
                        return '"conflicts"' in f.read()
                except OSError:
                    return False
            if not self.wait_for("forge.json to keep the conflict said", said,
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("forge.json never kept the conflict said")
                return False
            self.wire.set_merge_into("pull_requests")
            if not self.restart(self.forge_env(d)):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            read = self.wait_for("the first live read", lambda: (
                self.snapshot().get("forge", {}).get("sampled_at_ms", 0) > 0),
                STEP_TIMEOUT, self.watch)
            if not read:
                record["failures"].append("the stub forge was never read after the restart")
                return False
            time.sleep(3)
            return True
        if what == "forge_later_open":
            # A second PR opens on the merged ticket (T-774) and stays open:
            # the ticket reads unmerged again, and DONE refuses it.
            d = self.forge_stub()
            later = {"number": 43, "head": "c0ffee43" * 5, "state": "OPEN"}
            self.write_pr(d, self.forge_branch, "MERGED", "passed", later=later)
            held = self.wait_for("the second PR to bind and unmerge the card", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 43
                for p in self.snapshot().get("prs", [])) and not any(
                w["ticket"] == test["ticket"] and w.get("merged")
                for w in self.snapshot().get("worktrees", [])), STEP_TIMEOUT, self.watch)
            if not held:
                record["failures"].append("the second PR never bound, or the card stayed merged")
                return False
            return True
        if what == "prs_board":
            # The board's branches land through pull requests (T-810), on
            # the stub forge with no PR: what the board says of a branch
            # nobody has opened one from yet.
            d = self.forge_stub()
            path = os.path.join(d, "prs.json")
            with open(path + ".tmp", "w") as f:
                f.write("[]")
            os.replace(path + ".tmp", path)
            self.wire.set_merge_into("pull_requests")
            if not self.restart(self.forge_env(d)):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            return True
        if what == "forge_reviewed":
            # The reviewed PR 43 binds to the worker's ticket (T-859), read
            # off the stub by itself, before the stub goes: the ledger's
            # pass and the forge's each take a bucket.
            bound = self.wait_for("the reviewed PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 43 and p.get("reviewed")
                for p in self.snapshot().get("prs", [])), STEP_TIMEOUT, self.watch)
            if not bound:
                record["failures"].append("the stub forge's PR 43 never bound reviewed to the ticket")
                return False
            return True
        if what == "forge_lab":
            # A stub forge up before the worker starts (T-859): someone
            # else's PR 43 is served by itself alone (`pr view`), never in
            # the account's list, so only a review or a Summary line binds
            # it. The worker reaches the stub by its path: a pane's PATH is
            # the person's login shell's, never the daemon's.
            d = self.forge_stub()
            with open(os.path.join(d, "prs.json"), "w") as f:
                f.write("[]")
            with open(os.path.join(d, "view.43.json"), "w") as f:
                f.write('{"number":43,"url":"https://github.com/example/rig/pull/43",'
                        '"state":"OPEN","isDraft":false,"headRefName":"feature/other",'
                        '"headRefOid":"' + "ab" * 20 + '","mergeCommit":null,"mergeable":"MERGEABLE",'
                        '"reviewDecision":"REVIEW_REQUIRED","updatedAt":"2026-10-19T06:00:00Z",'
                        '"statusCheckRollup":[{"__typename":"CheckRun","name":"test-linux",'
                        '"status":"COMPLETED","conclusion":"SUCCESS"}]}')
            self.wire.set_merge_into("pull_requests")
            if not self.restart(self.forge_env(d)):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            return True
        if what == "forge_off":
            # The board's own Claude Code and PATH back: the stub is gone,
            # and the board merges into main again (T-810).
            self.wire.set_merge_into("main")
            d = self.forge_stub()
            for name in ("prs.json", "checks.json"):
                try:
                    os.remove(os.path.join(d, name))
                except FileNotFoundError:
                    pass
            return bool(self.restart())
        die(f"unknown rig step {what}")

    def forge_wake(self, test, mark):
        """The board's forge wake for the test's worker since `mark`."""
        return self.wait_for("the crown's forge wake", lambda: next((
            l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
            and l.get("worker") == test["ticket"] and l.get("cause") == "forge"), None),
            WAKE_GRACE * 2, self.watch)

    def forge_env(self, d):
        """The daemon's environment on the stub forge in `d`."""
        return {
            "PATH": d + ":" + os.environ.get("PATH", ""),
            "MSMN_GH_PRS": os.path.join(d, "prs.json"),
            "MSMN_GH_CHECKS": os.path.join(d, "checks.json"),
            "MESIMON_FORGE_EVERY_MS": "2000",
        }

    def forge_stub(self):
        """A stub `gh` (T-739): `auth status` passes, `pr list` prints the
        file the test writes, `api …/check-runs` the merge commit's pipeline.
        Under the state dir, where nothing of the person's is."""
        d = os.path.join(self.paths.state_dir, "forge-stub")
        os.makedirs(d, mode=0o700, exist_ok=True)
        gh = os.path.join(d, "gh")
        if not os.path.exists(gh):
            with open(gh, "w") as f:
                f.write(GH_STUB)
            os.chmod(gh, 0o755)
        checks = os.path.join(d, "checks.json")
        if not os.path.exists(checks):
            with open(checks, "w") as f:
                f.write('{"check_runs":[]}')
        return d

    def write_pr(self, d, branch, state, pipeline, later=None, head=None, conflict=False):
        """PR 42 on `branch` at its tip, or at `head` (T-778), in conflict
        where `conflict` (T-785); `later` (number, head, state) a newer PR
        on the same branch in front of it (T-757)."""
        tip = head or subprocess.run(["git", "-C", self.repo, "rev-parse", branch],
                                     capture_output=True, text=True).stdout.strip()
        rollups = {
            "running": '[{"__typename":"CheckRun","name":"test-linux","status":"IN_PROGRESS","conclusion":null}]',
            "failed": '[{"__typename":"CheckRun","name":"test-linux","status":"COMPLETED","conclusion":"FAILURE"}]',
            "passed": '[{"__typename":"CheckRun","name":"test-linux","status":"COMPLETED","conclusion":"SUCCESS"}]',
        }
        rollup = rollups[pipeline]
        def pr(number, head, state, at):
            merge = f'{{"oid":"{head}"}}' if state == "MERGED" else "null"
            # The later PR opens running: its opening is no pipeline news.
            checks = rollups["running"] if state == "OPEN" and number != 42 else rollup
            mergeable = "CONFLICTING" if conflict and number == 42 else "MERGEABLE"
            return (f'{{"number":{number},"url":"https://github.com/example/rig/pull/{number}",'
                    f'"state":"{state}","isDraft":false,"headRefName":"{branch}",'
                    f'"headRefOid":"{head}","mergeCommit":{merge},"mergeable":"{mergeable}",'
                    f'"reviewDecision":"REVIEW_REQUIRED","updatedAt":"{at}",'
                    f'"statusCheckRollup":{checks}}}')
        prs = [pr(42, tip, state, "2026-10-13T06:00:00Z")]
        if later:
            prs.insert(0, pr(later["number"], later["head"], later["state"], "2026-10-13T07:00:00Z"))
        body = "[" + ",".join(prs) + "]"
        path = os.path.join(d, "prs.json")
        with open(path + ".tmp", "w") as f:
            f.write(body)
        os.replace(path + ".tmp", path)

    def run_test(self, test):
        self.current = test
        self.words.pop(test["key"], None)
        self.history.pop(test["key"], None)
        start_mark = len(self.feed_lines)
        t0 = time.time()
        say(f"\n▶ {test['id']} {test['key']}: {test['title']}")
        record = {"failures": [], "pings": [], "doctor": None, "wakes": [],
                  "t0_ms": int(t0 * 1000)}
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
            # A start that made no agent (U1) is judged on the board's and
            # the crown's side alone.
            if rec is None and name not in ("woke", "crown_hand", "crown_reply", "crown_line",
                                            "crown", "crown_board", "peer_fed"):
                check(c, False, "no agent record on the ticket")
            elif name == "road":
                check(c, rec.get("road", "hooks") == arg, f"road {rec.get('road', 'hooks')}")
            elif name == "plugin_dir":
                check(c, "--plugin-dir" in rec["argv"], "--plugin-dir in argv"
                      if "--plugin-dir" in rec["argv"] else "no --plugin-dir in argv")
            elif name == "no_plugin_dir":
                check(c, "--plugin-dir" not in rec["argv"], "no --plugin-dir in argv"
                      if "--plugin-dir" not in rec["argv"] else "--plugin-dir in argv")
            elif name == "started_by_person":
                check(c, rec.get("started_by") is None, f"started by {rec.get('started_by')}")
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
                # `crown:<cmd>=N` (T-880): exactly N of them, not at least one.
                cmd, _, times = arg.partition("=")
                n = crown_cmds.count(cmd)
                check(c, n == int(times) if times else n > 0, f"{cmd}×{n}")
            elif name == "crown_board":
                # A board line of the crown's own ticket (T-733): its ask to
                # a linked board is about no ticket here.
                own = [l for l in lines if l.get("kind") == "board" and l.get("actor") == "agent"
                       and l.get("ticket") == self.crown and l.get("cmd") == arg]
                check(c, own, f"{arg}×{len(own)} ({own[-1].get('outcome')})" if own else f"no {arg}")
            elif name == "peer_fed":
                # The peer board's feed (X1): what its daemon did with the words.
                fed_peer = self.peer.fed() if self.peer else self.peer_fed
                check(c, arg in fed_peer, f"{arg}×{fed_peer.count(arg)}")
            elif name == "crown_hand":
                # The crown's own hand (T-744), its reason naming `arg`: a
                # hand is on the crown's ticket, so the feed's line is not
                # this one's; the crown's transcript says what it wrote.
                crown = self.agent_of(board, self.crown)
                hands = [i.get("reason", "") for n, i in tool_inputs(transcript(crown) if crown else [])
                         if (n or "").endswith("raise_hand")]
                said = [h for h in hands if arg in h]
                check(c, said, repr(said[-1][:100]) if said
                      else f"{len(hands)} hands, none naming {arg}: {[h[:60] for h in hands[-2:]]}")
            elif name == "hand":
                # The worker's own hand (T-908), its reason naming `arg`,
                # case aside: what the worker wrote, from its transcript.
                hands = [i.get("reason", "") for n, i in tool_inputs(rows)
                         if (n or "").endswith("raise_hand")]
                said = [h for h in hands if arg.lower() in h.lower()]
                check(c, said, repr(said[-1][:100]) if said
                      else f"{len(hands)} hands, none naming {arg}: {[h[:60] for h in hands[-2:]]}")
            elif name == "crown_never":
                # T-895: no Bash call of the crown's since the test began
                # holds any of the words (a `gh pr checks --watch`, a
                # `sleep` loop), and no Monitor stood in for one: the
                # board's wake is the wait, and a crown that polls holds the
                # merge train and that wake.
                crown = self.agent_of(board, self.crown)
                since = record.get("t0_ms", 0)
                crown_rows = [r for r in (transcript(crown) if crown else [])
                              if (iso_ms(r.get("timestamp") or "") or 0) >= since]
                words = [w for w in arg.split(",") if w]
                calls = tool_inputs(crown_rows)
                ran = [i.get("command", "") for n, i in calls if n == "Bash"]
                bad = [r for r in ran if any(w in r for w in words)]
                bad += [f"<{n}>" for n, _ in calls if n == "Monitor"]
                check(c, not bad, " → ".join(repr(b[:60]) for b in bad)
                      or f"{len(ran)} bash calls, none of {words}")
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
            elif name == "summary":
                # T-710: the worker's own notes (not the rig's two), read
                # off the board's disk as the card reads them.
                # T-743: `+<n>` also wants that many plain rows.
                want, _, want_plain = arg.partition("+")
                ticket = self.ticket(board, tid)
                notes = [n for n in ticket.get("notes", []) if str(n.get("created_by", "")).startswith("agent:")]
                found, used = None, []
                for n in notes:
                    path = os.path.join(self.repo, ".mesimon", "board", "tickets", key, "notes", f"{n['id']}.md")
                    try:
                        with open(path, encoding="utf-8") as f:
                            got, heads = summary_rows(f.read())
                    except OSError:
                        continue
                    used += heads
                    if got:
                        found = got
                        break
                if found is None:
                    check(c, False, f"no Summary section in {len(notes)} agent note(s); headings {used!r}")
                else:
                    boxes = [d for d, _ in found if d is not None]
                    plain = sum(1 for d, _ in found if d is None)
                    have = f"{sum(boxes)}/{len(boxes)}"
                    ok = have == want and (not want_plain or plain == int(want_plain))
                    check(c, ok, f"{have} boxes, {plain} plain row(s): "
                          + "; ".join(f"[{'x' if d else ' '}] {t[:24]}" if d is not None else f"- {t[:24]}" for d, t in found))
            elif name == "boxes_free":
                # T-727: no box of the worker's own Summary names what the
                # board does by itself. Boxes alone: a plain row may say
                # who merges.
                words = [w.strip().lower() for w in arg.split(",") if w.strip()]
                ticket = self.ticket(board, tid)
                notes = [n for n in ticket.get("notes", []) if str(n.get("created_by", "")).startswith("agent:")]
                boxes, hits = [], []
                for n in notes:
                    path = os.path.join(self.repo, ".mesimon", "board", "tickets", key, "notes", f"{n['id']}.md")
                    try:
                        with open(path, encoding="utf-8") as f:
                            got, _ = summary_rows(f.read())
                    except OSError:
                        continue
                    for done, text in got:
                        if done is None:
                            continue
                        boxes.append(text)
                        if any(w in text.lower() for w in words):
                            hits.append(text)
                check(c, boxes and not hits, "; ".join(f"[{t[:30]}]" for t in hits) if hits
                      else (f"{len(boxes)} box(es), none names " + ", ".join(words) if boxes else "no box"))
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
            elif name == "no_own_key":
                said = [r for r in replies(rows) if key in r]
                check(c, not said, f"{key} in {said[0][:80]!r}" if said
                      else f"{key} in none of {len(replies(rows))} replies")
            elif name == "reply":
                said = last_reply(rows)
                check(c, arg.lower() in said.lower(), f"said {said[:60]!r}")
            elif name == "shell_workflow":
                said = last_reply(rows)
                try:
                    answer, _ = json.JSONDecoder().raw_decode(said[said.index("{"):])
                except (ValueError, TypeError):
                    answer = {}
                expected = {"evidence": "review-evidence-904", "capture_requires_send": True,
                            "escape_discards": True, "escape_adopts_terminal": False, "capture_grants_live_reads": False,
                            "capture_starts_agent": False, "live_reads_available_in_tui": False}
                ok = all(type(answer.get(k)) is type(v) and answer.get(k) == v for k, v in expected.items())
                ran = [n for n, _ in tool_inputs(rows) if n in ("Bash", "Write", "Edit", "Agent")]
                check(c, ok and not ran, f"workflow {answer}; execution tools {ran}")
            elif name == "shell_close_pending":
                shells = [s for s in board.get("sessions", []) if s.get("ticket") == tid and s.get("kind") == "bash"]
                pending = [s for s in shells if s.get("shell_close_request")]
                alive = pending and all(self.tmux("display-message", "-p", "-t", sid16(s["id"]), "#{pane_dead}").stdout.strip() == "0" for s in pending)
                crown = self.agent_of(board, self.crown)
                since = record.get("t0_ms", 0)
                crown_rows = [r for r in (transcript(crown) if crown else []) if (iso_ms(r.get("timestamp") or "") or 0) >= since]
                calls = tool_calls(crown_rows)
                read = any(n == "mcp__mesimon__read_shell" and result and not result.get("is_error") for n, result in calls)
                ran = [n for n, _ in tool_inputs(crown_rows) if n in ("Bash", "Write", "Edit", "Agent")]
                check(c, alive and read and not ran, f"{len(pending)} live shells await confirmation; crown read={read}, execution tools={ran}")
            elif name == "shell":
                # A `!` line ran in the worker's bash mode (T-876): its
                # transcript holds the line as Claude Code records one
                # (`<bash-input>`), never as a prompt the model was sent.
                ran = shell_lines(rows)
                check(c, arg in ran, " → ".join(repr(r[:40]) for r in ran) or "no shell line ran")
            elif name == "shell_out":
                outs = shell_outputs(rows)
                check(c, any(arg in o for o in outs),
                      " → ".join(repr(o[:40]) for o in outs) or "no shell output")
            elif name == "no_shell":
                ran = shell_lines(rows)
                check(c, not ran, " → ".join(repr(r[:40]) for r in ran) or "no shell line ran")
            elif name == "doctor":
                check(c, arg in (record["doctor"] or ""), record["doctor"] or "no line")
            elif name == "final":
                check(c, word_of(rec["state"]) == arg, word_of(rec["state"]))
            elif name == "tier":
                # The tier the ticket's agent last launched on, by name (L2).
                on = next((t["name"] for t in board.get("tiers", [])
                           if t["id"] == rec.get("tier")), rec.get("tier") or "none")
                check(c, on == arg, f"on {on}")
            elif name == "column" or name == "not_column":
                # Where the ticket stands at the verdict (T-774).
                col = next((t.get("column") for t in board.get("tickets", [])
                            if t.get("id") == tid), None)
                check(c, (col == arg) == (name == "column"), f"in {col}")
            elif name == "unlanded":
                # The ticket's branch is not on the checkout's base (T-778):
                # nothing landed it here.
                branch = next((w["branch"] for w in self.snapshot().get("worktrees", [])
                               if w["ticket"] == tid), None)
                landed = bool(branch) and subprocess.run(
                    ["git", "-C", self.repo, "merge-base", "--is-ancestor", branch, "HEAD"],
                    capture_output=True).returncode == 0
                check(c, branch and not landed, f"{branch} landed on the base" if landed
                      else f"{branch} not on the base" if branch else "no branch")
            elif name == "crown_reply":
                # The crown's last words on the test (T-774), `arg` within.
                crown = self.agent_of(board, self.crown)
                said = last_reply(transcript(crown)) if crown else ""
                check(c, arg.lower() in said.lower(), f"said {said[:100]!r}")
            elif name == "crown_line":
                # The crown's last line beginning `prefix` (T-810), `word`
                # within it, or with `!word` not: one test asks the crown
                # twice and reads each answer by its own line.
                # The line runs to the end of its reply: a crown may put
                # the prefix on a line of its own and its answer under it.
                prefix, _, word = arg.rpartition(":")
                crown = self.agent_of(board, self.crown)
                line = ""
                for r in reversed(replies(transcript(crown)) if crown else []):
                    said = [l.strip().strip("*`").strip() for l in r.splitlines()]
                    at = next((i for i in reversed(range(len(said)))
                               if said[i].startswith(prefix)), None)
                    if at is not None:
                        line = " ".join(l for l in said[at:] if l)
                        break
                if word.startswith("!"):
                    ok = bool(line) and word[1:].lower() not in line.lower()
                else:
                    ok = word.lower() in line.lower()
                check(c, ok, f"said {line[:120]!r}" if line else f"no line {prefix!r}")
            elif name == "woke":
                causes = [l.get("cause") for l in lines if l.get("kind") == "crown_wake"
                          and l.get("worker") == tid]
                check(c, arg in causes, f"wakes {causes}" if causes else "no wake")
            elif name == "reviewed":
                # T-859: the feed says PR <arg> bound to the ticket as
                # someone else's, reviewed here. The feed, not the snapshot:
                # `forge_off` has put the board back on main by now.
                said = [str(l.get("outcome", "")) for l in lines if l.get("kind") == "board"
                        and l.get("actor") == "forge" and l.get("cmd") == "pr"
                        and l.get("ticket") == tid]
                check(c, any(f"#{arg} bound, reviewed here" in s for s in said),
                      "; ".join(said) or "the forge said nothing of the ticket")
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
        if self.peer:
            self.peer_fed = self.peer.fed()
            self.peer.stop()
            self.peer = None
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
            # The peer board a test laid (T-733) goes with the test, and the
            # link with it: the next test lays its own.
            if self.peer:
                self.peer_fed = self.peer.fed()
                self.peer.stop()
                self.peer = None
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
    ap.add_argument("--provider", choices=("claude", "codex"), default="claude")
    from codex import add_arguments, run as run_codex
    add_arguments(ap)
    ap.add_argument("--lay", action="store_true", help="lay the Claude board or list Codex cases; start nothing")
    ap.add_argument("--only", help="test ids: Claude R1,R3 (or a group letter); Codex mcp,sleep-resume")
    ap.add_argument("--failed", action="store_true",
                    help="rerun unsuccessful tests from the provider's last verdicts")
    ap.add_argument("--reset", action="store_true", help="park and archive everything, stop the daemon and tmux")
    ap.add_argument("--stale", action="store_true",
                    help="list the dead rig boards every run clears; remove nothing")
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
    if args.provider == "codex":
        sys.exit(run_codex(repo, args, ap))
    if args.live or args.suite != "smoke" or any(value is not None for value in
            (args.model, args.effort, args.max_turns, args.max_tokens, args.case_timeout)):
        ap.error("model, budget, --live and --suite options are for the Codex rig")
    rig = Rig(repo, args)
    rig.branch = branch
    os.makedirs(rig.out, exist_ok=True)
    LOG = open(os.path.join(rig.out, "rig.log"), "a")
    say(f"\n=== rig {time.strftime('%Y-%m-%d %H:%M:%S')} ∙ {branch} ∙ proj16 {rig.paths.proj16}")

    sweep(repo, dry=args.stale)
    if args.stale:
        return
    # The checkout this board is keyed on, for a later run's sweep to find
    # once that checkout is gone (T-765).
    os.makedirs(os.path.join(rig.paths.state_dir, "rig"), mode=0o700, exist_ok=True)
    with open(os.path.join(rig.paths.state_dir, "rig", "parent"), "w") as f:
        f.write(repo + "\n")

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
