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
import shlex
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
# The workers' model, by its full id: the `sonnet` alias resolved to another
# model on one build, and a tier saved with the alias is rewritten to this.
TIER_MODEL = "claude-sonnet-5-5"
TIER_EFFORT = "low"
# The person's words name one tier alone for TR1's one-line edit (T-930's
# review: "every rig test" fitted it as well as rig-small did).
TIER_DESCRIPTION = "rig tests that are not a one-line edit"
# The crown's own tier (T-930): crown text is judged on the crown's real
# tier, Opus, never on the workers' Sonnet. `--crown-model` and
# `--crown-effort` change it.
CROWN_TIER_NAME = "rig-crown"
CROWN_MODEL = "claude-opus-5-5"
CROWN_EFFORT = "high"
CROWN_TIER_DESCRIPTION = "the rig's crown alone, never a worker"
# TR1's tier (T-930): the person's words on when to use it, for a crown that
# reads the tiers when it starts a ticket.
SMALL_TIER_NAME = "rig-small"
SMALL_TIER_DESCRIPTION = "a one-line edit to a single file"
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
      */commits/*/status) echo '{"state":"pending","statuses":[]}' ;;
      *deployments/*/statuses*) echo '[{"state":"success"}]' ;;
      *deployments*) echo '[]' ;;
      *) echo '{}' ;;
    esac ;;
  *) echo "stub gh: $*" >&2; exit 1 ;;
esac
"""
WAKE_GRACE = 60
# The crown's wake row while a client in its pane holds it, at the default
# bound (T-945).
WHEN_YOU_LEAVE = "leave or 10m idle"
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


COMPOSER_RULE_MIN = 8


def composer_empty(lines, cursor=None):
    """Claude Code's composer drawn with nothing typed in it (A3, T-945), by
    the shape `agents::claude::composer::read` measured: the bottom-most `❯`
    row with a rule of eight or more `─` directly above it and a rule below
    it, nothing after the glyph and nothing wrapped under it, or a
    placeholder there with the cursor (`(x, y)`, the pane's own) parked at
    the input's start. An old prompt in the scrollback, a dialog's selected
    row and a half-painted box are not one."""
    def rule(line):
        line = line.strip()
        return len(line) >= COMPOSER_RULE_MIN and set(line) == {"─"}

    for row in range(len(lines) - 1, -1, -1):
        body = lines[row].lstrip()
        if not body.startswith("❯"):
            continue
        if row == 0 or not rule(lines[row - 1]):
            continue
        below = next((i for i in range(row + 1, len(lines)) if rule(lines[i])), None)
        if below is None:
            continue
        wrapped = any(l.strip() for l in lines[row + 1:below])
        if wrapped:
            return False
        if not body[1:].strip():
            return True
        start = len(lines[row]) - len(body) + 2
        return cursor == (start, row)
    return False


def load_tests(path):
    """The suite from `tests.toml`. An id names one test: `--only`, the
    failure retry and the verdict columns all find a test by it, so a second
    test under the same id is refused rather than run beside the first."""
    with open(path, "rb") as f:
        tests = tomllib.load(f)["test"]
    seen = set()
    for t in tests:
        if t["id"] in seen:
            raise ValueError(f"{path}: two tests are named {t['id']}")
        seen.add(t["id"])
    return tests


def file_generation(path):
    """Which write of `path` is on disk: its inode and mtime, or None."""
    try:
        st = os.stat(path)
    except OSError:
        return None
    return (st.st_ino, st.st_mtime_ns)


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


def once(rows):
    """`rows` with each tool_use id and each tool_result's tool_use_id kept
    once (T-1034): Claude Code 2.1.310 writes an MCP tool_use block in two
    rows under one id, and its result twice. Other blocks stay as they are."""
    uses, results, out = set(), set(), []
    for row in rows:
        msg = row.get("message") or {}
        content = msg.get("content")
        if not isinstance(content, list):
            out.append(row)
            continue
        kept = []
        for b in content:
            if isinstance(b, dict) and b.get("type") == "tool_use":
                if b.get("id") in uses:
                    continue
                uses.add(b.get("id"))
            elif isinstance(b, dict) and b.get("type") == "tool_result":
                if b.get("tool_use_id") in results:
                    continue
                results.add(b.get("tool_use_id"))
            kept.append(b)
        out.append(row if len(kept) == len(content) else {**row, "message": {**msg, "content": kept}})
    return out


def tool_calls(rows):
    """(tool name, its result) for every tool use in a transcript, in order;
    a result is `{is_error, text}`, or None while none came."""
    rows = once(rows)
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


# The words of `CrownAsk::refusal` (T-938): a second ask refused because the
# first is still out, not for any other reason (a stale seen stamp).
OUTSTANDING_ASK_MARK = "A second ask is refused while the first is out"


def second_ask_refusal(rows, key):
    """(ok, observed) for the crown's first two ask_agent calls on `key`, from
    its own tool results, never its summary (T-938, round 3): the first's
    receipt names its ask, and the second's answer is the outstanding-ask
    refusal naming that same ask."""
    results, asks = {}, []
    for row in once(rows):
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if not isinstance(b, dict):
                continue
            if b.get("type") == "tool_result":
                body = b.get("content")
                results[b.get("tool_use_id")] = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
            elif (b.get("type") == "tool_use"
                  and (b.get("name") or "").rsplit("__", 1)[-1] == "ask_agent"
                  and str((b.get("input") or {}).get("key", "")).strip() == key):
                asks.append(b.get("id"))
    said = [results.get(i) for i in asks]
    if len(said) < 2 or said[0] is None or said[1] is None:
        return False, f"{len(said)} ask_agent call(s) on {key} with results"
    try:
        first = json.loads(said[0])["asked"]["ask"]
    except (ValueError, KeyError, TypeError):
        return False, f"the first receipt names no ask: {said[0][:120]!r}"
    ok = OUTSTANDING_ASK_MARK in said[1] and first in said[1]
    return ok, f"first ask {first}; second answered {said[1][:160]!r}"


# The words of `Daemon::check_seen`'s refusal: a stale stamp, not any other
# refusal.
STALE_SEEN_MARK = "changed since it was read"


def receipt_ask_outcome(rows, key):
    """(ok, observed) for the crown's first ask_agent on `key` after its
    start_agent on it (T-1035), from its own calls and results: the ask
    carries the seen the start's receipt returned, no get_ticket of `key`
    came between the two, and the ask's answer is its receipt (its `asked`
    record), not the stale refusal."""
    rows = once(rows)
    results = {}
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_result":
                body = b.get("content")
                results[b.get("tool_use_id")] = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
    start_seen, reads, ask = None, 0, None
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if not isinstance(b, dict) or b.get("type") != "tool_use":
                continue
            tool = (b.get("name") or "").rsplit("__", 1)[-1]
            args = b.get("input") or {}
            if str(args.get("key", "")).strip() != key:
                continue
            if tool == "start_agent" and start_seen is None:
                try:
                    start_seen = json.loads(results.get(b.get("id")) or "")["seen"]
                except (ValueError, KeyError, TypeError):
                    return False, f"the start's receipt names no seen: {results.get(b.get('id'), '')[:120]!r}"
            elif tool == "get_ticket" and start_seen is not None and ask is None:
                reads += 1
            elif tool == "ask_agent" and start_seen is not None and ask is None:
                ask = (str(args.get("seen", "")).strip(), results.get(b.get("id")))
    if start_seen is None:
        return False, "no start_agent on the ticket with a receipt"
    if ask is None:
        return False, "no ask_agent on the ticket after the start"
    seen, said = ask
    if said is None:
        return False, "the ask has no result"
    if seen != start_seen:
        return False, f"the ask carried {seen!r}, the start's receipt {start_seen!r}; {reads} read(s) between"
    try:
        receipt = json.loads(said)
        taken = isinstance(receipt, dict) and "asked" in receipt and STALE_SEEN_MARK not in said
    except ValueError:
        taken = False
    stale = STALE_SEEN_MARK in said
    return taken and reads == 0, (
        f"the ask carried the start's seen, {reads} read(s) between, "
        f"answered {'stale' if stale else 'taken' if taken else said[:120]!r}")


def create_start_outcome(rows):
    """(ok, observed) for the crown's first create_ticket and the first
    start_agent on the key it returned (T-1072): the start carries the seen
    the create's receipt returned, no get_ticket of that key came between
    the two, and the start's answer is its receipt, not a refusal."""
    rows = once(rows)
    results = {}
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if isinstance(b, dict) and b.get("type") == "tool_result":
                body = b.get("content")
                results[b.get("tool_use_id")] = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
    filed, reads, start = None, 0, None
    for row in rows:
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if not isinstance(b, dict) or b.get("type") != "tool_use":
                continue
            tool = (b.get("name") or "").rsplit("__", 1)[-1]
            args = b.get("input") or {}
            if tool == "create_ticket" and filed is None:
                try:
                    # The first call after a crowning carries the rules after
                    # its receipt (T-930): the receipt is the leading object.
                    receipt = json.JSONDecoder().raw_decode((results.get(b.get("id")) or "").lstrip())[0]
                    filed = (receipt["key"], receipt["seen"])
                except (ValueError, KeyError, TypeError):
                    return False, f"the create's receipt names no key and seen: {results.get(b.get('id'), '')[:120]!r}"
                continue
            if filed is None or start is not None or str(args.get("key", "")).strip() != filed[0]:
                continue
            if tool == "get_ticket":
                reads += 1
            elif tool == "start_agent":
                start = (str(args.get("seen", "")).strip(), results.get(b.get("id")))
    if filed is None:
        return False, "no create_ticket with a receipt"
    if start is None:
        return False, f"no start_agent on {filed[0]} after its create"
    seen, said = start
    if said is None:
        return False, "the start has no result"
    if seen != filed[1]:
        return False, f"the start carried {seen!r}, the create's receipt {filed[1]!r}; {reads} read(s) between"
    try:
        receipt = json.JSONDecoder().raw_decode(said.lstrip())[0]
        taken = isinstance(receipt, dict) and receipt.get("key") == filed[0]
    except ValueError:
        taken = False
    return taken and reads == 0, (
        f"the start carried the create's seen, {reads} read(s) between, "
        f"answered {'stale' if STALE_SEEN_MARK in said else 'taken' if taken else said[:120]!r}")


def budget_reused(rows, first, second):
    """T-1048: the zero receipt, a successful park, then a successful start;
    no hand. Refused calls remain evidence, but do not satisfy a step."""
    calls, phase, observed = {}, 0, []
    for row in once(rows):
        content = (row.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if not isinstance(b, dict):
                continue
            if b.get("type") == "tool_use":
                name = (b.get("name") or "").rsplit("__", 1)[-1]
                key = (b.get("input") or {}).get("key")
                if name == "raise_hand":
                    return False, "the crown raised its hand"
                calls[b.get("id")] = (name, key, phase)
            elif b.get("type") == "tool_result":
                name, key, called_at = calls.get(b.get("tool_use_id"), (None, None, None))
                if name not in ("start_agent", "sleep_agent"):
                    continue
                body = b.get("content")
                text = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
                if b.get("is_error"):
                    observed.append(f"{name} {key} refused")
                    continue
                observed.append(f"{name} {key}")
                if name == "start_agent":
                    try:
                        receipt = json.loads(text)
                    except ValueError:
                        return False, f"no start receipt: {text[:120]}"
                    if receipt.get("status") not in ("started", "waiting_for_worktree"):
                        return False, f"no fresh start: {text[:120]}"
                    if key == first and phase == 0 and receipt.get("budget_left") == 0:
                        phase = 1
                    elif key == second and called_at == 2:
                        phase = 3
                elif key == first and called_at == 1:
                    phase = 2
    return phase == 3, " → ".join(observed) or "no start receipt"


def plan_read_order(rows, key, word):
    """`plan_read` (T-991, D7): the crown had this request's whole plan before
    it accepted it, read off its transcript in order, record by record and
    block by block (the `assistant` and `user` records the model saw; Claude
    Code's `api-request-blob` copies are skipped). A get_ticket on `key` with
    plan true whose own result, not an error, carries the plan whole (`word`
    in it, no plan_bytes) and its request; then, called after that result,
    an accept_plan on `key` for that request whose own result is
    `accepted`. How the crown gets there is its own (T-930): a head read is
    not asked for, but one made before is checked, and every plain read
    that carries a plan must carry a head (at most 512 UTF-8 bytes before
    its `…`, without `word`), its plan_bytes the whole's UTF-8 length for
    the same request. (passed, the order seen)."""
    if not word:
        return False, "no word from the worker"
    word = word.lower()
    calls, heads, seen = {}, {}, []
    whole, done, bad = None, False, False

    def reading(b):
        body = b.get("content")
        text = body if isinstance(body, str) else " ".join(
            x.get("text", "") for x in body or [] if isinstance(x, dict))
        try:
            got = json.loads(text)
        except ValueError:
            return None
        return None if b.get("is_error") or not isinstance(got, dict) else got

    for r in once(rows):
        if r.get("type") not in ("assistant", "user") or done:
            continue
        blocks = (r.get("message") or {}).get("content")
        for b in blocks if isinstance(blocks, list) else []:
            if not isinstance(b, dict) or done:
                continue
            if b.get("type") == "tool_use":
                name = (b.get("name") or "").rsplit("__", 1)[-1]
                args = b.get("input") or {}
                if str(args.get("key", "")).strip() != key:
                    continue
                step = {("get_ticket", False): "head", ("get_ticket", True): "whole",
                        ("accept_plan", False): "accept"}.get((name, args.get("plan") is True))
                if step is None:
                    continue
                if step == "accept" and whole is None:
                    seen.append("accept_plan (before a whole read's result: not counted)")
                    continue
                seen.append(f"{name}{' plan' if step == 'whole' else ''}")
                calls[b.get("id")] = (step, args.get("request"), whole)
            elif b.get("type") == "tool_result" and b.get("tool_use_id") in calls:
                step, asked, after = calls.pop(b.get("tool_use_id"))
                got = reading(b)
                needs = (got or {}).get("needs_you") or {}
                plan = needs.get("plan") if isinstance(needs.get("plan"), str) else None
                request = needs.get("request")
                if step == "head" and plan is None:
                    seen.append("  → no plan")
                elif step == "head":
                    bare = plan[:-1] if plan.endswith("…") else None
                    good = bare is not None and len(bare.encode()) <= 512 \
                        and word not in plan.lower() and isinstance(needs.get("plan_bytes"), int) \
                        and bool(request)
                    seen.append(f"  → head {len(plan.encode())} B, plan_bytes "
                                f"{needs.get('plan_bytes')}, word "
                                f"{'in' if word in plan.lower() else 'not in'}"
                                + ("" if good else ": not a head"))
                    bad |= not good
                    if good:
                        heads.setdefault(request, set()).add(needs["plan_bytes"])
                elif step == "whole":
                    good = plan is not None and bool(request) and word in plan.lower() \
                        and "plan_bytes" not in needs
                    size = len(plan.encode()) if plan else 0
                    match = heads.get(request, {size}) == {size}
                    seen.append(f"  → whole {size} B, word "
                                f"{'in' if plan and word in plan.lower() else 'not in'}, request "
                                f"…{str(request)[-8:]}"
                                + ("" if request not in heads else
                                   f", the head's plan_bytes {'match' if match else 'differ'}")
                                + ("" if good else ": not the whole"))
                    bad |= not match
                    if good and match:
                        whole = request
                else:
                    outcome = (got or {}).get("outcome") or "refused"
                    seen.append(f"  → {outcome}, request "
                                f"{'the whole read' if asked == after else 'another'}'s")
                    done = asked == after and outcome == "accepted"
    return done and not bad, f"{word!r}: " + "; ".join(seen)


def row_ms(row):
    """A transcript row's time in epoch ms, or None."""
    import datetime
    try:
        return int(datetime.datetime.fromisoformat(row["timestamp"].replace("Z", "+00:00"))
                   .timestamp() * 1000)
    except (KeyError, TypeError, ValueError):
        return None


# A result that carries the crown's rules: `CROWN_WAKES`'s first words, in
# `crown_view()` wherever it rides and in every receipt of the text before
# T-930.
CROWN_RULES_MARK = "The board wakes this session on its own"


def crown_measures(rows, since_ms):
    """What the crown spent on a test (T-930), from its transcript since
    `since_ms`: its turns (a prompt that is not a tool result), its tokens
    (each API message once, though Claude Code writes a row per block),
    its tool calls, the calls before its first start_agent or create_ticket,
    and the bytes of each start_agent and ask_agent receipt."""
    rows = [r for r in rows if (row_ms(r) or 0) >= since_ms]
    turns, seen, tokens = 0, set(), dict(input=0, cache_read=0, cache_write=0, output=0)
    for r in rows:
        msg = r.get("message") or {}
        if r.get("type") == "user" and not r.get("isMeta"):
            content = msg.get("content")
            if isinstance(content, str) or (isinstance(content, list) and not any(
                    isinstance(b, dict) and b.get("type") == "tool_result" for b in content)):
                turns += 1
        elif r.get("type") == "assistant" and msg.get("id") not in seen:
            seen.add(msg.get("id"))
            u = msg.get("usage") or {}
            tokens["input"] += u.get("input_tokens", 0)
            tokens["cache_read"] += u.get("cache_read_input_tokens", 0)
            tokens["cache_write"] += u.get("cache_creation_input_tokens", 0)
            tokens["output"] += u.get("output_tokens", 0)
    calls = tool_calls(rows)
    names = [(n or "").rsplit("__", 1)[-1] for n, _ in calls]
    first = next((i for i, n in enumerate(names) if n in ("start_agent", "create_ticket")), None)
    receipts = [len(r["text"].encode()) for (n, r), short in zip(calls, names)
                if short in ("start_agent", "ask_agent") and r and not r["is_error"]]
    # Every result, and those that carry the crown's rules wherever they
    # ride (T-930's review: a first list_board carries them too).
    results = [r for _, r in calls if r]
    rules = [len(r["text"].encode()) for r in results if CROWN_RULES_MARK in r["text"]]
    return {"turns": turns, "tokens": tokens, "calls": len(calls),
            "calls_before_start": first, "receipt_bytes": receipts,
            "result_bytes": sum(len(r["text"].encode()) for r in results),
            "rules_bytes": rules, "failed": sum(1 for r in results if r["is_error"]),
            "model": model_of(rows)}


def compaction_frames(hook_lines):
    """T-1069: a session's compaction frames as the daemon took them, in
    order: `PreCompact`, a `SessionStart` whose reason is `compact`, and
    `PostCompact`."""
    return [l["event"] for l in hook_lines if l.get("event") in ("PreCompact", "PostCompact")
            or (l.get("event") == "SessionStart" and l.get("reason") == "compact")]


def held_by_compaction(hook_lines, queued_ms, prompts, words):
    """T-1069: (ok, observed) for words a person queued while the session's
    compaction window was open: queued after its first `PreCompact` and
    before the `PostCompact` that closed it, and the one prompt holding them
    entered at the close or after (a second's slack, as `held`: the hook's
    exec). `prompts` is `user_prompts(rows, at=True)`. Words queued outside
    the window prove nothing about the hold and fail."""
    pre = next((l["at_ms"] for l in hook_lines if l.get("event") == "PreCompact"), None)
    post = next((l["at_ms"] for l in hook_lines if l.get("event") == "PostCompact"
                 and pre is not None and l["at_ms"] >= pre), None)
    if pre is None or post is None:
        return False, f"no closed window: PreCompact at {pre}, PostCompact at {post}"
    if queued_ms is None:
        return False, "nothing was queued"
    span = f"the window was open {(post - pre) / 1000:.1f} s"
    if not pre <= queued_ms < post:
        return False, (f"queued {(queued_ms - pre) / 1000:+.1f} s from its opening, outside it "
                       f"({span}): the hold was not exercised")
    entered = [iso_ms(t) for p, t in prompts if words in p]
    if len(entered) != 1 or entered[0] is None:
        return False, f"{len(entered)} prompts hold the words; {span}"
    ok = entered[0] >= post - 1000
    return ok, (f"queued {(queued_ms - pre) / 1000:.1f} s into the window; {span}; the prompt "
                f"entered {(entered[0] - post) / 1000:+.1f} s from its close")


def rules_after_compaction(rows, since_ms):
    """T-1069: (ok, observed) from the crown's transcript since `since_ms`:
    its first compact boundary, the last board tool result before it (the
    rules already told: no `CROWN_RULES_MARK`) and the first after it, which
    carries them again. All three are required: with no board result before
    the boundary, or one carrying the rules, nothing shows the rules were
    owed again rather than still owed, and the check fails. Only the
    conversation's own rows are read: Claude Code 2.1.312 writes each call
    first into an `api-request-blob` row with no timestamp, which `once`
    would keep in the real row's place."""
    names, seq = {}, []
    rows = [r for r in rows if r.get("type") in ("user", "assistant", "system")]
    for r in once(rows):
        if (row_ms(r) or 0) < since_ms:
            continue
        if r.get("type") == "system" and r.get("subtype") == "compact_boundary":
            seq.append(("boundary", None))
            continue
        content = (r.get("message") or {}).get("content")
        for b in content if isinstance(content, list) else []:
            if not isinstance(b, dict):
                continue
            if b.get("type") == "tool_use":
                names[b.get("id")] = b.get("name") or ""
            elif b.get("type") == "tool_result" and names.get(b.get("tool_use_id"), "").startswith(
                    "mcp__mesimon__"):
                body = b.get("content")
                text = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
                seq.append(("result", CROWN_RULES_MARK in text))
    at = next((i for i, (k, _) in enumerate(seq) if k == "boundary"), None)
    if at is None:
        return False, "no compact boundary in the crown's transcript"
    before = [c for k, c in seq[:at] if k == "result"]
    after = [c for k, c in seq[at + 1:] if k == "result"]
    said = lambda c: "the rules" if c else "no rules"  # noqa: E731
    seen = (f"last board result before the boundary: {said(before[-1]) if before else 'none'}; "
            f"first after: {said(after[0]) if after else 'none'}")
    return bool(before and not before[-1] and after and after[0]), seen


def measures_line(m):
    if not m:
        return "no crown transcript"
    t = m["tokens"]
    return (f"crown ({m.get('model')}): {m['turns']} turns, {m['calls']} tool calls, "
            f"{m.get('failed', 0)} failed, results {m.get('result_bytes', 0)} bytes, rules in "
            f"{len(m.get('rules_bytes', []))} ({sum(m.get('rules_bytes', []))} bytes)"
            + (f" ({m['calls_before_start']} before its first start)" if m["calls_before_start"] is not None else "")
            + f", tokens in {t['input']} + cache read {t['cache_read']} + cache write {t['cache_write']}"
            f", out {t['output']}"
            + (f", receipts {'/'.join(str(b) for b in m['receipt_bytes'])} bytes" if m["receipt_bytes"] else ""))


def tool_uses(rows):
    """(tool name, its input, its result text or None) for every tool use in
    a transcript, in order: `tool_calls` and `tool_inputs` side by side."""
    return [(n, i, (r or {}).get("text")) for (n, i), (_, r)
            in zip(tool_inputs(rows), tool_calls(rows))]


def context_read(text):
    """`get_ticket`'s `context` in a tool result's text (T-969): (tokens,
    window), or None where the answer carries none or no tokens."""
    m = re.search(r'"context"\s*:\s*\{([^}]*)\}', text or "")
    if not m:
        return None
    tokens = re.search(r'"tokens"\s*:\s*(\d+)', m.group(1))
    window = re.search(r'"window"\s*:\s*(\d+)', m.group(1))
    if not tokens or not window:
        return None
    return int(tokens.group(1)), int(window.group(1))


def sampled_before(rows, key, prefix):
    """`sampled_before:<prefix>` (T-973, round 2), over a crown's transcript
    in order, record by record and block by block: the board's overrun line
    for `key` arrives as a prompt; in that turn a `sample_agent` call of
    `key` is made, and its own result comes back, not an error, its JSON
    naming `key` with `state` and `captured_at_ms`; only then a reply line
    beginning `prefix`. A verdict before that result fails, in the call's
    own message or a later one; another prompt before the verdict starts
    over. (passed, what was seen)."""
    stage, calls, seen = "wake", set(), []
    for r in rows:
        blocks = (r.get("message") or {}).get("content")
        blocks = blocks if isinstance(blocks, list) else [{"type": "text", "text": blocks or ""}]
        results = [b for b in blocks if isinstance(b, dict) and b.get("type") == "tool_result"]
        if r.get("type") == "user" and not r.get("isMeta") and not results:
            said = "\n".join(texts(r)).strip()
            if not said or said.startswith("<task-notification>"):
                continue
            if f"{key} " in said and "has run its turn" in said:
                stage, calls = "sample", set()
                seen.append("wake")
            elif stage in ("sample", "verdict"):
                stage = "wake"
                seen.append("a prompt before the verdict")
            continue
        for b in blocks:
            if not isinstance(b, dict):
                continue
            kind = b.get("type")
            if kind == "tool_use" and stage == "sample" and b.get("name") == "mcp__mesimon__sample_agent" \
                    and (b.get("input") or {}).get("key") == key:
                calls.add(b.get("id"))
            elif kind == "tool_result" and stage == "sample" and b.get("tool_use_id") in calls:
                body = b.get("content")
                text = body if isinstance(body, str) else " ".join(
                    x.get("text", "") for x in body or [] if isinstance(x, dict))
                try:
                    reading = json.loads(text)
                except ValueError:
                    reading = None
                good = not b.get("is_error") and isinstance(reading, dict) \
                    and reading.get("key") == key and "state" in reading and "captured_at_ms" in reading
                if good:
                    stage = "verdict"
                    seen.append("sampled")
                else:
                    seen.append(f"sample refused: {text[:80]!r}")
            elif kind == "text" and r.get("type") == "assistant" and any(
                    l.strip().strip("*`").startswith(prefix) for l in (b.get("text") or "").splitlines()):
                if stage == "verdict":
                    seen.append("verdict")
                    return True, " → ".join(seen)
                if stage == "sample":
                    seen.append("a verdict before the reading")
                    return False, " → ".join(seen)
    return False, " → ".join(seen) or "no overrun line reached the crown"


def tool_inputs(rows):
    """(tool name, its input) for every tool use in a transcript, in order."""
    out = []
    for row in once(rows):
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


def steer_verdict(kind, arg, lines, tid, prompts):
    """T-974: a steer beside the crown's ask still out, judged off the feed
    `lines` and the worker's user `prompts`; (ok, observed).

    The ticket's `ask_agent_sent` lines name their asks; the steer is the
    first whose `beside` names an earlier send's ask at `sent` or `handed`.
    The answered wakes after it each name the ask their turn answered and its
    word then (round 1).

    `steered:<word>[:<secs>]`: the steer is `now` or `immediately`, and the
    first answered wake after it names the earlier ask at <word>, within
    <secs> of that ask's send when given (the long command cut short).

    `held_steer:<payload>` (round 2): the first send is `now` (a held
    submit), the steer `immediately`, two distinct ask ids, the steer beside
    the first at `sent`; exactly two of the worker's prompts carry the
    payload (the same words twice, once as the steer, once as the held
    submit, T-934); and every answered wake naming the first says `taken`,
    at least one. No wake naming the steer is required: the engine's Stop
    and the held prompt can come inside one settle, one stretch to the
    board, whose end is the submit's (round 1, the first ST2 run)."""
    sends = [l for l in lines if l.get("kind") == "board"
             and l.get("cmd") == "ask_agent_sent" and l.get("ticket") == tid]
    levels = ("immediately",) if kind == "held_steer" else ("now", "immediately")
    found = None
    for i, l in enumerate(sends):
        if l.get("outcome") not in levels:
            continue
        before = {e.get("ask"): e for e in sends[:i] if e.get("ask")}
        for b in l.get("beside") or []:
            ask, _, word = b.partition(" ")
            if ask in before and ask != l.get("ask") and word in ("sent", "handed"):
                found = (l, before[ask], word)
                break
        if found:
            break
    if not found:
        return False, ("; ".join(f"{l.get('outcome')} {l.get('ask')} beside {l.get('beside') or []}"
                                 for l in sends) or "no ask_agent_sent line")
    steer, first, word = found
    wakes = [(w["at_ms"], *(w.get("ask") or "? ?").split(" ", 1)) for w in lines
             if w.get("kind") == "crown_wake" and w.get("worker") == tid
             and w.get("cause") == "answered" and w.get("at_ms", 0) >= steer["at_ms"]]
    told = ", ".join(f"{'first' if a == first['ask'] else 'steer' if a == steer['ask'] else a}"
                     f" {w} at +{(t - first['at_ms']) / 1000:.0f} s" for t, a, w in wakes)
    observed = (f"{first.get('outcome')} first {first['ask']}, {steer['outcome']} steer "
                f"{steer.get('ask')} beside it {word}; wakes: {told or 'none'}")
    if kind == "steered":
        want, _, secs = arg.partition(":")
        ok = bool(wakes) and wakes[0][1] == first["ask"] and wakes[0][2] == want
        if ok and secs:
            ok = (wakes[0][0] - first["at_ms"]) / 1000 <= float(secs)
        return ok, observed
    payload = " ".join(arg.split())
    carrying = [p for p in prompts if payload and payload in " ".join(p.split())]
    first_wakes = [w for _, a, w in wakes if a == first["ask"]]
    ok = (first.get("outcome") == "now" and steer.get("outcome") == "immediately"
          and bool(first.get("ask")) and bool(steer.get("ask")) and first["ask"] != steer["ask"]
          and word == "sent" and len(carrying) == 2
          and bool(first_wakes) and all(w == "taken" for w in first_wakes))
    return ok, f"{observed}; prompts carrying the payload: {len(carrying)} of {len(prompts)}"


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


# A transcript record the fixture keeps whole (T-1014): the conversation's
# own. Every other kind (the engine's request blobs, attachments, file
# history, latches) keeps only these keys: what a tail reader may pass over,
# never what it carries, which holds the person's memory and system prompt.
WHOLE_RECORDS = ("user", "assistant", "system")
STUB_KEYS = ("type", "subtype", "uuid", "parentUuid", "timestamp", "promptId", "isMeta",
             "operation")


def fixture_of(raw, repo, state_dir):
    """A recorded trace as the e2e fixture: the header, then each line with
    `t` (ms from the first) for its stamp, the session as `{session}`, the
    transcript's path, the session's cwd, the repo, the state dir and the
    home (slashed or dashed, as Claude Code names a project) as
    placeholders, a thinking block with no words or signature, and a record
    outside `WHOLE_RECORDS` cut to `STUB_KEYS` (an attachment keeps its
    kind). `raw` is the raw file's lines, its header first."""
    header, lines = raw[0], raw[1:]
    sessions = sorted({l["session"] for l in lines if len(l["session"]) == 36})
    names = {}
    for n, s in enumerate(sessions, 1):
        tag = "" if len(sessions) == 1 else str(n)
        names[s] = f"{{session{tag}}}"
        names[sid16(s)] = f"{{sid16{tag}}}"
    swaps = [(k, v) for k, v in names.items()]
    for l in lines:
        payload = l.get("payload") or {}
        if payload.get("transcript_path"):
            swaps.append((payload["transcript_path"], "{transcript}"))
        if payload.get("cwd"):
            swaps += [(payload["cwd"], "{cwd}"), (os.path.realpath(payload["cwd"]), "{cwd}")]
    home = os.environ["HOME"]
    swaps += [(repo, "{repo}"), (state_dir, "{state}"), (home, "~"),
              (home.replace("/", "-"), "~"), (home.replace("/", "-").replace(".", "-"), "~")]
    swaps = sorted({p for p in swaps if p[0]}, key=lambda p: -len(p[0]))
    base = lines[0]["at_ms"] if lines else 0
    out = [header]
    for l in lines:
        l = dict(l)
        l["t"] = l.pop("at_ms") - base
        record = l.get("line")
        if isinstance(record, dict):
            if record.get("type") not in WHOLE_RECORDS:
                stub = {k: record[k] for k in STUB_KEYS if k in record}
                if isinstance(record.get("attachment"), dict):
                    stub["attachment"] = {"type": record["attachment"].get("type")}
                record = stub
            else:
                record = json.loads(json.dumps(record))
                for block in (record.get("message") or {}).get("content") or []:
                    if isinstance(block, dict) and block.get("type") == "thinking":
                        block["thinking"] = ""
                        block["signature"] = ""
            l["line"] = record
        text = json.dumps(l, ensure_ascii=False)
        for old, new in swaps:
            text = text.replace(old, new)
        out.append(json.loads(text))
    return out


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


def board_exchange(rows, kind, evidence):
    """X1–X3: judge the actual call, receipt and words at both ends.

    A feed entry alone also records a refusal, and a queued receipt does
    not prove delivery. The nonce comes only from the peer's prompt, never
    the crown's brief. Exactly one call rules out retries and reply loops.
    """
    if kind not in ("ask", "reply", "closed"):
        return False, f"unknown exchange: {kind}"
    inputs = tool_inputs(rows)
    local_work = [n for n, _ in inputs if n in ("Write", "Edit", "NotebookEdit", "Agent", "Task",
                  "mcp__mesimon__start_agent", "mcp__mesimon__ask_agent", "mcp__mesimon__create_ticket")]
    if local_work:
        return False, f"work started here instead of the peer: {', '.join(local_work)}"
    calls = [(i, r) for (n, i), (_, r) in zip(inputs, tool_calls(rows))
             if n == "mcp__mesimon__ask_board"]
    if len(calls) != 1:
        return False, f"{len(calls)} ask_board calls; expected one"
    args, result = calls[0]
    if not args.get("board") or args["board"] not in (PEER_NAME, evidence.get("path")):
        return False, f"wrong destination: {args.get('board')!r}"
    if not result:
        return False, "ask_board has no result"
    if kind == "closed":
        ok = result["is_error"] and "not open" in result["text"]
        return ok, f"closed board receipt: {result['text'][:160]}"
    try:
        # The first crown receipt can carry the board's rules after its
        # JSON (T-930). Judge that first object, not the trailing prose.
        receipt, _ = json.JSONDecoder().raw_decode(result["text"].lstrip())
    except (ValueError, TypeError):
        receipt = None
    if result["is_error"] or not isinstance(receipt, dict) or not (
            receipt.get("board") == PEER_NAME and receipt.get("road") == "queued"
            and receipt.get("crown") == evidence.get("crown")
            and receipt.get("reply") is (kind == "reply")):
        return False, f"wrong receipt: {result['text'][:200]}"
    sent = args.get("text", "")
    delivered = evidence.get("delivered", "")
    if not sent or sent not in delivered:
        return False, "the crown's exact words never reached the peer's pane"
    incoming = evidence.get("incoming", "")
    prompts = user_prompts(rows)
    if not incoming or sum(incoming in p for p in prompts) != 1:
        return False, "the peer's attributed prompt did not arrive exactly once"
    peer_receipt = evidence.get("receipt", {})
    if not (peer_receipt.get("board") == evidence.get("local_board")
            and peer_receipt.get("crown") == evidence.get("local_key")
            and peer_receipt.get("road") == "queued"
            and peer_receipt.get("reply") is (kind == "ask")):
        return False, f"wrong peer receipt: {peer_receipt}"
    nonce = evidence.get("nonce")
    if not nonce:
        return False, "no fresh peer reference"
    if kind == "ask":
        header = f"The crown of the {evidence['local_board']} board ({evidence['local_key']} "
        good = header in delivered and f"ask_board board={evidence['local_board']} replies" in delivered
        good &= "0.1.0-beta.2" in sent and "/version" in sent
        good &= nonce in last_reply(rows)
    else:
        good = f"The crown of the {evidence['local_board']} board replied: {sent}" in delivered
        good &= nonce in sent
    return good, (f"one {'reply' if kind == 'reply' else 'ask'}, queued and delivered; "
                  f"peer reference {nonce}, attribution/content {'match' if good else 'differ'}")


# A shell that stands in for Claude on the peer board: it paints Claude's
# composer (the daemon pastes only into one) and keeps every line pasted.
PEER_STUB = (
    "#!/bin/sh\n"
    "printf '\\033]0;✳ Claude Code\\007\\033[999;1H\\033[3A────────────────────"
    "\\033[999;1H\\033[2A❯ \\033[999;1H\\033[1A────────────────────"
    "\\033[999;1H  ? for shortcuts\\033[H'\n"
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
        self.process = subprocess.Popen([rig.bin, "daemon", "--repo", self.repo], stdin=subprocess.DEVNULL,
                                        stdout=log, stderr=log, env=env, start_new_session=True, cwd=self.repo)
        log.close()
        self.wire = Wire(self.paths.orch_sock, client="rig-peer", connect_for=30)
        self.feed_mark = len(feed_all(self.paths))
        self.ticket = self.wire.create_ticket("TODO", "Relay release")
        self.wire.set_workspace(self.ticket, "shared_checkout")
        self.session = self.wire.spawn(self.ticket, "claude", submit_prompt=False)["id"]
        # Like boardlink_e2e's one_turn: the stand-in has no real engine to
        # emit hooks. A queued peer ask drains only after a proven idle.
        # Its title/composer above match common::COMPOSER (T-1002/T-1019),
        # so the startup probe cannot mistake it for an unanswered modal.
        try:
            for event, body, state in (("UserPromptSubmit", {"prompt": "ready"}, "working"),
                                       ("Stop", {"stop_hook_active": False}, "idle")):
                subprocess.run([rig.bin, "hook", "--sock", os.path.join(self.paths.rt_dir, "hook.sock"),
                                "--session", self.session, "--event", event],
                               input=json.dumps(body), text=True, capture_output=True, check=True, timeout=10)
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    rec = next((s for s in self.wire.board()["sessions"] if s["id"] == self.session), None)
                    if rec and word_of(rec["state"]) == state:
                        break
                    time.sleep(0.05)
                else:
                    raise WireError(f"the peer stub never became {state}")
        except (WireError, OSError, subprocess.SubprocessError):
            self.stop()
            raise
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

    def received(self):
        """Words actually read by the peer's stub, not just accepted by its daemon."""
        try:
            with open(self.got) as f:
                return f.read()
        except FileNotFoundError:
            return ""

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
        self.wire.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=5)
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
        if getattr(args, "record", False):
            self.seams["MESIMON_RECORD_TRACE"] = os.path.join(self.out, "trace.jsonl")
        self.phone = None
        # The second board X1 lays (T-733), while it stands, and its feed after.
        self.peer = None
        self.peer_fed = []
        # The checkout's looks while a test attributes its moves by session
        # (`crown_wrote_nothing`, `codex.crown_changes`); None otherwise.
        self.looks = None

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
        env_file = os.path.join(self.paths.rt_dir, "shellenv.env")
        for attempt in range(1, 11):
            self.stop_daemon()
            # The shell environment as it stood before this attempt's daemon
            # (A3, T-945): only a write after it is the winner's own.
            self.env_before = file_generation(env_file)
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

    def enterprise_claude(self):
        """T-1069: the real Claude Code, but for `auth status`, which says an
        enterprise account: the probe's own reason for a native road."""
        wrapper = os.path.join(self.out, "claude-enterprise")
        with open(wrapper, "w") as f:
            f.write("#!/bin/sh\n# The rig's NX tests (T-1069): an enterprise account to the probe.\n"
                    'if [ "$1" = "auth" ] && [ "$2" = "status" ]; then\n'
                    "  echo '{\"loggedIn\":true,\"subscriptionType\":\"enterprise\"}'; exit 0\n"
                    "fi\n"
                    f'exec "{self.claude}" "$@"\n')
        os.chmod(wrapper, 0o755)
        return wrapper

    def passed_over_mod(self):
        """T-1069's simulation: the laid sources, with the mod's
        `session.compact` hook passing the event on before it reads it, as a
        seated security default's `next.to(e, "append")` keeps it from a
        person's mod. Everything else is the mod as shipped."""
        src = os.path.join(self.repo, "crates/mesimon-daemon/mod")
        dst = os.path.join(self.paths.state_dir, "rig", "mod-compact-passed-over")
        if os.path.exists(dst):
            shutil.rmtree(dst)
        shutil.copytree(src, dst)
        path = os.path.join(dst, "hooks", "register.ts")
        with open(path) as f:
            text = f.read()
        head = "  on('session.compact', async ($, e, next) => {\n"
        if text.count(head) != 1:
            die("the mod's session.compact hook moved: the NX simulation cannot be patched in")
        text = text.replace(head, head + "    if (RIG_PASSED_OVER) return next(e)\n", 1)
        anchor = "let typedCompact: { closed: boolean } | undefined\n"
        if anchor not in text:
            die("the mod's typedCompact moved: the NX simulation's switch cannot be declared")
        text = text.replace(anchor, anchor + "const RIG_PASSED_OVER = true\n", 1)
        os.remove(os.path.join(dst, "hooks", "register.test.ts"))
        with open(path, "w") as f:
            f.write(text)
        out = subprocess.run([self.claude, "plugin", "validate", dst], capture_output=True, text=True)
        if out.returncode != 0:
            die(f"the NX simulation's mod does not validate:\n{out.stdout}{out.stderr}")
        return dst

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
            "model": TIER_MODEL,
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
        small = next((t for t in board.get("tiers", []) if t["name"] == SMALL_TIER_NAME), None)
        small_spec = {**spec, "id": small["id"] if small else ulid(), "name": SMALL_TIER_NAME,
                      "description": SMALL_TIER_DESCRIPTION}
        if small != small_spec:
            self.wire.save_tier("board", small_spec)
        crown = next((t for t in board.get("tiers", []) if t["name"] == CROWN_TIER_NAME), None)
        crown_spec = {**spec, "id": crown["id"] if crown else ulid(), "name": CROWN_TIER_NAME,
                      "model": self.args.crown_model, "effort": self.args.crown_effort,
                      "description": CROWN_TIER_DESCRIPTION}
        if crown != crown_spec:
            self.wire.save_tier("board", crown_spec)
        self.crown_tier = crown_spec
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
            f"the crown is autonomous on {CROWN_TIER_NAME} ({crown_spec['model']}, effort "
            f"{crown_spec['effort']})")

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
            # A blind test's ticket reads as any other on list_board: its
            # title says nothing of what the crown is to decide.
            if test.get("blind"):
                test["title"] = f"{test['id']} · do the steps below"
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
        self.wire.request({"cmd": "set_ticket_tier", "id": crown, "tier": self.crown_tier["id"]})
        self.wire.crown(crown)
        self.crown = crown
        self.crown_key = self.ticket(self.board(), crown)["short_key"]
        say(f"  filed and crowned {self.crown_key}: the rig's crown, on the checkout")

    def file_one(self, test, title):
        tid = self.wire.create_ticket(test.get("column", "TODO"), title)
        if "{mcp_probe}" in test["brief"]:
            receipt = os.path.join(self.out, f"mcp-probe-{tid}.json")
            test["mcp_probe_receipt"] = receipt
            command = shlex.join([sys.executable, "-B", os.path.join(HERE, "rig", "mcp_probe.py"),
                                  self.paths.orch_sock, tid, receipt])
            test["brief"] = test["brief"].replace("{mcp_probe}", command)
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
        lines = [f"# {test['id']}: what the rig checks", ""]
        if not test.get("blind"):
            lines += [f"**Expected.** {test['expect']}", ""]
        lines.append("**Steps.**")
        for step in test["steps"]:
            if "crown" in step:
                said = step["crown"].format(key="this ticket", key2="the second", key3="the third",
                                            crown="the crown's ticket")
                lines.append(f"- the crown is told: “{said}”")
            else:
                lines.append(f"- the rig: `{step['rig']}`")
        if not test.get("blind"):
            lines += ["", "**Checks.** " + ", ".join(f"`{c}`" for c in test["checks"])]
        return "\n".join(lines)

    def crown_brief(self, tests):
        # A blind test (T-930) judges what the crown decides from the board's
        # own words, so its brief names the test and nothing it checks.
        order = "\n".join(f"- {t['id']} ({t['key']}): its steps come as prompts." if t.get("blind")
                          else f"- {t['id']} ({t['key']}): "
                          f"{t['title'].split(' · ', 1)[-1].removesuffix(': do the steps below')}. "
                          f"{t['expect']}" for t in tests)
        # The rules above script a test's every call; a blind test judges what
        # the crown does with none named, so they are lifted there (T-930's
        # first U1 ended on the rig's "raise no hand" instead).
        blind = ("\nA test whose line says its steps come as prompts lifts the rules above, but "
                 "for archiving: in it, do what this board's crown does, at each step and on "
                 "each wake, and end each turn with the line its step names.\n"
                 if any(t.get("blind") for t in tests) else "")
        return f"""You wear the crown of the rig: a mesimon board inside a ticket worktree, run by `ci/rig.py`, that tests mesimon's Claude Code mod road on the real Claude Code. You are the subject under test: each test proves a crown can drive a worker through the board while the mod relays every frame, with nothing read off a screen.

How a run goes. The rig sends you one step at a time, as a prompt that names one test ticket and the exact tool calls to make. Make exactly those calls, in that order, and nothing else: read no file, run no command, write no note, file no ticket and raise no hand; the rig reads the board itself and writes every verdict. Move a ticket only when the rig tells you to: a test that passed goes to DONE with move_ticket. Never archive a ticket: on this board only `ci/rig.py --reset` archives. Take a ticket's seen stamp from get_ticket with its key right before a call that needs one. When the board wakes you for a worker, do what the step said for that moment; if it said nothing, end your turn with one line naming the test and the worker's state word.

The tests, in order, and what each needs:
{order}
{blind}
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
        # A verdict on crown text is worth only the model it ran on (T-930).
        if model and not model.startswith(self.crown_tier["model"]):
            die(f"the crown ran on {model}, not its tier's {self.crown_tier['model']}")
        self.crown_model = model

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

    def peer_delivery(self, record):
        """Do not manufacture a reply to words the peer never received."""
        crown = self.agent_of(self.board(), self.crown)
        rows = [r for r in (transcript(crown) if crown else [])
                if (row_ms(r) or 0) >= record["t0_ms"]]
        sent = [i.get("text") for n, i in tool_inputs(rows) if n == "mcp__mesimon__ask_board"]
        if len(sent) != 1 or not sent[0]:
            record["failures"].append(f"expected one crown ask before peer delivery, got {len(sent)}")
            return False

        def landed():
            got = self.peer.received()
            return got if sent[0] in got else None

        delivered = self.wait_for("the crown's words in the peer's pane", landed, 30, self.watch)
        record["peer"]["delivered"] = delivered or ""
        if not delivered:
            record["failures"].append("the peer never read the crown's words")
        return bool(delivered)

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
        if self.looks is not None:
            look = self.look(board)
            if look != self.looks[-1]:
                self.looks.append(look)
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
            if until.get("filed"):
                # A request in a person's words (DL1, T-916): the ticket the
                # crown filed and started since the step began stands in for
                # the test's own, which nothing starts.
                tid = next(iter(self.crown_filed(self.feed_lines[mark:])), None)
                if tid is None:
                    return False
            if until.get("crown_only"):
                # A step that is the crown's read alone (DB1, T-941): no
                # worker and no feed line; the crown's settle ends it.
                return True
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
            want = until.get("state", "idle")
            # A blind test's crown may park a worker that went idle (T-930:
            # N1's parked its finished worker), which is idle and then some.
            return word_of(rec["state"]) == want or (
                test.get("blind") and want == "idle" and word_of(rec["state"]) == "sleeping")

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

    def restore_binary(self, record):
        """Put back the stamp `binary_moved` changed (DB1, T-941), on the
        file it changed alone: one a build replaced or rewrote since keeps its
        own stamp, and the cleanup is a failure of the test. Its atime is not
        compared: every exec of the binary moves it."""
        moved = getattr(self, "bin_moved", None)
        if moved is None:
            return True
        self.bin_moved = None
        before, after = moved
        ident = lambda st: (st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns)  # noqa: E731
        try:
            if ident(os.stat(self.bin)) == ident(after):
                os.utime(self.bin, ns=(before.st_atime_ns, before.st_mtime_ns))
                return True
            why = f"{self.bin} changed after binary_moved, so its stamp was left as it is"
        except OSError as e:
            why = f"putting {self.bin}'s stamp back failed: {e}"
        record["failures"].append(f"cleanup: {why}")
        say(f"  cleanup: {why}")
        return False

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
        if what == "prompt_at_start":
            # T-961: the person's words the moment the wake returns, while
            # the woken seat is still spawning: its box cannot be drawn yet
            # (a resume draws 0.1-4 s after `SessionStart`, which comes later
            # still), so the board's wait for the box is what is tested, never
            # a race with it. The step ends at the worker's turn on them and
            # the crown's wake.
            rec = self.agent_of(self.board(), test["ticket"])
            self.wire.sleep(rec["id"])
            if not self.wait_for("the worker parked", lambda: word_of((self.agent_of(
                    self.board(), test["ticket"]) or {}).get("state")) == "sleeping", 60, self.watch):
                record["failures"].append("the worker would not park")
                return False
            self.watch()
            mark = len(self.feed_lines)
            self.wire.wake(rec["id"])
            try:
                r = self.wire.prompt(test["ticket"], step["text"], queued=False)
            except WireError as e:
                record["failures"].append(f"the person's prompt at the wake: {e}")
                return False
            state = (self.agent_of(self.board(), test["ticket"]) or {}).get("state")
            self.log(f"the person's prompt at {test['key']}'s wake, the seat {word_of(state)}: "
                     f"{r.get('resp')}")

            def answered():
                r = self.agent_of(self.board(), test["ticket"])
                return r and word_of(r["state"]) == "idle" and any(
                    l.get("kind") == "hook" and of_session(l, r["id"]) and l.get("event") == "Stop"
                    for l in self.feed_lines[mark:])

            if not self.wait_for("the worker's turn on the person's words", answered,
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the worker never answered the person's words")
                return False
            wake = self.wait_for("the board's wake for the crown", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == test["ticket"]), None), WAKE_GRACE, self.watch)
            self.quiet_since = None
            self.wait_for("crown", lambda: self.crown_settled(after_ms=wake["at_ms"], quiet=0)
                          if wake else self.crown_settled(), STEP_TIMEOUT, self.watch)
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
        if what == "person_in_crown":
            # A1 (T-918): a person comes into the crown's pane, a tmux client
            # on its session as the board's focus would attach, and stays
            # until `person_leaves`.
            crown = self.agent_of(self.board(), self.crown)
            tmux = os.environ.get("MESIMON_TMUX_BIN") or (
                os.path.join(self.repo, "target/debug/mesimon-tmux")
                if os.path.exists(os.path.join(self.repo, "target/debug/mesimon-tmux")) else "tmux")
            self.person = subprocess.Popen(
                [tmux, "-S", self.paths.tmux_sock, "-C", "attach", "-t", sid16(crown["id"])],
                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            came = self.wait_for("the person in the crown's pane", lambda: self.tmux(
                "list-clients", "-t", sid16(crown["id"]), "-F", "#{client_name}").stdout.strip(),
                10, self.watch)
            if not came:
                record["failures"].append("no tmux client came onto the crown's session")
                return False
            record["person_came_ms"] = int(time.time() * 1000)
            return True
        if what == "pane_keys":
            # A person's keys in the worker's pane (T-1014): the dialog the
            # card shows answered or refused there, never through the board.
            # `screen` is words the pane must show first, `keys` what tmux
            # sends (`literal` typed as text), `hold` seconds the rig then
            # watches before the step ends, `until` the card word it then
            # waits for, `fed` a board line for the ticket it waits for
            # first. The press's time is kept for the checks (`keys`).
            rec = self.agent_of(self.board(), test["ticket"])
            pane = sid16(rec["id"])
            if step.get("screen") and not self.wait_for(
                    f"{step['screen']!r} in {test['key']}'s pane", lambda: step["screen"] in
                    self.tmux("capture-pane", "-p", "-t", pane).stdout, 60, self.watch):
                record["failures"].append(f"the pane never showed {step['screen']!r}")
                return False
            if step.get("fed") and not self.wait_for(f"the board's {step['fed']}", lambda: any(
                    l.get("kind") == "board" and l.get("cmd") == step["fed"]
                    and l.get("ticket") == test["ticket"] for l in self.feed_lines), 60, self.watch):
                record["failures"].append(f"the board never said {step['fed']}")
                return False
            time.sleep(step.get("before", 1))
            self.watch()
            at_ms = int(time.time() * 1000)
            if step.get("literal"):
                self.tmux("send-keys", "-t", pane, "-l", step["literal"])
            out = self.tmux("send-keys", "-t", pane, *step.get("keys", []))
            record["keys"].append(at_ms)
            self.log(f"the person pressed {' '.join(step.get('keys', [])) or 'nothing'} in "
                     f"{test['key']}'s pane ({out.returncode}), press {len(record['keys'])}")
            deadline = time.time() + step.get("hold", 0)
            while time.time() < deadline:
                self.watch()
                self.sub.wait(0.5)
            if step.get("until"):
                want = step["until"]
                mark = next((i for i, l in enumerate(self.feed_lines)
                             if l.get("at_ms", 0) >= at_ms), len(self.feed_lines))

                def there():
                    r = self.agent_of(self.board(), test["ticket"])
                    stopped = want != "idle" or any(
                        l.get("kind") == "hook" and of_session(l, r["id"])
                        and l.get("event") == "Stop" for l in self.feed_lines[mark:]) or (
                        r["state"].get("stop_reason") or r["state"].get("reason")) == "interrupted"
                    return word_of(r["state"]) == want and stopped

                if not self.wait_for(f"{test['key']} {want}", there, STEP_TIMEOUT, self.watch):
                    record["failures"].append(f"the worker never went {want} after the keys")
                    return False
                self.quiet_since = None
                self.wait_for("crown", lambda: self.crown_settled(), STEP_TIMEOUT, self.watch)
            return True
        if what == "draft_at_launch":
            # T-987: a draft in the worker's box while its launch's words wait
            # at the daemon's launch barrier (`crown_road` with `barrier`):
            # the gap between the sample that let them go and the edge's own
            # read of the box. The barrier goes up once the worker's seat is
            # on the board, so the crown's own words never meet it; the
            # writer waits at it, so the rig then reads only the files and
            # tmux until it is down.
            barrier = os.path.join(self.out, "barrier")
            reached = barrier + ".reached"
            for stale in (barrier, reached):
                if os.path.exists(stale):
                    os.remove(stale)
            rec = self.wait_for(f"{test['key']}'s seat", lambda: self.agent_of(
                self.board(), test["ticket"]), 120)
            if not rec:
                record["failures"].append("no seat on the ticket for the barrier")
                return False
            with open(barrier, "w"):
                pass
            pane = sid16(rec["id"])
            deadline = time.time() + 120
            while time.time() < deadline and not os.path.exists(reached):
                time.sleep(0.02)
            if not os.path.exists(reached):
                os.remove(barrier)
                record["failures"].append("the launch's words never reached the barrier")
                return False
            self.tmux("send-keys", "-t", pane, "-l", step["text"])
            typed = step["text"].strip()[:20]
            until = time.time() + 5
            while time.time() < until and typed not in self.tmux(
                    "capture-pane", "-p", "-t", pane).stdout:
                time.sleep(0.05)
            os.remove(barrier)
            os.remove(reached)
            self.log(f"the person's draft typed into {test['key']}'s box at the launch's edge")
            return True
        if what == "crown_road":
            # A3 (T-945): the rig's crown on `road` (hooks|mod|native). The road
            # is read at each launch, so the daemon restarts with the seam
            # and the crown is parked and woken onto it; `idle_ms` is the
            # bound a client attached to its pane holds a wake for.
            road = step.get("road", "hooks")
            env = {"MESIMON_CLAUDE_ROAD": "hooks"} if road == "hooks" else {}
            # Back off the native road, the daemon's own probe must land
            # before the crown is woken: a launch before it takes the hook set.
            want_probe = "the mod validated" if road == "mod" and self.crown_native else None
            if road == "native":
                # T-1069: every launch native, the crown's and its workers',
                # on the real Claude Code and the real mod. The wrapper says
                # an enterprise account to the probe's `auth status`, which
                # is what makes the probe's verdict classic-off; this account
                # seats no security default, so the classic events still
                # reach the mod, which ignores them on this road. With
                # `passed_over`, the mod is a copy whose `session.compact`
                # hook passes the event on unread: a SIMULATION of a seated
                # security default's skip (a person's plugin cannot spell
                # `next.to`), never a managed machine's.
                env["MESIMON_CLAUDE_BIN"] = self.enterprise_claude()
                if step.get("passed_over"):
                    env["MESIMON_MOD_DIR"] = self.passed_over_mod()
                want_probe = "hook events do not reach the mod"
            self.crown_native = road == "native"
            if step.get("idle_ms"):
                env["MESIMON_PERSON_IDLE_MS"] = str(step["idle_ms"])
            if step.get("barrier"):
                env["MESIMON_TEST_LAUNCH_BARRIER"] = os.path.join(self.out, "barrier")
            # The road is judged by the woken crown's record below, never by
            # `road.json`, which a restarted daemon rewrites only when it
            # changes its mind; a native road waits for the probe's verdict,
            # which the crown's relaunch reads.
            ok = self.restart(env, want_probe=want_probe)
            self.restarted = True
            if not ok:
                record["failures"].append(f"the daemon would not restart for the {road} road")
                return False
            # A launch before the new daemon has the person's shell
            # environment finds no `claude` to probe and takes the hook set.
            env_file = os.path.join(self.paths.rt_dir, "shellenv.env")
            if not self.wait_for("the daemon's shell environment", lambda: file_generation(
                    env_file) not in (None, self.env_before), 60, self.watch):
                record["failures"].append("the restarted daemon never captured the shell environment")
                return False
            crown = self.agent_of(self.board(), self.crown)
            self.wire.sleep(crown["id"])
            if not self.wait_for("the crown parked", lambda: word_of(
                    (self.agent_of(self.board(), self.crown) or {}).get("state")) == "sleeping",
                    60, self.watch):
                record["failures"].append("the crown would not park")
                return False
            self.wire.wake(crown["id"])
            on_road = "mod" if road == "native" else road
            on = self.wait_for(f"the crown on the {road} road", lambda: (lambda r: r and word_of(
                r["state"]) == "idle" and r.get("road", "hooks") == on_road
                and bool(r.get("native")) == (road == "native"))(
                self.agent_of(self.board(), self.crown)), 120, self.watch)
            if not on:
                rec = self.agent_of(self.board(), self.crown) or {}
                record["failures"].append(f"the crown woke on {rec.get('road', 'hooks')}"
                                          f"{' native' if rec.get('native') else ''}, "
                                          f"{word_of(rec.get('state'))}, not idle on {road}")
                return False
            # `SessionStart` fires during Claude Code's startup, before its
            # composer takes a paste: the next step's words wait for the box.
            def box_empty():
                pane = sid16(crown["id"])
                at = self.tmux("display-message", "-p", "-t", pane,
                               "#{cursor_x} #{cursor_y}").stdout.split()
                cursor = (int(at[0]), int(at[1])) if len(at) == 2 else None
                return composer_empty(self.tmux("capture-pane", "-p", "-t", pane)
                                      .stdout.splitlines(), cursor)

            if not self.wait_for("the crown's empty composer", box_empty, 60, self.watch):
                record["failures"].append("the woken crown never drew an empty composer")
                return False
            return True
        if what == "queue_in_compaction":
            # T-1069: the person's words queued while the worker's compaction
            # window is open (its `PreCompact` came, its `PostCompact` has
            # not): the board holds words against a compacting seat, and they
            # go once the window closes. The step ends at the worker's turn
            # on them and the crown's turn on the board's wake for it.
            rec = self.agent_of(self.board(), test["ticket"])
            since = record["t0_ms"]
            frames = lambda: [l for l in self.feed_lines if l.get("kind") == "hook"  # noqa: E731
                              and of_session(l, rec["id"]) and l.get("at_ms", 0) >= since]
            if not self.wait_for("the worker's PreCompact", lambda: any(
                    l["event"] == "PreCompact" for l in frames()), 120, self.watch):
                record["failures"].append("the worker's compaction never opened")
                return False
            closed = any(l["event"] == "PostCompact" for l in frames())
            try:
                r = self.wire.prompt(test["ticket"], step["text"], queued=True)
            except WireError as e:
                record["failures"].append(f"the person's queued prompt: {e}")
                return False
            record["queued_ms"] = int(time.time() * 1000)
            self.log(f"the person's prompt queued for a compacting {test['key']}"
                     f"{' (the window had closed already)' if closed else ''}: {r.get('resp')}")
            post = lambda: next((l["at_ms"] for l in frames() if l["event"] == "PostCompact"), None)  # noqa: E731
            def answered():
                at = post()
                return at is not None and any(l["event"] == "Stop" and l["at_ms"] > at for l in frames()) \
                    and word_of(self.agent_of(self.board(), test["ticket"])["state"]) == "idle"
            if not self.wait_for("the worker's turn on the held words", answered, STEP_TIMEOUT, self.watch):
                record["failures"].append("the worker never took a turn on the queued words")
                return False
            stop_at = max(l["at_ms"] for l in frames() if l["event"] == "Stop")
            wake = self.wait_for("the board's wake for the crown", lambda: next((
                l for l in self.feed_lines if l.get("kind") == "crown_wake"
                and l.get("worker") == test["ticket"] and l.get("at_ms", 0) >= stop_at - 500), None),
                WAKE_GRACE, self.watch)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            self.quiet_since = None
            after = wake["at_ms"] if wake else stop_at
            if not self.wait_for("crown", lambda: self.crown_settled(
                    after_ms=after if wake else None, quiet=0 if wake else QUIET),
                    STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle")
                return False
            return True
        if what == "crown_compact":
            # T-1069: the crown's own `/compact`, sent as a person's prompt to
            # its seat (a slash command takes the paste road), and its
            # compaction window's two frames from its mod. A compaction is no
            # turn: the step ends at the `PostCompact` and an idle crown.
            crown = self.agent_of(self.board(), self.crown)
            mark = len(self.feed_lines)
            try:
                self.wire.prompt(self.crown, "/compact", queued=False)
            except WireError as e:
                record["failures"].append(f"the crown's /compact: {e}")
                return False
            def compacted():
                mine = [l for l in self.feed_lines[mark:] if l.get("kind") == "hook"
                        and of_session(l, crown["id"])]
                rec = self.agent_of(self.board(), self.crown) or {}
                return any(l["event"] == "PostCompact" for l in mine) and not rec.get("compacting") \
                    and word_of(rec.get("state")) == "idle"
            if not self.wait_for("the crown's compaction", compacted, STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's compaction never closed")
                return False
            self.quiet_since = None
            if not self.wait_for("crown", lambda: self.crown_settled(quiet=QUIET), STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle after its compaction")
                return False
            return True
        if what == "person_idles":
            # A3 (T-945): the person stays in the crown's pane and sends it
            # no input. The board owes the crown its wake and holds it while
            # tmux's record of the client's activity is inside the bound;
            # past it, the wake goes in with the person still attached.
            tid = test["ticket"]
            mark = next((i for i, l in enumerate(self.feed_lines)
                         if l.get("at_ms", 0) >= record["person_came_ms"]), len(self.feed_lines))
            owed = self.wait_for("the board to owe the crown its wake", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == tid), None), WAKE_GRACE, self.watch)
            if not owed:
                record["failures"].append("the board never owed the crown a wake for the worker")
                return False

            def sent_in():
                return [l for l in self.feed_lines[mark:] if l.get("kind") == "board"
                        and l.get("cmd") == "crown_wake_sent" and l.get("ticket") == self.crown]

            rows = []
            deadline = time.time() + step.get("hold", 180)
            while time.time() < deadline and not sent_in():
                rows.append(next((p.get("held") for p in self.wire.snapshot().get("pending", [])
                                  if p.get("action") == "crown_wake"
                                  and p.get("ticket") == self.crown), None))
                self.watch()
                time.sleep(1)
            sent = sent_in()
            crown = self.agent_of(self.board(), self.crown) or {}
            still_in = bool(self.tmux("list-clients", "-t", sid16(crown.get("id", "")),
                                      "-F", "#{client_name}").stdout.strip())
            record["person"] = {"rows": rows, "sent_while_in": len(sent) if still_in else 0,
                                "sent_after_attach_ms":
                                sent[0]["at_ms"] - record["person_came_ms"] if sent else None}
            self.person.kill()
            self.person.wait()
            if not sent:
                record["failures"].append(f"the wake waited for the idle person (rows {rows[-1:]})")
                return False
            settled = lambda: self.crown_settled(after_ms=sent[0]["at_ms"], quiet=0)  # noqa: E731
            if not self.wait_for("crown", settled, STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle on the wake")
                return False
            self.closed()
            return True
        if what == "person_leaves":
            # A1 (T-918): with the worker's wake owed, the person stays
            # `hold` seconds, while the board must paste nothing into the
            # crown's box and its wake row must say why; then they leave,
            # and the wake goes in, and the crown takes its turn on it. On
            # the mod road (T-931) the wake goes by the mod's submit, which
            # leaves the box alone, so it goes in while the person stays.
            tid = test["ticket"]
            road = (self.agent_of(self.board(), self.crown) or {}).get("road", "hooks")
            mark = next((i for i, l in enumerate(self.feed_lines)
                         if l.get("at_ms", 0) >= record["person_came_ms"]), len(self.feed_lines))
            owed = self.wait_for("the board to owe the crown its wake", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == tid), None), WAKE_GRACE, self.watch)
            if not owed:
                record["failures"].append("the board never owed the crown a wake for the worker")
                return False

            def row():
                return next((p.get("held") for p in self.wire.snapshot().get("pending", [])
                             if p.get("action") == "crown_wake" and p.get("ticket") == self.crown),
                            None)

            def sent_in():
                return [l for l in self.feed_lines[mark:] if l.get("kind") == "board"
                        and l.get("cmd") == "crown_wake_sent" and l.get("ticket") == self.crown]

            rows = []
            deadline = time.time() + step.get("hold", 10)
            while time.time() < deadline and not (road == "mod" and sent_in()):
                rows.append(row())
                self.watch()
                time.sleep(1)
            sent_while_in = sent_in()
            by_mod = any(l.get("kind") == "board" and l.get("cmd") == "prompt_by_mod"
                         and l.get("ticket") == self.crown for l in self.feed_lines[mark:])
            record["person"] = {"road": road, "rows": rows, "sent_while_in": len(sent_while_in),
                                "by_mod": by_mod}
            self.person.kill()
            self.person.wait()
            if road == "mod":
                sent = sent_while_in[0] if sent_while_in else None
                record["person"]["sent_in_ms"] = sent["at_ms"] - owed["at_ms"] if sent else None
                if not sent:
                    record["failures"].append(f"the mod road's wake waited for the person (rows {rows[-1:]})")
                    return False
                settled = lambda: self.crown_settled(after_ms=sent["at_ms"], quiet=0)  # noqa: E731
                if not self.wait_for("crown", settled, STEP_TIMEOUT, self.watch):
                    record["failures"].append("the crown did not settle on the wake")
                    return False
                self.closed()
                return True
            left_ms = int(time.time() * 1000)
            sent = self.wait_for("the wake after the person left", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "board"
                and l.get("cmd") == "crown_wake_sent" and l.get("ticket") == self.crown
                and l.get("at_ms", 0) >= left_ms), None), WAKE_GRACE, self.watch)
            record["person"]["sent_after_ms"] = sent["at_ms"] - left_ms if sent else None
            if not sent:
                record["failures"].append(f"no wake went in after the person left (rows {rows[-1:]})")
                return False
            settled = lambda: self.crown_settled(after_ms=sent["at_ms"], quiet=0)  # noqa: E731
            if not self.wait_for("crown", settled, STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle on the wake")
                return False
            self.closed()
            return True
        if what in ("limit", "fail"):
            # L1 (T-802): the account's usage limit, as Claude Code reports
            # it — a `StopFailure` whose class is `rate_limit` — sent through
            # the real hook binary on both roads (the daemon takes the one
            # the session reports by). No real account is made to hit one.
            # E1 (T-936): `fail` is the same frame with a server error's
            # class (`class`, server_error unless said): the turn failed.
            rec = self.agent_of(self.board(), test["ticket"])
            # `live` (T-930's review): the limit ends a turn, so it lands on
            # a worker proven in one, never on one already done and parked.
            if step and step.get("live") and word_of(rec["state"]) != "working":
                record["failures"].append(f"the limit came to a worker not in a turn ({word_of(rec['state'])})")
                return False
            mark = len(self.feed_lines)
            if what == "limit":
                cls = "rate_limit"
                words = ("You've reached your Sonnet limit. Run /usage-credits to continue "
                         "or switch models with /model.")
            else:
                cls = step.get("class", "server_error")
                words = "API Error: 500 Internal server error"
            body = json.dumps({"error": cls, "last_assistant_message": words})
            for road in (["--road", "mod"], []):
                subprocess.run([self.bin, "hook", "--sock", os.path.join(self.paths.rt_dir, "hook.sock"),
                                "--session", rec["id"], "--event", "StopFailure",
                                "--reason", cls, *road],
                               input=body, text=True, capture_output=True, timeout=10)
            wake = self.wait_for(f"the board's {what} wake for the crown", lambda: next((
                l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                and l.get("worker") == test["ticket"]), None), WAKE_GRACE, self.watch)
            record["wakes"].append(wake.get("cause") if wake else "none came")
            if not wake:
                record["failures"].append(f"the board never woke the crown for the {what}")
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
            record["peer"] = {"path": self.peer.repo, "crown": self.peer.crown_key,
                              "local_board": os.path.basename(self.repo), "local_key": self.crown_key}
            self.log(f"the peer board {PEER_NAME} is up, crowned by {self.peer.crown_key}, "
                     f"and linked with this board")
            return True
        if what == "peer_ask":
            # X2: the peer's crown asks this board's crown, with nothing
            # said to it by the rig: the words reach it through the Board
            # ask template alone, and the step ends when its turn on them
            # ended.
            sent_ms = int(time.time() * 1000)
            nonce = ulid()
            words = step.get("text", "which release is tagged on your board? answer with the "
                                     "version, or that you cannot tell, and end your turn")
            words += f" Include reference {nonce} in your answer."
            record["peer"].update(nonce=nonce, incoming=(
                f'The crown of the {PEER_NAME} board ({self.peer.crown_key} "Relay release") asks: {words}'))
            try:
                r = self.peer.reply(os.path.basename(self.repo), words)
                record["peer"]["receipt"] = r
                self.log(f"the peer's crown asked: {r.get('road')}")
            except WireError as e:
                record["failures"].append(f"the peer's ask: {e}")
                return False
            if not self.wait_for("crown", lambda: self.crown_settled(after_ms=sent_ms, quiet=0),
                                 STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown did not settle on the peer's ask")
                return False
            if not self.peer_delivery(record):
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
            if not self.peer_delivery(record):
                return False
            sent_ms = int(time.time() * 1000)
            nonce = ulid()
            words = step.get("text", "relay 0.1.0-beta.2 is live; /version serves it")
            words += f" Reference {nonce}."
            record["peer"].update(nonce=nonce, incoming=f"The crown of the {PEER_NAME} board replied: {words}")
            try:
                r = self.peer.reply(os.path.basename(self.repo), words)
                record["peer"]["receipt"] = r
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
        if what in ("train_on", "train_off"):
            # HT1 (T-919): the merge train armed on this connection, and the
            # test ticket off the person's hold every test is filed under,
            # so the crown's own hold is the only one; off disarms it and
            # puts the person's hold back. The rig's base is this ticket
            # worktree's own branch, which no test may land a commit on, so
            # `block` is an untracked file at the checkout's root where the
            # worker writes the same path: git will not overwrite it, and
            # the train's fast-forward is refused (`merge_train_refused`),
            # which is the train taking the ticket and landing nothing.
            on = what == "train_on"
            block = step.get("block")
            try:
                self.wire.request({"cmd": "set_automation", "merge_train": on, "merge_notice": False})
                self.wire.set_manual_merge(test["ticket"], not on)
            except WireError as error:
                record["failures"].append(f"{what}: {error}")
                return False
            if block:
                path = os.path.join(self.repo, block)
                if on:
                    with open(path, "w") as f:
                        f.write("the rig's blocker: no train merge lands here\n")
                elif os.path.exists(path):
                    os.remove(path)
            self.log(f"merge train {'armed' if on else 'disarmed'} for {test['key']}"
                     + (f", {block} {'in the way' if on else 'removed'}" if block else ""))
            return True
        if what == "train_wait":
            # HT1 (T-919): the armed train's first word on the ticket, a
            # merge or its refusal, within a minute of the step.
            said = self.wait_for("the merge train to take the ticket", lambda: next((
                l for l in self.feed_lines if l.get("kind") == "board"
                and l.get("ticket") == test["ticket"]
                and str(l.get("cmd", "")).startswith("merge_train")), None), 60, self.watch)
            if not said:
                record["failures"].append("the merge train never took the returned ticket")
                return False
            self.log(f"the train took {test['key']}: {said.get('cmd')}")
            return True
        if what == "binary_moved":
            # DB1 (T-941): the rig daemon's binary, same bytes, its mtime a
            # second on, as a same-version rebuild leaves it. The stamp goes
            # back at `binary_back`, which runs after a failed step too, and
            # at the test's end whatever ended it (`run_test`'s finally).
            before = os.stat(self.bin)
            os.utime(self.bin, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000_000))
            self.bin_moved = (before, os.stat(self.bin))
            return True
        if what == "binary_back":
            return self.restore_binary(record)
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
            env = self.forge_env(d)
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
        if what in ("forge_merged_elsewhere", "forge_merged_running"):
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
            if what == "forge_merged_running":
                checks = os.path.join(d, "checks.json")
                with open(checks + ".tmp", "w") as f:
                    json.dump({"check_runs": [{"name": "deploy", "status": "in_progress"}]}, f)
                os.replace(checks + ".tmp", checks)
            self.write_pr(d, branch, "MERGED", "passed", head="c0ffee42" * 5)
            env = self.forge_env(d)
            self.wire.set_merge_into("pull_requests")
            if not self.restart(env):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            bound = self.wait_for("the merged PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 42 and p["state"] == "merged"
                and (what != "forge_merged_running" or p["pipeline"] == "running")
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
            env = self.forge_env(d)
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
            env = self.forge_env(d)
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
        if what in ("forge_pass", "forge_merged_pass"):
            # The pipeline passes on the forge (T-744): the board wakes the
            # crown with the news, and the step ends when the crown's turn
            # on it did; what it did is the checks'.
            d = self.forge_stub()
            mark = len(self.feed_lines)
            if what == "forge_merged_pass":
                checks = os.path.join(d, "checks.json")
                with open(checks + ".tmp", "w") as f:
                    json.dump({"check_runs": [{"name": "deploy", "status": "completed",
                                              "conclusion": "success"}]}, f)
                os.replace(checks + ".tmp", checks)
            else:
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
        if what == "forge_later_news":
            # A second PR on the merged ticket (T-1071): it opens running,
            # which is no news; its pipeline passes, which is the forge's
            # news on a branch that still reads merged; then it merges, at a
            # commit the worktree's branch does not hold, so the forge's
            # wake says it. Each wake ends at the crown's turn on it.
            d = self.forge_stub()
            later = {"number": 43, "head": "c0ffee43" * 5, "state": "OPEN"}
            self.write_pr(d, self.forge_branch, "MERGED", "passed", later=later)
            bound = self.wait_for("the second PR to bind", lambda: any(
                p["ticket"] == test["ticket"] and p["number"] == 43
                for p in self.snapshot().get("prs", [])), STEP_TIMEOUT, self.watch)
            if not bound:
                record["failures"].append("the second PR never bound to the ticket")
                return False
            for news in (dict(later, pipeline="passed"), dict(later, state="MERGED")):
                mark = len(self.feed_lines)
                self.write_pr(d, self.forge_branch, "MERGED", "passed", later=news)
                wake = self.forge_wake(test, mark)
                record["wakes"].append(wake.get("cause") if wake else "none came")
                if not wake:
                    said = "the merge" if news["state"] == "MERGED" else "the pipeline"
                    record["failures"].append(f"the board never woke the crown for {said} of #43")
                    return False
                self.quiet_since = None
                if not self.wait_for("the crown's turn on the forge's news",
                                     lambda: self.crown_settled(after_ms=wake.get("at_ms", 0)),
                                     STEP_TIMEOUT, self.watch):
                    record["failures"].append("the crown's turn on the forge's news never ended")
                    return False
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
        if what in ("forge_prs", "forge_merge_all"):
            # SM1 (T-1064): one PR per ticket of the test, 42 up, each on its
            # own branch at its tip; `forge_prs` opens them on the stub and
            # restarts the daemon onto it, `forge_merge_all` merges them all
            # at once and ends at the crown's turn on the last merge's wake.
            branches = {w["ticket"]: w["branch"] for w in self.snapshot().get("worktrees", [])}
            rows = [(42 + i, branches.get(tid)) for i, (tid, _, _) in
                    enumerate(self.test_tickets(test))]
            if not all(b for _, b in rows):
                record["failures"].append("a worker has no worktree branch to open a PR from")
                return False
            d = self.forge_stub()
            merged = what == "forge_merge_all"
            mark = len(self.feed_lines)
            self.write_prs(d, rows, "MERGED" if merged else "OPEN", "passed" if merged else "running")
            if not merged:
                self.wire.set_merge_into("pull_requests")
                if not self.restart(self.forge_env(d)):
                    record["failures"].append("the daemon would not restart with the stub forge")
                    return False
                bound = self.wait_for("every PR to bind", lambda: {
                    (p["ticket"], p["number"]) for p in self.snapshot().get("prs", [])} >= {
                    (tid, n) for (n, _), (tid, _, _) in zip(rows, self.test_tickets(test))},
                    STEP_TIMEOUT, self.watch)
                if not bound:
                    record["failures"].append("the stub forge's PRs never bound to the tickets")
                return bool(bound)
            def woken():
                got = {l.get("worker") for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
                       and l.get("cause") in ("merged", "delivered")}
                return all(tid in got for tid, _, _ in self.test_tickets(test))
            if not self.wait_for("the crown's merge wakes", woken, WAKE_GRACE * 4, self.watch):
                record["failures"].append("the board never woke the crown for every merge")
                return False
            last = max(l.get("at_ms", 0) for l in self.feed_lines[mark:]
                       if l.get("kind") == "crown_wake")
            self.quiet_since = None
            if not self.wait_for("the crown's turn on the merges",
                                 lambda: self.crown_settled(after_ms=last), STEP_TIMEOUT, self.watch):
                record["failures"].append("the crown's turn on the merges never ended")
                return False
            return True
        if what == "crown_away":
            # SM1 (T-1064): the crown taken off and put back, as a person's
            # switch away and back does; the board forgets what it told it.
            record.setdefault("step_ms", {})[what] = int(time.time() * 1000)
            try:
                self.wire.request({"cmd": "uncrown"})
                self.wire.crown(self.crown)
            except WireError as e:
                record["failures"].append(f"crown_away: {e}")
                return False
            self.log("the crown taken off and put back")
            return True
        if what == "forge_again":
            # SM1 (T-1064): a handover on the same stub forge, then long
            # enough for the flags to read every merged branch again, the
            # content scans' budget spread over several passes.
            d = self.forge_stub()
            if not self.restart(self.forge_env(d)):
                record["failures"].append("the daemon would not restart with the stub forge")
                return False
            self.restarted = True
            tids = [tid for tid, _, _ in self.test_tickets(test)]
            merged = self.wait_for("every branch read merged", lambda: all(
                any(w["ticket"] == tid and w.get("merged") for w in self.snapshot().get("worktrees", []))
                for tid in tids), STEP_TIMEOUT, self.watch)
            if not merged:
                record["failures"].append("the flags never read every branch merged after the restart")
                return False
            time.sleep(step.get("hold", 30))
            self.watch()
            return True
        if what == "crown_rest":
            # SM1 (T-1064): the crown parked and woken, as `x x` does.
            crown = self.agent_of(self.board(), self.crown)
            self.wire.sleep(crown["id"])
            if not self.wait_for("the crown parked", lambda: word_of(
                    (self.agent_of(self.board(), self.crown) or {}).get("state")) == "sleeping",
                    60, self.watch):
                record["failures"].append("the crown would not park")
                return False
            self.wire.wake(crown["id"])
            if not self.wait_for("the crown idle again", lambda: word_of(
                    (self.agent_of(self.board(), self.crown) or {}).get("state")) == "idle",
                    120, self.watch):
                record["failures"].append("the woken crown never went idle")
                return False
            time.sleep(step.get("hold", 15))
            self.watch()
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

    def checkout(self):
        """The checkout's HEAD and its working tree (T-916)."""
        from codex import checkout_state
        return checkout_state(self.repo)

    def look(self, board):
        """The checkout now, and whether the crown and an agent it started
        are each in a turn (`codex.crown_changes`, T-929): a crown off the
        board, or in any word but idle, reads as in one."""
        crown = self.agent_of(board, self.crown)
        return {"state": self.checkout(), "crown": not crown or word_of(crown["state"]) != "idle",
                "worker": any(word_of(s["state"]) in ("working", "needs_you") for s in board["sessions"]
                              if s.get("started_by") == self.crown and s["ticket"] != self.crown
                              and s["kind"] != "bash")}

    def crown_filed(self, lines):
        """The tickets the crown both filed and started an agent on, in
        `lines`, oldest first (T-916)."""
        mine = lambda cmd: [l.get("ticket") for l in lines if l.get("kind") == "board"  # noqa: E731
                            and l.get("actor") == "agent" and l.get("cmd") == cmd]
        started = set(mine("start_agent"))
        return [t for t in mine("create_ticket") if t in started]

    def forge_wake(self, test, mark):
        """The board's forge wake for the test's worker since `mark`."""
        return self.wait_for("the crown's forge wake", lambda: next((
            l for l in self.feed_lines[mark:] if l.get("kind") == "crown_wake"
            and l.get("worker") == test["ticket"] and l.get("cause") == "forge"), None),
            WAKE_GRACE * 2, self.watch)

    def forge_env(self, d):
        """The daemon's environment on the stub forge in `d`. The forge
        prefers the captured login-shell PATH over the daemon's own, so the
        stub is put first there too, as the e2e harness does (T-1046)."""
        env = {
            "PATH": d + ":" + os.environ.get("PATH", ""),
            "MSMN_GH_PRS": os.path.join(d, "prs.json"),
            "MSMN_GH_CHECKS": os.path.join(d, "checks.json"),
            "MESIMON_FORGE_EVERY_MS": "2000",
        }
        shell = os.path.join(d, "login-shell")
        with open(shell, "w") as f:
            f.write("#!/bin/sh\nPATH=" + shlex.quote(env["PATH"])
                    + "\nexport PATH\nfor command; do :; done\nexec /bin/sh -c \"$command\"\n")
        os.chmod(shell, 0o700)
        env["SHELL"] = shell
        # No provisional capture of the person's PATH on restart.
        try:
            os.unlink(os.path.join(self.paths.rt_dir, "shellenv.env"))
        except FileNotFoundError:
            pass
        return env

    def forge_stub(self):
        """A stub `gh` (T-739): `auth status` passes, `pr list` prints the
        file the test writes, `api …/check-runs` the merge commit's pipeline.
        Under the state dir, where nothing of the person's is."""
        d = os.path.join(self.paths.state_dir, "forge-stub")
        os.makedirs(d, mode=0o700, exist_ok=True)
        gh = os.path.join(d, "gh")
        with open(gh + ".tmp", "w") as f:
            f.write(GH_STUB)
        os.chmod(gh + ".tmp", 0o755)
        os.replace(gh + ".tmp", gh)
        checks = os.path.join(d, "checks.json")
        if not os.path.exists(checks):
            with open(checks, "w") as f:
                f.write('{"check_runs":[]}')
        return d

    def write_prs(self, d, rows, state, pipeline):
        """One PR per `(number, branch)` of `rows`, each at its branch's tip
        (SM1, T-1064): `write_pr`'s shape, many branches."""
        checks = {"running": '[{"__typename":"CheckRun","name":"test-linux","status":"IN_PROGRESS","conclusion":null}]',
                  "passed": '[{"__typename":"CheckRun","name":"test-linux","status":"COMPLETED","conclusion":"SUCCESS"}]'}[pipeline]
        prs = []
        for number, branch in rows:
            tip = subprocess.run(["git", "-C", self.repo, "rev-parse", branch],
                                 capture_output=True, text=True).stdout.strip()
            merge = f'{{"oid":"{tip}"}}' if state == "MERGED" else "null"
            prs.append(f'{{"number":{number},"url":"https://github.com/example/rig/pull/{number}",'
                       f'"state":"{state}","isDraft":false,"headRefName":"{branch}",'
                       f'"headRefOid":"{tip}","mergeCommit":{merge},"mergeable":"MERGEABLE",'
                       f'"reviewDecision":"REVIEW_REQUIRED","updatedAt":"2026-10-13T06:00:00Z",'
                       f'"statusCheckRollup":{checks}}}')
        path = os.path.join(d, "prs.json")
        with open(path + ".tmp", "w") as f:
            f.write("[" + ",".join(prs) + "]")
        os.replace(path + ".tmp", path)

    def write_pr(self, d, branch, state, pipeline, later=None, head=None, conflict=False):
        """PR 42 on `branch` at its tip, or at `head` (T-778), in conflict
        where `conflict` (T-785); `later` (number, head, state, and with
        `pipeline` its checks, T-1071) a newer PR on the same branch in
        front of it (T-757)."""
        tip = head or subprocess.run(["git", "-C", self.repo, "rev-parse", branch],
                                     capture_output=True, text=True).stdout.strip()
        rollups = {
            "running": '[{"__typename":"CheckRun","name":"test-linux","status":"IN_PROGRESS","conclusion":null}]',
            "failed": '[{"__typename":"CheckRun","name":"test-linux","status":"COMPLETED","conclusion":"FAILURE"}]',
            "passed": '[{"__typename":"CheckRun","name":"test-linux","status":"COMPLETED","conclusion":"SUCCESS"}]',
        }
        rollup = rollups[pipeline]
        def pr(number, head, state, at, own=None):
            merge = f'{{"oid":"{head}"}}' if state == "MERGED" else "null"
            # The later PR opens running: its opening is no pipeline news.
            checks = rollups["running"] if state == "OPEN" and number != 42 else rollup
            if own:
                checks = rollups[own]
            mergeable = "CONFLICTING" if conflict and number == 42 else "MERGEABLE"
            return (f'{{"number":{number},"url":"https://github.com/example/rig/pull/{number}",'
                    f'"state":"{state}","isDraft":false,"headRefName":"{branch}",'
                    f'"headRefOid":"{head}","mergeCommit":{merge},"mergeable":"{mergeable}",'
                    f'"reviewDecision":"REVIEW_REQUIRED","updatedAt":"{at}",'
                    f'"statusCheckRollup":{checks}}}')
        prs = [pr(42, tip, state, "2026-10-13T06:00:00Z")]
        if later:
            prs.insert(0, pr(later["number"], later["head"], later["state"], "2026-10-13T07:00:00Z",
                             later.get("pipeline")))
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
                  "t0_ms": int(t0 * 1000), "checkout0": self.checkout(), "keys": []}
        trace = self.seams.get("MESIMON_RECORD_TRACE")
        trace_at = os.path.getsize(trace) if trace and os.path.exists(trace) else 0
        # The crown is asked at once: the first span is its own.
        self.looks = ([{"state": record["checkout0"], "crown": True, "worker": False}]
                      if any(c.startswith("crown_wrote_nothing") for c in test.get("checks", [])) else None)
        ok = True
        budget = self.board()["crown_budget"] if "budget" in test else None
        try:
            if budget is not None:
                self.wire.set_crown_budget(test["budget"])
            for step in test["steps"]:
                if ok:
                    ok = self.run_step(test, step, record)
                elif step.get("rig") in ("restart_plain", "binary_back"):
                    # A failed R7 still gives the board back its own Claude
                    # Code, and a failed DB1 the binary its own stamp.
                    self.run_step(test, step, record)
        finally:
            # An exception skips the steps above: DB1's stamp goes back here.
            ok = self.restore_binary(record) and ok
            if budget is not None:
                self.wire.set_crown_budget(budget)
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
        if trace:
            self.cut_trace(test, trace, trace_at, verdict)
        self.record(test, verdict)
        self.current = self.looks = None
        return verdict

    # ---- traces (T-1014)

    def cut_trace(self, test, trace, at, verdict):
        """The test's workers' lines of the recorder's file since the test
        began (`--record`): kept whole as `traces/<id>.raw.jsonl` (private:
        paths, prompts, the engine's request blobs) and cut into
        `traces/<id>.jsonl`, the fixture an e2e replays (`fixture_of`)."""
        recs = [r for r in (self.agent_of(self.board(), tid)
                            for tid, _, _ in self.test_tickets(test)) if r]
        ids = {r["id"] for r in recs} | {sid16(r["id"]) for r in recs}
        with open(trace, "rb") as f:
            f.seek(at)
            raw = [json.loads(l) for l in f.read().decode("utf-8", "replace").splitlines()
                   if l.strip()]
        header = {"trace": test["id"], "claude": self.claude_version,
                  "road": recs[0].get("road", "hooks") if recs else None,
                  "model": verdict.get("model"), "recorded": time.strftime("%Y-%m-%d"),
                  "by": f"python3 -B ci/rig.py --record --only {test['id']}",
                  "verdict": "PASS" if verdict["pass"] else "FAIL"}
        mine = [header] + [l for l in raw if l.get("session") in ids]
        d = os.path.join(self.out, "traces")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, f"{test['id']}.raw.jsonl"), "w") as f:
            f.writelines(json.dumps(l) + "\n" for l in mine)
        path = os.path.join(d, f"{test['id']}.jsonl")
        with open(path, "w") as f:
            f.writelines(json.dumps(l, ensure_ascii=False) + "\n"
                         for l in fixture_of(mine, self.repo, self.paths.state_dir))
        say(f"  trace: {len(mine) - 1} lines → {path}")

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
        crown = self.agent_of(self.board(), self.crown)
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
            "crown": crown_measures(transcript(crown), record["t0_ms"]) if crown else None,
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
            if rec is None and name not in ("woke", "no_wake_after", "crown_hand", "crown_reply", "crown_line", "crown_obstacle",
                                            "crown", "crown_board", "peer_fed", "board_exchange", "filed",
                                            "crown_wrote_nothing", "title_words", "title_never",
                                            "title_is", "title_match", "described", "create_start"):
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
            elif name == "board_exchange":
                crown = self.agent_of(board, self.crown)
                crown_rows = [r for r in (transcript(crown) if crown else [])
                              if (row_ms(r) or 0) >= record["t0_ms"]]
                check(c, *board_exchange(crown_rows, arg, record.get("peer", {})))
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
            elif name == "mcp_refused":
                try:
                    with open(test["mcp_probe_receipt"]) as f:
                        receipt = json.load(f)
                    response = receipt["response"]
                    check(c, receipt["unchanged"] and response.get("resp") == "err"
                          and "not applied" in response.get("message", "")
                          and "raise_hand" in response.get("message", ""), str(response))
                except (OSError, KeyError, ValueError) as error:
                    check(c, False, str(error))
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
                # T-973, rounds 1-2: every tool's input but the board's own
                # (`mcp__mesimon__` alone), so
                # a Read, Grep or Glob of a transcript counts as a Bash `cat`.
                hand = [f"<{n}> {json.dumps(i)}" for n, i in calls
                        if not (n or "").startswith("mcp__mesimon__") and n != "Bash"]
                bad = [r for r in ran + hand if any(w in r for w in words)]
                bad += [f"<{n}>" for n, _ in calls if n == "Monitor"]
                check(c, not bad, " → ".join(repr(b[:60]) for b in bad)
                      or f"{len(ran)} bash calls, {len(hand)} other tool calls, none of {words}")
            elif name == "compacted":
                # T-969, round 1: the worker's own transcript records a
                # compaction of trigger `arg` with a reply on each side of
                # it, both readings in the observed line. Which way the
                # reading moves is not judged: on a short conversation the
                # system prompt and tools are most of it (49,282 to 50,651
                # measured), on a long one it drops by the hundred thousand.
                def fill_of(r):
                    u = (r.get("message") or {}).get("usage") or {}
                    return (u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0)
                            + u.get("cache_creation_input_tokens", 0))
                at = next((i for i, r in enumerate(rows) if r.get("type") == "system"
                           and r.get("subtype") == "compact_boundary"
                           and (r.get("compactMetadata") or {}).get("trigger") == arg), None)
                main = [(i, fill_of(r)) for i, r in enumerate(rows) if r.get("type") == "assistant"
                        and not r.get("isSidechain") and fill_of(r) > 0]
                before = [f for i, f in main if at is not None and i < at]
                after = [f for i, f in main if at is not None and i > at]
                if at is None:
                    check(c, False, f"no {arg} compact_boundary in the transcript")
                elif not before or not after:
                    check(c, False, f"boundary at row {at}; replies before {before[-1:]}, after {after[:1]}")
                else:
                    check(c, True,
                          f"{arg} boundary; last reply before read {before[-1]}, first after {after[0]}")
            elif name == "context_nearer":
                # T-969: the crown read each of the test's workers' context
                # off get_ticket since the test began, and its last line
                # beginning `arg` names the one whose reading there stood
                # nearest its window.
                crown = self.agent_of(board, self.crown)
                since = record.get("t0_ms", 0)
                crown_rows = [r for r in (transcript(crown) if crown else [])
                              if (iso_ms(r.get("timestamp") or "") or 0) >= since]
                keys = [k for _, k, _ in self.test_tickets(test)]
                read = {}
                for n, i, text in tool_uses(crown_rows):
                    if (n or "").endswith("get_ticket") and i.get("key") in keys:
                        got = context_read(text)
                        if got:
                            read[i["key"]] = got
                line = ""
                for r in reversed(replies(transcript(crown)) if crown else []):
                    said = [l.strip().strip("*`").strip() for l in r.splitlines()]
                    at = next((j for j in reversed(range(len(said)))
                               if said[j].startswith(arg)), None)
                    if at is not None:
                        line = " ".join(l for l in said[at:] if l)
                        break
                if len(read) < len(keys):
                    check(c, False, f"context read off get_ticket for {sorted(read)} of {keys}")
                else:
                    nearest = max(keys, key=lambda k: read[k][0] / read[k][1])
                    others = [k for k in keys if k != nearest]
                    ok = nearest in line and not any(k in line for k in others)
                    check(c, ok, f"read {read}; said {line[:120]!r}" if line
                          else f"read {read}; no line {arg!r}")
            elif name in ("title_words", "title_never", "title_is", "title_match", "described"):
                # T-1073: the tickets the crown filed since the test began,
                # judged by title or description. `title_words:<n>` is this case's
                # bound in words, not a product's; the others read words, the
                # exact title, a regex, and the description's words.
                filed = [self.ticket(board, t) for t in dict.fromkeys(
                    l.get("ticket") for l in lines if l.get("kind") == "board"
                    and l.get("actor") == "agent" and l.get("cmd") == "create_ticket")]
                filed = [t for t in filed if t]
                words_of = [w for w in arg.split(",") if w]
                bad = []
                for t in filed:
                    tt = t["title"]
                    if name == "title_words":
                        ok = len(tt.split()) <= int(arg)
                    elif name == "title_never":
                        ok = not any(w.lower() in tt.lower() for w in words_of)
                    elif name == "title_is":
                        ok = tt == arg
                    elif name == "title_match":
                        ok = re.search(arg, tt) is not None
                    else:
                        notes = t.get("notes") or []
                        body = self.wire.read_note(t["id"], notes[0]["id"]) if notes else ""
                        ok = all(w.lower() in body.lower() for w in words_of)
                    if not ok:
                        bad.append(t)
                said = "; ".join(f"{t['short_key']} {t['title']!r} ({len(t['title'].split())} words)"
                                 for t in filed)
                check(c, bool(filed) and not bad, said or "the crown filed nothing")
            elif name == "filed":
                # T-916: every ticket the crown started an agent on was filed
                # by the crown's own session from its ticket, its agent is the
                # one seat there and took a turn, and it works where the
                # ticket's workspace puts it: its own worktree, or the shared
                # checkout, the crown's choice (T-929).
                from codex import place_problems, seat_problems
                crown_rec = self.agent_of(board, self.crown)
                trees = {w["ticket"]: w for w in self.snapshot().get("worktrees", [])}
                good, bad = [], []
                starts = [l.get("ticket") for l in lines if l.get("kind") == "board"
                          and l.get("actor") == "agent" and l.get("cmd") == "start_agent"]
                for t in dict.fromkeys(starts):
                    a, tk, tree = self.agent_of(board, t), self.ticket(board, t), trees.get(t)
                    why = []
                    if tk.get("created_by") != f"agent:{crown_rec and crown_rec['id']}" \
                            or tk.get("created_from") != self.crown:
                        why.append(f"filed by {tk.get('created_by')!r}")
                    if not a or a.get("started_by") != self.crown:
                        why.append("no agent the crown started")
                    else:
                        why += seat_problems(board["sessions"], a)
                        why += place_problems(a["cwd"], self.repo, tree, shared=True)
                        if not any(r.get("type") == "user" for r in transcript(a)):
                            why.append("took no turn")
                    where = f" in {tree.get('branch')}" if tree else " on the shared checkout"
                    (bad if why else good).append(f"{tk['short_key']} {tk['title'][:50]!r}"
                                                  + (": " + ", ".join(why) if why else where))
                # Every start the crown made must hold: one good start
                # beside a bad one fails, as the Codex rig's does.
                check(c, bool(good) and not bad, "; ".join(good + bad) or "the crown filed and started nothing")
            elif name == "crown_wrote_nothing":
                # T-916: the work went to the filed ticket's agent, not the
                # crown's: no Write, Edit or NotebookEdit, no Bash command that
                # writes naming the work (`codex.command_writes`), and no move
                # of the checkout's HEAD or tree that is the crown's
                # (`codex.crown_changes`: a span the crown was idle across and
                # its agent working is that agent's, T-929), a tip the board
                # merged from a worktree aside.
                from codex import board_tips, command_writes, crown_changes
                names = [n for n in arg.split(",") if n]
                crown = self.agent_of(board, self.crown)
                since = record.get("t0_ms", 0)
                crown_rows = [r for r in (transcript(crown) if crown else [])
                              if (iso_ms(r.get("timestamp") or "") or 0) >= since]
                calls = tool_inputs(crown_rows)
                signs = [f"{n} {i.get('file_path', '')}" for n, i in calls if n in ("Write", "Edit", "NotebookEdit")]
                signs += [f"Bash {i.get('command', '')[:80]!r}" for n, i in calls
                          if n == "Bash" and command_writes(i.get("command", ""), names)]
                merged = board_tips(self.repo, self.snapshot().get("worktrees"))
                looks = self.looks or [{"state": record["checkout0"], "crown": True, "worker": False}]
                signs += crown_changes(looks + [self.look(board)], merged)
                check(c, not signs, "; ".join(signs) or f"no write of the crown's; {len(calls)} calls, "
                      "checkout as the test found it")
            elif name == "fed":
                # `fed:<cmd>=N` (T-1071): exactly N of them, as `crown:`.
                cmd, _, times = arg.partition("=")
                n = fed.count(cmd)
                check(c, n == int(times) if times else n > 0, f"{cmd}×{n}")
            elif name in ("steered", "held_steer"):
                check(c, *steer_verdict(name, arg, lines, tid, user_prompts(rows)))
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
            elif name == "native":
                # T-1069: the native road, as the daemon launched it (T-658):
                # the mod and the one-entry hook set beside it.
                ok = rec.get("native") is True and "--plugin-dir" in rec["argv"] and "--settings" in rec["argv"]
                check(c, ok, f"native {rec.get('native')}; --plugin-dir "
                      f"{'in' if '--plugin-dir' in rec['argv'] else 'not in'} argv, --settings "
                      f"{'in' if '--settings' in rec['argv'] else 'not in'} argv")
            elif name == "compaction_frames":
                got = compaction_frames([l for l in mine if l.get("kind") == "hook"])
                check(c, got == arg.split(","), " → ".join(got) or "none")
            elif name == "held_by_compaction":
                ok, seen = held_by_compaction([l for l in mine if l.get("kind") == "hook"],
                                              record.get("queued_ms"), user_prompts(rows, at=True), arg)
                check(c, ok, seen)
            elif name == "crown_compaction_frames":
                crown = self.agent_of(board, self.crown)
                got = compaction_frames([l for l in lines if crown and l.get("kind") == "hook"
                                         and of_session(l, crown["id"])])
                check(c, got == arg.split(","), " → ".join(got) or "none")
            elif name == "crown_rules_after_compact":
                crown = self.agent_of(board, self.crown)
                ok, seen = rules_after_compaction(transcript(crown) if crown else [], record["t0_ms"])
                check(c, ok, seen)
            elif name == "mod_report":
                event, _, reason = arg.partition(":")
                seen = [l for l in mine if l.get("kind") == "hook" and l.get("event") == event
                        and (not reason or l.get("reason") == reason)]
                check(c, seen, f"{event}" + (f" ({reason})" if reason else "") + f"×{len(seen)}")
            elif name == "prompts":
                got = user_prompts(rows)
                check(c, len(got) == int(arg), f"{len(got)} prompts: "
                      + "; ".join(repr(p[:30]) for p in got))
            elif name == "prompt_once":
                # T-961: the person's words reached the worker as one prompt
                # of their own, exactly once, whole: a user prompt equal to
                # <words>, and no other that holds them (doubled, or joined
                # to a title or a draft). The one normalisation is
                # `user_prompts`' trim of the text's surrounding whitespace.
                prompts = user_prompts(rows)
                exact = [p for p in prompts if p == arg]
                holding = [p for p in prompts if arg in p]
                check(c, len(exact) == 1 and len(holding) == 1,
                      f"{len(exact)} exact, {len(holding)} holding the words: "
                      + "; ".join(repr(p[:60]) for p in holding))
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
            elif name == "no_file":
                # T-932: the job a worker filed for a session of its own is
                # not done in its own cwd.
                there = os.path.exists(os.path.join(rec["cwd"], arg))
                check(c, not there, f"{arg} written here" if there else f"no {arg}")
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
            elif name == "reply_never":
                # T-930 (WR1): the worker's last reply names none of the
                # words, which are the board's own doings.
                said = last_reply(rows)
                hit = [w for w in arg.split(",") if w.lower() in said.lower()]
                check(c, not hit, f"names {', '.join(hit)}: {said[:80]!r}" if hit
                      else f"said {said[:80]!r}")
            elif name == "shell_workflow":
                said = last_reply(rows)
                try:
                    answer, _ = json.JSONDecoder().raw_decode(said[said.index("{"):])
                except (ValueError, TypeError):
                    answer = {}
                expected = {"evidence": "review-evidence-904", "capture_needs_separate_send": True,
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
            elif name == "fed_after":
                # `fed_after:<cmd>:<prefix>` (T-919): the ticket's first board
                # line whose cmd begins <prefix> comes after its first <cmd>
                # line, and none before it (HT1: the train takes nothing of
                # a held ticket, and takes it once returned).
                first, _, prefix = arg.partition(":")
                at = next((i for i, f in enumerate(fed) if f == first), None)
                hit = next((i for i, f in enumerate(fed) if str(f).startswith(prefix)), None)
                check(c, at is not None and hit is not None and hit > at,
                      f"{first} at {at}, first {prefix}* at {hit} ({fed[hit] if hit is not None else 'none'})")
            elif name == "crown_obstacle":
                # T-930's review (F3): the crown's hands or its last words
                # name the merge's owner or what blocks it, any of `arg`'s
                # words, case aside; no call is required of it.
                crown = self.agent_of(board, self.crown)
                rows_c = transcript(crown) if crown else []
                hands = [i.get("reason", "") for n, i in tool_inputs(rows_c) if (n or "").endswith("raise_hand")]
                words = [w.lower() for w in arg.split(",")]
                said = [t for t in hands + [last_reply(rows_c)] if any(w in t.lower() for w in words)]
                check(c, said, repr(said[0][:100]) if said else f"no hand or line names {arg}")
            elif name == "crown_reply":
                # The crown's last words on the test (T-774), `arg` within.
                crown = self.agent_of(board, self.crown)
                said = last_reply(transcript(crown)) if crown else ""
                check(c, arg.lower() in said.lower(), f"said {said[:100]!r}")
            elif name == "went_past_idle_person":
                # A3 (T-945): the wake row named the bound while the client
                # was fresh, and the wake went in once, the person still
                # attached, no sooner than the bound after their attach and
                # within 10 s past it (`arg`: the bound in seconds).
                p = record.get("person") or {}
                rows = [r for r in p.get("rows") or [] if r]
                secs = int(arg)
                span = f"{secs // 60}m" if secs % 60 == 0 else f"{secs}s"
                words = f"leave or {span} idle"
                after = p.get("sent_after_attach_ms")
                bound = int(arg) * 1000
                ok = (bool(rows) and all(r == words for r in rows) and p.get("sent_while_in") == 1
                      and after is not None and bound <= after <= bound + 10_000)
                check(c, ok, f"rows {sorted(set(map(str, rows)))}, {p.get('sent_while_in')} sent "
                             f"while in, {after} ms after the attach")
            elif name == "held_for_person":
                # A1 (T-918): the wake waited the whole of the person's stay,
                # its row said `when you leave` throughout, and it went in
                # within seconds of their leaving. On the mod road (T-931)
                # it went by the mod's submit while they stayed, within
                # seconds of being owed, its row never saying it waited.
                p = record.get("person") or {}
                rows = p.get("rows") or []
                if p.get("road") == "mod":
                    took = p.get("sent_in_ms")
                    unheld = not any(r and (r.startswith("leave or ") or r.startswith("idle unknown")
                                            or r == "after your draft") for r in rows)
                    check(c, unheld and p.get("by_mod") and took is not None and took < 10_000,
                          f"mod road: rows {sorted(set(map(str, rows)))}, by the mod {p.get('by_mod')}, "
                          f"in {took} ms of being owed, the person still in")
                else:
                    after = p.get("sent_after_ms")
                    held = bool(rows) and all(r == WHEN_YOU_LEAVE for r in rows)
                    check(c, held and p.get("sent_while_in") == 0 and after is not None and after < 10_000,
                          f"rows {sorted(set(map(str, rows)))}, {p.get('sent_while_in')} sent while in, "
                          f"in {after} ms after leaving")
            elif name == "crown_line":
                # The crown's last line beginning `prefix` (T-810), `word`
                # within it, or with `!word` not: one test asks the crown
                # twice and reads each answer by its own line.
                # The line runs to the end of its reply: a crown may put
                # the prefix on a line of its own and its answer under it.
                prefix, _, word = arg.rpartition(":")
                # `{reply}` (T-991, D7): the word the worker chose, its last
                # reply, which no board text names before the crown's line.
                if word == "{reply}":
                    word = last_reply(transcript(rec)).strip().strip("*`.").strip() if rec else ""
                    word = word or "\0"
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
                    # Any of a comma list: merge_refusal names the forge by its
                    # name ("merge it on GitHub", T-810), not the word "forge".
                    ok = any(w.lower() in line.lower() for w in word.split(","))
                check(c, ok, f"said {line[:120]!r}" if line else f"no line {prefix!r}")
            elif name == "plan_read":
                # T-991 (D7): the head, then the whole with plan, then the
                # accept, the word the worker's own last reply.
                crown = self.agent_of(board, self.crown)
                word = last_reply(transcript(rec)).strip().strip("*`.").strip() if rec else ""
                ok, seen = plan_read_order(transcript(crown) if crown else [], key, word)
                check(c, ok, seen)
            elif name == "second_ask":
                # `second_ask:refused_outstanding` (T-938, round 3): read off
                # the crown's own call results, so a stale-seen refusal fails.
                crown = self.agent_of(board, self.crown)
                ok, seen = second_ask_refusal(transcript(crown) if crown else [], key)
                check(c, ok and arg == "refused_outstanding", seen)
            elif name == "budget_reused":
                crown = self.agent_of(board, self.crown)
                since = record.get("t0_ms", 0)
                crown_rows = [r for r in (transcript(crown) if crown else [])
                              if (iso_ms(r.get("timestamp") or "") or 0) >= since]
                keys = self.keys_of(test)
                ok, seen = budget_reused(crown_rows, keys["key"], keys["key2"])
                check(c, ok, seen)
            elif name == "create_start":
                # `create_start:accepted` (T-1072): the crown's own calls,
                # so a read between the filing and the start fails too.
                crown = self.agent_of(board, self.crown)
                ok, seen = create_start_outcome(transcript(crown) if crown else [])
                check(c, ok and arg == "accepted", seen)
            elif name == "receipt_ask":
                # `receipt_ask:accepted` (T-1035): read off the crown's own
                # calls and results, so a re-read before the ask fails too.
                crown = self.agent_of(board, self.crown)
                ok, seen = receipt_ask_outcome(transcript(crown) if crown else [], key)
                check(c, ok and arg == "accepted", seen)
            elif name == "woke":
                causes = [l.get("cause") for l in lines if l.get("kind") == "crown_wake"
                          and l.get("worker") == tid]
                check(c, arg in causes, f"wakes {causes}" if causes else "no wake")
            elif name == "no_wake_after":
                # `no_wake_after:<rig step>` (SM1, T-1064): no crown wake for
                # the ticket from that step's start on.
                since = record.get("step_ms", {}).get(arg)
                late = [l.get("cause") for l in lines if l.get("kind") == "crown_wake"
                        and l.get("worker") == tid and since is not None
                        and l.get("at_ms", 0) >= since]
                check(c, since is not None and not late,
                      f"no {arg} step ran" if since is None else
                      f"woken after {arg}: {late}" if late else f"no wake after {arg}")
            elif name == "sampled_before":
                # `sampled_before:<prefix>` (T-973): see `sampled_before`.
                crown = self.agent_of(board, self.crown)
                since = record.get("t0_ms", 0)
                crown_rows = [r for r in (transcript(crown) if crown else [])
                              if (iso_ms(r.get("timestamp") or "") or 0) >= since]
                ok, seen = sampled_before(crown_rows, key, arg)
                check(c, ok, seen)
            elif name == "after_wake":
                # `after_wake:<cmd>:<cause>` (T-973): the crown's first <cmd>
                # board line for the ticket follows the board's first wake of
                # <cause> for it; the wake turn's last line is the crown's
                # judgement, so a read here came before that judgement.
                cmd, _, cause = arg.partition(":")
                woke_at = next((i for i, l in enumerate(lines) if l.get("kind") == "crown_wake"
                                and l.get("worker") == tid and l.get("cause") == cause), None)
                read_at = next((i for i, l in enumerate(lines) if l.get("kind") == "board"
                                and l.get("actor") == "agent" and l.get("ticket") == tid
                                and l.get("cmd") == cmd), None)
                check(c, woke_at is not None and read_at is not None and read_at > woke_at,
                      f"{cause} wake at {woke_at}, first {cmd} at {read_at}")
            elif name == "reviewed":
                # T-859: the feed says PR <arg> bound to the ticket as
                # someone else's, reviewed here. The feed, not the snapshot:
                # `forge_off` has put the board back on main by now.
                said = [str(l.get("outcome", "")) for l in lines if l.get("kind") == "board"
                        and l.get("actor") == "forge" and l.get("cmd") == "pr"
                        and l.get("ticket") == tid]
                check(c, any(f"#{arg} bound, reviewed here" in s for s in said),
                      "; ".join(said) or "the forge said nothing of the ticket")
            elif name == "held_between":
                # T-988: from the person's key <n> in the pane to key <n+1>,
                # the card read needs-you and nothing else, polls and feed.
                n = int(arg)
                keys = record.get("keys", [])
                if len(keys) <= n:
                    check(c, False, f"{len(keys)} presses recorded")
                else:
                    a, b = keys[n - 1], keys[n]
                    # The word the card already said at the first press, and
                    # every word it said after it until the next: a card that
                    # never changed says one word.
                    said = sorted(self.history.get(key, []))
                    seen = [w for t, w in said if t < a][-1:] + [w for t, w in said if a <= t < b]
                    check(c, seen and all(w == "needs_you" for w in seen),
                          f"{(b - a) / 1000:.1f} s between the presses: "
                          + (" → ".join(squeeze(seen)) or "no word seen"))
            elif name == "left_after":
                # T-957, T-988: the card left needs-you within <secs> of the
                # person's key <n>, and a decision of <source> came between
                # the key and that change: the dialog's own result, never a
                # later timer.
                n, secs, source = arg.split(":")
                keys = record.get("keys", [])
                if len(keys) < int(n):
                    check(c, False, f"{len(keys)} presses recorded")
                else:
                    at = keys[int(n) - 1]
                    left = next((l for l in mine if l.get("kind") == "session_state"
                                 and l.get("at_ms", 0) >= at and feed_word(l) != "needs_you"), None)
                    between = [l.get("source") for l in mine if l.get("kind") == "state_decision"
                               and at <= l.get("at_ms", 0) <= (left or {}).get("at_ms", at)]
                    if left is None:
                        check(c, False, "the card never left needs-you after the press")
                    else:
                        took = (left["at_ms"] - at) / 1000
                        check(c, took <= float(secs) and source in between,
                              f"{feed_word(left)} {took:.1f} s after the press; decided by "
                              + (", ".join(dict.fromkeys(s for s in between if s)) or "nothing"))
            elif name == "one_message":
                # T-988: <n> calls of <tool> under one message id, as the
                # engine writes the blocks of one message, a line each.
                tool, _, n = arg.partition(":")
                by = {}
                for row in once(rows):
                    m = row.get("message") or {}
                    content = m.get("content")
                    for b in content if isinstance(content, list) else []:
                        if isinstance(b, dict) and b.get("type") == "tool_use" and b.get("name") == tool:
                            by.setdefault(m.get("id"), []).append(b.get("id"))
                most = max((len(v) for v in by.values()), default=0)
                check(c, most == int(n), f"{sum(len(v) for v in by.values())} {tool} calls over "
                      f"{len(by)} messages, at most {most} in one")
            elif name == "never_in_prompts":
                # T-987: no prompt of the worker's holds <words>.
                got = [p for p in user_prompts(rows) if arg in p]
                check(c, not got, f"{len(got)} prompts hold {arg!r}" if got
                      else f"no prompt holds {arg!r}")
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
        say(f"    {measures_line(v['crown'])}")
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
                 f"**Its mod at the end:** {v['alive'] or 'no pane to ask'}.",
                 f"**What the crown spent:** {measures_line(v['crown'])}."]
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
            observed += f" ∙ ticket in {v.get('column')} ∙ {measures_line(v.get('crown'))}"
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
        # T-1069: whether the last `crown_road` put the daemon on the native road.
        self.crown_native = False
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
    ap.add_argument("--crown-model", default=CROWN_MODEL,
                    help=f"the Claude rig crown's model (default {CROWN_MODEL}; workers stay on {TIER_MODEL})")
    ap.add_argument("--crown-effort", default=CROWN_EFFORT,
                    help=f"the Claude rig crown's effort (default {CROWN_EFFORT})")
    ap.add_argument("--flags-off", action="store_true",
                    help="Claude Code's flags service off for the rig's sessions (T-598's seam)")
    ap.add_argument("--record", action="store_true",
                    help="record each test's frames and transcript lines (T-1014) into "
                         "target/rig/traces/<id>.jsonl, an e2e fixture")
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

    try:
        tests = load_tests(os.path.join(HERE, "rig", "tests.toml"))
    except ValueError as e:
        die(str(e))
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
