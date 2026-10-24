"""Sequential native Codex acceptance through the production daemon.

Each case owns a disposable board and Codex home through test_guard.py. The
native terminal submits prompts; app-server RPC is used only for observations.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
SMOKE = ("complete", "mcp", "brief", "queue", "sleep-resume", "handover")
EXTENDED = ("guard", "plan", "plan-launch", "interrupt", "steer", "child", "stop", "compact",
            "identity", "question", "delegate", "forge")
TURNS = {"complete": 1, "mcp": 1, "brief": 2, "queue": 2, "sleep-resume": 2,
         "handover": 2, "guard": 2, "plan": 2, "plan-launch": 2, "interrupt": 1, "steer": 2, "child": 2,
         "stop": 2, "compact": 3, "identity": 2, "question": 3, "delegate": 1, "forge": 1}
LIFECYCLE = {"stop", "compact", "identity", "question"}
# T-916: a crowned Codex seat asked one thing by the person, in words a person
# would use. Everything after that one submission is the crown's own doing and
# the board's wakes, so these cases also bound the native turns every thread of
# the fixture takes (children included), and take a longer deadline.
CROWN = {"delegate", "forge"}
NATIVE_TURNS = {"delegate": 20, "forge": 24}
TIMEOUTS = {"delegate": 300, "forge": 300}
# Observed tokens, cached input included, one crown case may use. A Codex
# crown re-sends a ~60k context on every code-mode request, so a forge case's
# four turns read about 1.5M, 90 % of it cached (measured on Luna, T-916).
CASE_TOKENS = {"delegate": 1000000, "forge": 2000000}
# Model-created children stay off except where a case measures them, or (the
# delegate case) where Codex's own default (`multi_agent` on) is the competing
# road the crown could take instead of a board ticket.
CHILDREN = {"child", "delegate"}
# The person's one request in the delegate case: concrete work, no tool named,
# and the outcome the person wants said (T-930's review: whether to delegate
# is otherwise the crown's judgement, so doing it itself is no failure).
DELEGATE_ASK = ("Have greet.py done on a ticket of its own, by its own agent: a small Python "
                "module with a function greet(name) that returns \"Hello, <name>!\", and a "
                "unittest for it in test_greet.py.")
FORGE_WORKER = "Append the line forge fixture to README.md, then reply done."
FORGE_ASK = "Please start {key} in its own worktree and look after its pull request. If its CI fails, get it fixed."
BOARD_TOOLS = ("get_ticket", "list_board", "read_note", "write_note", "create_ticket", "tag_ticket",
               "raise_hand", "move_ticket", "start_agent", "ask_agent", "answer_agent", "accept_plan",
               "sleep_agent", "merge_ticket", "watch_ticket", "set_workspace", "rename_ticket",
               "archive_ticket", "read_attachment", "read_shell", "ask_board", "request_close_shell")
# A wait on a pull request that holds the session busy (T-895), in a command's
# words; Codex's own sleep tool is the same wait by another name. One read of
# the pull request (which job failed) is not a wait; the same read again in
# the turn is (`polling`).
POLL_WORDS = ("--watch", "sleep ", "run watch")
FORGE_READ = re.compile(r"\b(?:gh|glab)\s+(?:pr\s+(?:view|checks)|ci\s+(?:status|view|list)|run\s+(?:view|list))"
                        r"[^\"'\n;&|]*")


class Inconclusive(Exception):
    """A case that ran and proved neither way: the runner records it as
    inconclusive (any exception but an AssertionError)."""


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "ci" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def add_arguments(parser):
    parser.add_argument("--live", action="store_true", help="authorize Codex model calls")
    parser.add_argument("--model", help="Codex model, checked against authenticated model/list")
    parser.add_argument("--effort", help="Codex effort, checked before submission (default: low)")
    parser.add_argument("--suite", choices=("smoke", "extended"), default="smoke")
    parser.add_argument("--max-turns", type=int, help="Codex suite turn allowance (default: 12)")
    parser.add_argument("--max-tokens", type=int,
                        help="stop at observed native total tokens; not a billing cap")
    parser.add_argument("--case-timeout", type=int)


def resolve_options(args, previous):
    # case_timeout None is each case's own (TIMEOUTS, else 180 s).
    # max_tokens None is 600000, raised to cover the selected crown cases.
    defaults = dict(model="gpt-6-luna", effort="low", max_turns=12, max_tokens=None, case_timeout=None)
    for name, default in defaults.items():
        if getattr(args, name) is None:
            setattr(args, name, (previous or {}).get(name, default) if args.failed else default)


def case_timeout(args, case):
    return args.case_timeout or TIMEOUTS.get(case, 180)


def select_cases(args, previous):
    available = SMOKE + EXTENDED
    selected = list(SMOKE if args.suite == "smoke" else available)
    if args.only:
        selected = args.only.split(",")
        unknown = set(selected) - set(available)
        if unknown or len(selected) != len(set(selected)):
            raise ValueError("unknown or duplicate Codex cases: " + args.only)
    if args.failed:
        if previous is None:
            raise ValueError("no previous Codex verdicts for --failed")
        failed = [row["case"] for row in previous["cases"] if row["outcome"] != "passed"]
        selected = [case for case in failed if case in selected] if args.only else failed
    # 900 s at most (T-930): Sol's forge case outlasts the 300 s its Luna runs
    # fit, and the outer supervisor's 20 minutes still covers one such case.
    if (args.case_timeout is not None and not 1 <= args.case_timeout <= 900) \
            or args.max_turns < 1 or (args.max_tokens is not None and args.max_tokens < 1):
        raise ValueError("positive budgets and --case-timeout 1..900 are required")
    if args.max_tokens is None:
        args.max_tokens = max(600000, sum(CASE_TOKENS.get(case, 0) for case in selected))
    needed = sum(TURNS[case] for case in selected)
    if needed > args.max_turns:
        raise ValueError(f"selected cases allow {needed} turns; --max-turns is {args.max_turns}")
    return selected


def choose_model(pages, model, effort):
    models = [entry for page in pages for entry in page.get("data", [])]
    selected = next((entry for entry in models if entry.get("model") == model), None)
    if selected is None:
        raise ValueError(f"requested model {model!r} absent from authenticated model/list; no fallback")
    supported = {item["reasoningEffort"] for item in selected.get("supportedReasoningEfforts", [])}
    if effort not in supported:
        raise ValueError(f"{model} does not advertise effort {effort!r}; supported: {sorted(supported)}")
    return selected


def read_history(path, home):
    path.resolve().relative_to(home.resolve())
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("native fixture history exceeded 16 MiB")
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows


def usage_totals(rows):
    totals = [row.get("payload", {}).get("info", {}).get("total_token_usage") or {}
              for row in rows if row.get("payload", {}).get("type") == "token_count"
              and isinstance(row.get("payload", {}).get("info"), dict)]
    return {key: max((int(total.get(key, 0)) for total in totals), default=0)
            for key in ("input_tokens", "cached_input_tokens", "output_tokens", "reasoning_output_tokens", "total_tokens")}


def usage_of(rows):
    return usage_totals(rows)["total_tokens"]


def native_turns(rows):
    return sum(1 for row in rows if row.get("type") == "turn_context")


def tool_calls(rows, since=0):
    """Every tool call a native history records from row `since` on: its name
    and its words. Code mode nests the real call in an `exec` (`tools.x(...)`),
    so the words are searched, never the name alone."""
    calls = []
    for index, row in enumerate(rows[since:], since):
        payload = row.get("payload", {})
        if row.get("type") == "response_item" and payload.get("type") in (
                "function_call", "custom_tool_call", "local_shell_call"):
            words = payload.get("input") or payload.get("arguments") or payload.get("action") or ""
            calls.append(dict(row=index, name=payload.get("name") or payload["type"],
                              text=words if isinstance(words, str) else json.dumps(words)))
    return calls


def result_measures(rows):
    """What the crown's tool results weighed (T-930): every output's bytes,
    those that carry the crown's rules (CROWN_WAKES's opening words), and the
    scripts that failed."""
    outs = []
    for row in rows:
        payload = row.get("payload") or {}
        if payload.get("type") in ("custom_tool_call_output", "function_call_output"):
            body = payload.get("output")
            text = body if isinstance(body, str) else " ".join(
                part.get("text", "") for part in body or [] if isinstance(part, dict))
            outs.append(text)
    return dict(results=len(outs), bytes=sum(len(t.encode()) for t in outs),
                rules_bytes=[len(t.encode()) for t in outs if "The board wakes this session on its own" in t],
                failed=sum(1 for t in outs if t.startswith("Script failed") or '"isError": true' in t))


def board_calls(calls):
    """The board's tools among `calls`, by name, in order."""
    found = []
    for call in calls:
        found.extend(tool for tool in BOARD_TOOLS
                     if tool in call["name"] or tool + "(" in call["text"] or '"' + tool + '"' in call["text"])
    return found


def polling(calls):
    """The calls that wait on the forge: a sleep, a watch, or a forge read
    whose words come again in `calls` (the same `gh pr checks 42` twice)."""
    reads = lambda call: {" ".join(m.split()) for m in FORGE_READ.findall(call["text"])}  # noqa: E731
    seen = [read for call in calls for read in reads(call)]
    return [call for call in calls if call["name"] == "sleep" or "tools.sleep(" in call["text"]
            or any(word in call["text"] for word in POLL_WORDS)
            or any(seen.count(read) > 1 for read in reads(call))]


def spawned(calls):
    return [call for call in calls if "spawn_agent" in call["name"] or "spawn_agent(" in call["text"]]


def patched(calls):
    return [call for call in calls if call["name"] == "apply_patch" or "tools.apply_patch(" in call["text"]]


def user_turns(rows):
    """(row, words) of each user message the model read, the turn context
    Codex itself injects left out."""
    found = []
    for index, row in enumerate(rows):
        payload = row.get("payload", {})
        if row.get("type") != "response_item" or payload.get("role") != "user":
            continue
        words = " ".join(part.get("text", "") for part in payload.get("content", []) if isinstance(part, dict))
        if words.strip() and not words.lstrip().startswith(("<environment_context>", "<user_instructions>",
                                                            "<turn_aborted>", "# AGENTS.md")):
            found.append((index, words))
    return found


def answered_after(rows, index):
    """A final answer of the assistant's after row `index`: that turn ended."""
    return any(row.get("type") == "response_item" and row.get("payload", {}).get("role") == "assistant"
               and row["payload"].get("phase") in (None, "final_answer") for row in rows[index + 1:])


# A shell command that writes, in a command's words: output redirected to a
# file (REDIRECT: not `2>&1`, `>/dev/null`, `=>` or `>=`), the usual writers,
# an interpreter that may, and git's own writes. Searched together with the
# names of what the case's work touches (`shell_writes`).
REDIRECT = re.compile(r"(?<![=<>-])[0-9&]?>>?(?![>&=])\s*(?!/dev/null\b)[^\s&|;)'\"]")
WRITE_MARKS = ("tee ", "<<", "touch ", "cp ", "mv ", "install ", "sed -i",
               "python -c", "python3 -c", "perl -e", "perl -i", "node -e", "ruby -e")
WRITE_GIT = re.compile(r"\bgit\b(?:\s+(?:-C|-c|--git-dir|--work-tree)(?:=|\s+)\S+)*\s+"
                       r"(?:add|commit|apply|am|cherry-pick|merge|rebase|reset|checkout|switch|stash|restore|rm|mv)"
                       # The verb whole (T-930: `git merge-base` reads, it does not merge).
                       r"(?=\s|$)")


def command_writes(command, names):
    """A shell command that names one of `names` and writes (WRITE_MARKS,
    WRITE_GIT). Shared with the Claude rig's `crown_wrote_nothing`."""
    return any(name in command for name in names) and (
        any(mark in command for mark in WRITE_MARKS) or bool(REDIRECT.search(command))
        or bool(WRITE_GIT.search(command)))


def shell_writes(calls, names):
    """The calls that run a command which names one of `names` (the work's
    files, a worktree's path) and writes: the crown implementing by shell,
    which `patched` does not see."""
    return [call for call in calls
            if ("exec_command" in call["text"] or "write_stdin" in call["text"] or call["name"] in (
                "exec_command", "shell", "local_shell_call", "write_stdin"))
            and command_writes(call["text"], names)]


def checkout_state(repo):
    """The checkout's HEAD and its working tree, `.codex/` aside: each
    `status --porcelain` line with a digest of the file's bytes beside it,
    so an edit to a file already dirty is a new line too."""
    run = lambda *argv: subprocess.run(["git", "-C", str(repo), *argv], capture_output=True,  # noqa: E731
                                       text=True, timeout=5).stdout
    def digest(line):
        try:
            return hashlib.sha256((Path(repo) / line[3:].split(" -> ")[-1]).read_bytes()).hexdigest()[:8]
        except OSError:
            return "-"
    return dict(head=run("rev-parse", "HEAD").strip(),
                porcelain=sorted(f"{line} {digest(line)}" for line in run("status", "--porcelain", "-uall")
                                 .splitlines() if ".codex/" not in line))


def board_tips(repo, worktrees):
    """The commits a merge of the board's own may leave the checkout's HEAD
    on: each worktree's `merged_oid` (a squash found by patch) and the tip of
    each worktree's branch, which a fast-forward makes HEAD and which then
    carries no `merged_oid` (an ancestor merge names no one commit)."""
    tips = {w.get("merged_oid") for w in worktrees or []}
    for w in worktrees or []:
        if w.get("branch"):
            tips.add(subprocess.run(["git", "-C", str(repo), "rev-parse", "--verify", "-q",
                                     "refs/heads/" + w["branch"]], capture_output=True, text=True,
                                    timeout=5).stdout.strip())
    return tips - {None, ""}


def checkout_changes(before, after, merged=()):
    """What moved in the checkout between two `checkout_state`s: a HEAD
    that is neither the one before nor a tip the board's merge left there
    (`merged`, `board_tips`), and every working-tree line that is new.
    Empty is a checkout the crown left alone."""
    changes = []
    if after["head"] != before["head"] and after["head"] not in merged:
        changes.append(f"HEAD {before['head'][:9]} → {after['head'][:9]}")
    changes.extend(line for line in after["porcelain"] if line not in before["porcelain"])
    return changes


def crown_changes(looks, merged=()):
    """What moved in the checkout that is the crown's, over `looks` in order:
    each a `checkout_state` (`state`) and whether the crown and a worker it
    started were each in a turn at that look (`crown`, `worker`). Every span
    between two looks is `checkout_changes`, the crown's, except a span the
    crown was out of a turn at both ends and a worker in one at either: a
    worker on the shared checkout writes and commits there, and that span's
    moves are its own. A span both were in a turn across counts against the
    crown."""
    changes = []
    for a, b in zip(looks, looks[1:]):
        if not a["crown"] and not b["crown"] and (a["worker"] or b["worker"]):
            continue
        changes.extend(checkout_changes(a["state"], b["state"], merged))
    return changes


def place_problems(cwd, repo, tree, worktrees=None, shared=False):
    """Why a worker at `cwd` is not where its ticket's workspace puts it, or
    an empty list: on the shared checkout (`shared` allows it) its cwd is the
    checkout and no worktree is bound to the ticket (`tree`, the snapshot's,
    None); else that worktree has a branch, its path is the cwd, away from
    the checkout and under `worktrees` where named."""
    cwd, repo = Path(cwd or "/does-not-exist").resolve(), Path(repo).resolve()
    if shared and cwd == repo:
        return [] if tree is None else [f"it works in the checkout, yet {tree.get('path')} is its ticket's worktree"]
    problems = []
    if tree is None or not tree.get("branch"):
        problems.append("no worktree of its own on the board")
    elif Path(tree.get("path") or "/does-not-exist").resolve() != cwd:
        problems.append(f"its worktree {tree.get('path')} is not the worker's cwd {cwd}")
    if cwd == repo or (worktrees and not cwd.is_relative_to(Path(worktrees).resolve())):
        problems.append(f"the worker works in {cwd}, not a worktree of this board's")
    return problems


def seat_problems(sessions, worker):
    """Why `worker` is not the one agent seat on its ticket (a shell
    aside), or an empty list."""
    seats = [s["id"] for s in sessions if s.get("ticket") == worker["ticket"] and s.get("kind") != "bash"]
    return [] if seats == [worker["id"]] else [f"the ticket's agent seats are {seats}, not the worker's alone"]


def delegation_problems(snapshot, feed, crown, crown_sid, worker, repo, worktrees, filed=True, shared=False):
    """Why `worker` is not the crown's delegated worker, or an empty list:
    its ticket filed by the crown's own session from its ticket (`filed`),
    a create and a start of the crown's in `feed`, started by the crown, the
    one agent seat on that ticket (`seat_problems`), and where its workspace
    puts it (`place_problems`): a worktree of its own under this fixture's
    `worktrees`, or with `shared` the checkout too. Which of the two is the
    crown's choice where the request names none."""
    board = snapshot["board"]
    ticket = next((t for t in board["tickets"] if t["id"] == worker["ticket"]), {})
    problems = []
    if filed and (ticket.get("created_by") != "agent:" + crown_sid or ticket.get("created_from") != crown):
        problems.append(f"ticket filed by {ticket.get('created_by')!r} from {ticket.get('created_from')!r}, "
                        "not the crown's session")
    agent = [line.get("cmd") for line in feed if line.get("kind") == "board" and line.get("actor") == "agent"
             and line.get("ticket") == worker["ticket"]]
    for cmd in (("create_ticket", "start_agent") if filed else ("start_agent",)):
        if cmd not in agent:
            problems.append(f"no {cmd} of an agent's for the ticket in the feed")
    if worker.get("started_by") != crown:
        problems.append(f"started by {worker.get('started_by')!r}")
    problems += seat_problems(board.get("sessions") or [], worker)
    tree = next((w for w in snapshot.get("worktrees") or [] if w.get("ticket") == worker["ticket"]), None)
    return problems + place_problems(worker.get("cwd"), repo, tree, worktrees, shared)


# The failure the stub forge reports (`pull_requests`): words that carry it
# name the job or the pull request, and say it failed.
FAILURE_NAME = re.compile(r"test-linux|(?:#|\bpr\s*#?|pull request\s*#?)42\b")
FAILURE_WORD = re.compile(r"\b(?:fail\w*|red|broke\w*|error\w*)\b")


def conveys_failure(words):
    lower = " ".join(words.lower().split())
    return bool(FAILURE_NAME.search(lower) and FAILURE_WORD.search(lower))


ASK_TEXT = re.compile(r'''["']?text["']?\s*:\s*("(?:[^"\\]|\\.)*")''')


def ask_texts(calls):
    """The words of each ask_agent call, where its arguments spell them as a
    double-quoted string (code mode's JS object or JSON alike)."""
    texts = []
    for call in calls:
        if "ask_agent" not in call["name"] and "ask_agent(" not in call["text"]:
            continue
        for literal in ASK_TEXT.findall(call["text"]):
            try:
                texts.append(json.loads(literal))
            except ValueError:
                continue
    return texts


def heard_the_ask(heard, asked):
    """The worker's prompt carries the crown's ask: its words whole, spaces
    aside, where the ask's words could be read off the call; and either way
    the failure itself."""
    flat = lambda text: " ".join(text.split())  # noqa: E731
    if not conveys_failure(heard):
        return False
    return not asked or any(flat(text) in flat(heard) for text in asked)


# What the rig allows, matched exactly (a live model writes these requests):
# Codex's one-tool approval for mesimon's own server, as 0.163 words it, and a
# worker committing in its own folder (its worktree, or the checkout for a
# worker on the shared checkout) in one of three plain shapes.
MCP_APPROVAL = re.compile(r'Allow the mesimon MCP server to run tool "([a-z_]+)"\?')


def plain_git(tokens):
    """`git status`, `git add <paths>` (no option, no path outside the
    worktree), or `git commit -m <message>` / `-am <message>`: nothing else."""
    if tokens[:1] != ["git"] or len(tokens) < 2:
        return False
    verb, rest = tokens[1], tokens[2:]
    if verb == "status":
        return rest in ([], ["--short"], ["-s"])
    # A whitespace check writes nothing (T-930: Sol stages, checks, commits).
    if verb == "diff":
        return rest in (["--check"], ["--cached", "--check"], ["--staged", "--check"])
    if verb == "add":
        return bool(rest) and all(not t.startswith("-") and ".." not in Path(t).parts
                                  and not Path(t).is_absolute() and PLAIN_PATH.fullmatch(t)
                                  for t in rest)
    if verb == "commit":
        return len(rest) == 2 and rest[0] in ("-m", "-am") and bool(PLAIN_MESSAGE.fullmatch(rest[1]))
    return False


# What a shell expands nothing in (T-930): an approved string runs under the
# login shell, whose globs, `~`, `=cmd`, braces and `!` a shlex split cannot see.
PLAIN_PATH = re.compile(r"[A-Za-z0-9._/@+-]+")
PLAIN_MESSAGE = re.compile(r"[A-Za-z0-9 ._,:/@+-]+")


def approval_decision(request, role, worktree):
    """What the rig, as the person at the desk, does with a native approval
    on a crown fixture: (True, what) to allow it once, (False, why) to leave
    it standing. A board tool is allowed when the request names mesimon's
    server exactly and one of BOARD_TOOLS (the daemon's authorize is their
    gate); a worker's `plain_git` when the request's cwd resolves to that
    worker's own folder (`worktree`: its worktree, the checkout for a worker
    on the shared checkout, None for the crown); anything else stays the
    person's and is recorded."""
    what = request.get("what")
    command = str(request.get("command") or "").strip()
    reason = str(request.get("reason") or "").strip()
    words = (command + " " + reason).strip()
    if what == "mcp_tool":
        server, _, named = command.partition(" · ")
        match = MCP_APPROVAL.fullmatch(reason)
        tool = named or (match.group(1) if match else "")
        if server == "mesimon" and tool in BOARD_TOOLS and (not match or match.group(1) == tool):
            return True, tool
        return False, f"{what}: {words[:200]}"
    cwd = request.get("cwd")
    # Codex 0.164 asks for an escalated command as the login shell's `-lc`
    # string (`/bin/zsh -lc "git add …"`); that string is the command judged.
    try:
        wrapped = shlex.split(command)
    except ValueError:
        wrapped = []
    judged = command
    if len(wrapped) == 3 and wrapped[0] in ("/bin/zsh", "/bin/bash", "/bin/sh") and wrapped[1] == "-lc":
        judged = wrapped[2]
    # Plain shapes joined by `&&` are one commit (T-930: Sol writes `git add
    # README.md && git commit -m … && git status --short`); each is judged alone.
    parts = judged.split(" && ")
    if (what == "command" and role == "worker" and worktree and cwd
            and Path(cwd).resolve() == Path(worktree).resolve()
            and not any(mark in part for part in parts for mark in ";&|`$<>\n\r\\")):
        try:
            tokens = [shlex.split(part) for part in parts]
        except ValueError:
            tokens = [[]]
        if all(plain_git(t) for t in tokens):
            return True, command
    return False, f"{what}: {words[:200]}"


def worker_push(request):
    """A worker's `git push` approval, inside a login shell's `-lc` or not
    (T-930): the fixture's remote is never reached, so the person at the
    desk declines it, which ends the worker's turn as a refusal does."""
    command = str(request.get("command") or "").strip()
    try:
        words = shlex.split(command)
    except ValueError:
        return False
    if len(words) == 3 and words[0] in ("/bin/zsh", "/bin/bash", "/bin/sh") and words[1] == "-lc":
        command = words[2]
    # Any step of an `&&` chain (Sol: `git add … && git commit … && git push`).
    try:
        steps = [shlex.split(part) for part in command.split("&&")]
    except ValueError:
        return False
    return request.get("what") == "command" and any(step[:2] == ["git", "push"] for step in steps)


def dialog_allows(screen):
    """The native approval dialog is drawn with its allow-once row under the
    cursor (`› 1. Allow`, `› 1. Yes, …`). Anything else, a remembering row
    and the composer's `›` included, is not one."""
    rows = [line.strip()[1:].strip() for line in screen.splitlines()
            if line.strip().startswith("›") and line.strip()[1:].strip()[:2].rstrip(".").isdigit()]
    number, _, label = rows[-1].partition(".") if rows else ("", "", "")
    # Row 1 is once; a later Allow or Yes remembers the choice.
    return number == "1" and label.strip().startswith(("Allow", "Yes"))


def pull_requests(branch, tip, pipeline):
    """`gh pr list --json`'s answer for the stub forge: PR 42 open on the
    worker's branch at its tip, its one job in `pipeline` (ci/rig.py's shape)."""
    conclusion = {"running": ("IN_PROGRESS", None), "failed": ("COMPLETED", "FAILURE"),
                  "passed": ("COMPLETED", "SUCCESS")}[pipeline]
    return json.dumps([{
        "number": 42, "url": "https://github.com/example/rig/pull/42", "state": "OPEN", "isDraft": False,
        "headRefName": branch, "headRefOid": tip, "mergeCommit": None, "mergeable": "MERGEABLE",
        "reviewDecision": "REVIEW_REQUIRED", "updatedAt": "2026-10-23T06:00:00Z",
        "statusCheckRollup": [{"__typename": "CheckRun", "name": "test-linux",
                               "status": conclusion[0], "conclusion": conclusion[1]}]}])


def forge_stub(directory, stub_body):
    """The Claude rig's stub `gh` (ci/rig.py `GH_STUB`, T-739) with its two
    files named inside it, so no environment has to carry them."""
    directory.mkdir(mode=0o700)
    (directory / "prs.json").write_text("[]")
    (directory / "checks.json").write_text('{"check_runs":[]}')
    gh = directory / "gh"
    gh.write_text("#!/bin/sh\nMSMN_GH_PRS=" + shlex.quote(str(directory / "prs.json"))
                  + "\nMSMN_GH_CHECKS=" + shlex.quote(str(directory / "checks.json"))
                  + "\nexport MSMN_GH_PRS MSMN_GH_CHECKS\n" + stub_body.split("\n", 1)[1])
    gh.chmod(0o700)
    return gh


def fixture_home(home, path_first=None):
    """The crown fixtures' own HOME (T-916). The daemon reads the machine's
    tiers from `$HOME/.local/state/mesimon/tiers.toml`, and a crown picks
    among them: a first forge run's crown started its worker on the person's
    own Claude tier. Here only the built-in tiers exist, `claude` refuses to
    start (MESIMON_CLAUDE_BIN), git has a fixture identity, and the login shell
    the daemon captures is the rig's PATH, with `path_first` (the stub forge)
    ahead of it: the forge looks for `gh` there before the daemon's own PATH."""
    home.mkdir(mode=0o700)
    (home / ".gitconfig").write_text("[user]\n\tname = fixture\n\temail = fixture@example.invalid\n")
    claude = home / "claude-refused"
    claude.write_text("#!/bin/sh\necho 'claude is not part of this Codex fixture' >&2\nexit 1\n")
    claude.chmod(0o700)
    path = os.environ.get("PATH", "")
    if path_first:
        path = str(path_first) + ":" + path
    shell = home / "login-shell"
    shell.write_text("#!/bin/sh\nPATH=" + shlex.quote(path)
                     + "\nexport PATH\nfor command; do :; done\nexec /bin/sh -c \"$command\"\n")
    shell.chmod(0o700)
    return shell, claude, path


def fixture_configuration(original, args):
    settings = "plan_mode_reasoning_effort = " + json.dumps(args.effort) + "\n"
    settings += original.replace('model_reasoning_effort = "low"',
                                 "model_reasoning_effort = " + json.dumps(args.effort))
    enabled = "true" if args.case in CHILDREN else "false"
    agents = ("enabled = " + enabled + "\ndefault_subagent_model = " + json.dumps(args.model)
              + "\ndefault_subagent_reasoning_effort = " + json.dumps(args.effort) + "\n")
    if args.case in CHILDREN and "max_threads" not in original:
        agents += "max_threads = 1\n"
    if "[agents]\n" in settings:
        settings = settings.replace("[agents]\n", "[agents]\n" + agents)
    else:
        settings += "\n[agents]\n" + agents
    return settings + "\n[features]\nmulti_agent = " + enabled + "\n"


def adapter(base):
    class Verification(base):
        def __init__(self, args, out, manifest, guard):
            super().__init__(args, out, manifest, guard)
            self.last_usage_check = 0
            self.observed_tokens = 0
            config = self.home / "config.toml"
            config.write_text(fixture_configuration(config.read_text(), args))
            wrapper = guard.root / "codex-isolated"
            original = wrapper.read_text()
            wrapper.write_text(original.replace("#!/bin/sh\n", "#!/bin/sh\n" +
                "unset OPENAI_API_KEY OPENAI_BASE_URL CODEX_API_KEY AZURE_OPENAI_API_KEY\n", 1))
            if args.case in ("mcp", "brief"):
                for column in self.client.board()["columns"]:
                    settings = {key: value for key, value in column.items() if key not in ("name", "order")}
                    settings["agent_tools"] = "read"
                    self.request("set_column_settings", name=column["name"], settings=settings)
            self.crown_sid = self.crown = None
            self.answered, self.left = {}, {}
            # The checkout's looks while a case attributes its moves by
            # session (`crown_changes`); None where it does not.
            self.looks = None
            if args.case in CROWN:
                self.crown_board()

        def crown_board(self):
            """A board a person crowns a Codex seat on (T-916): one commit, so
            a worktree can be cut; the board's tools and its brief on, as on
            a fresh board; every column on workspace-write with on-request
            approvals, so the crown could do the work itself with no dialog in
            its way, and delegating is its choice. The forge case also gets a
            github.com remote, the stub forge first on the daemon's PATH and
            its login shell's, and Merge into: pull requests."""
            for argv in (["add", "-A"], ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid",
                                         "commit", "-q", "-m", "fixture"]):
                subprocess.run(["git", "-C", str(self.repo), *argv], check=True, timeout=5)
            stub = None
            if self.args.case == "forge":
                self.forge_dir = stub = self.guard.root / "forge-stub"
                forge_stub(self.forge_dir, module("rig_claude", "rig.py").GH_STUB)
                subprocess.run(["git", "-C", str(self.repo), "remote", "add", "origin",
                                "https://github.com/example/rig.git"], check=True, timeout=5)
            home = self.guard.root / "home"
            shell, claude, path = fixture_home(home, stub)
            self.request("shutdown")
            self.client.close()
            deadline = time.monotonic() + 10
            while (self.runtime / "orch.sock").exists():
                if time.monotonic() > deadline:
                    raise RuntimeError("the fixture daemon did not stop for its own HOME")
                time.sleep(0.1)
            self.env.update(HOME=str(home), SHELL=str(shell), PATH=path, MESIMON_CLAUDE_BIN=str(claude))
            if stub:
                self.env.update(MESIMON_FORGE_EVERY_MS="2000", MESIMON_FORGE_BUSY_MS="2000")
            # The board's state follows HOME; it lies inside the fixture root.
            self.state = home / ".local/state/mesimon" / self.state.name
            self.guard.request(op="spawn", argv=[str(self.args.binary), "daemon", "--repo", str(self.repo)],
                               env=self.env)
            deadline = time.monotonic() + 15
            while not (self.runtime / "orch.sock").exists():
                if time.monotonic() > deadline:
                    raise RuntimeError("the fixture daemon did not come back on its own HOME")
                time.sleep(0.1)
            self.client = type(self.client)(self.repo)
            machine = self.snapshot().get("machine_tiers") or {}
            if [tier.get("id") for tier in machine.get("tiers", []) if tier.get("id") not in ("claude", "codex")]:
                raise RuntimeError("the fixture board still sees a person's tiers")
            if stub:
                self.request("set_merge_into", into="pull_requests")
            self.request("set_mcp_tools", on=True)
            self.request("set_system_prompt", on=True)
            policy = dict(codex_sandbox="workspace_write", codex_approval="on_request")
            for column in self.client.board()["columns"]:
                settings = {key: value for key, value in column.items() if key not in ("name", "order")}
                self.request("set_column_settings", name=column["name"], settings=settings | policy)
            self.manifest["crown_fixture"] = dict(policy, mcp_tools=True, brief=True,
                                                  native_children=self.args.case in CHILDREN,
                                                  merge_into="pull_requests" if self.args.case == "forge" else "main")

        def preflight(self):
            endpoint = self.guard.root / "preflight.sock"
            executable = self.guard.root / "codex-isolated"
            pid = self.guard.request(op="spawn", argv=[str(executable), "app-server", "--listen",
                                     "unix://" + str(endpoint)], env=self.env)
            deadline = time.monotonic() + 20
            while not endpoint.exists():
                if time.monotonic() > deadline or self.guard.request(op="poll", pid=pid) is not None:
                    raise RuntimeError("model preflight app-server did not start")
                time.sleep(0.1)
            probe = module("rig_preflight", "codex-runtime-probe.py")
            rpc = probe.Rpc(endpoint, self.out, "preflight")
            try:
                rpc.initialize()
                response = rpc.call("config/read", {"includeLayers": False, "cwd": str(self.repo)})
                configuration = response.get("result", {}).get("config", {})
                if (configuration.get("model") != self.args.model
                        or configuration.get("model_reasoning_effort") != self.args.effort
                        or configuration.get("plan_mode_reasoning_effort") != self.args.effort
                        or configuration.get("agents", {}).get("enabled") != (self.args.case in CHILDREN)):
                    raise RuntimeError("native configuration did not confirm the fixture model, efforts and child policy")
                pages, cursor = [], None
                for _ in range(20):
                    params = {"includeHidden": False, "limit": 100}
                    if cursor:
                        params["cursor"] = cursor
                    response = rpc.call("model/list", params)
                    if "error" in response:
                        raise RuntimeError("model/list refused: " + str(response["error"]))
                    pages.append(response["result"])
                    cursor = response["result"].get("nextCursor")
                    if not cursor:
                        break
                else:
                    raise RuntimeError("model/list pagination exceeded its bound")
                self.manifest["selected_model"] = choose_model(pages, self.args.model, self.args.effort)
            finally:
                rpc.close()
            self.mark(assertion="authenticated model and effort available before any turn", passed=True)

        def reserve_turn(self):
            if self.calls >= self.args.user_turn_budget:
                raise AssertionError("case submission budget exhausted")
            self.check_usage(force=True)

        def start(self, prompt):
            self.reserve_turn()
            return super().start(prompt)

        def ask(self, sid, prompt):
            self.reserve_turn()
            return super().ask(sid, prompt)

        def check_usage(self, force=False):
            if not force and time.monotonic() - self.last_usage_check < 1:
                return
            self.last_usage_check = time.monotonic()
            histories = list(self.home.glob("sessions/**/*.jsonl"))
            totals = usage_totals([])
            turns = 0
            for path in histories:
                rows = read_history(path, self.home)
                reading = usage_totals(rows)
                turns += native_turns(rows)
                for key in totals:
                    totals[key] += reading[key]
            self.observed_tokens = totals["total_tokens"]
            self.manifest["observed_total_tokens"] = self.observed_tokens
            self.manifest["observed_usage"] = totals
            self.manifest["observed_native_turns"] = turns
            if self.observed_tokens >= self.args.remaining_tokens:
                raise AssertionError("observed token allowance exhausted; stopping owned fixture")
            if turns > NATIVE_TURNS.get(self.args.case, turns):
                raise AssertionError(f"native turn allowance exhausted ({turns} turns across the fixture's "
                                     "threads); stopping owned fixture")

        def observe(self, sid):
            self.check_usage()
            return super().observe(sid)

        def mark(self, **fields):
            super().mark(**fields)
            if fields.get("assertion"):
                print("  " + fields["assertion"], flush=True)

        # ---------------------------------------------- the crown (T-916)

        def history(self, session):
            path = Path(session.get("transcript_path") or "/does-not-exist")
            return read_history(path, self.home) if path.is_file() else []

        def feed(self):
            path = self.state / "activity.jsonl"
            rows = []
            for line in (path.read_text().splitlines() if path.is_file() else []):
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    continue
            return rows

        def carried(self, sid):
            path = self.runtime / ("cdx-" + sid.replace("-", "")[:16] + ".json")
            try:
                return json.loads(path.read_text()).get("request")
            except (OSError, ValueError):
                return None

        def porcelain(self):
            out = subprocess.run(["git", "-C", str(self.repo), "status", "--porcelain", "-uall"],
                                 capture_output=True, text=True, timeout=5).stdout
            return sorted(line for line in out.splitlines() if ".codex/" not in line)

        @staticmethod
        def seat(board, sid):
            return next(s for s in board["sessions"] if s["id"] == sid)

        @staticmethod
        def turn_ended(session):
            return (session["state"] == {"state": "idle", "stop_reason": "end_turn"}
                    and not session.get("pending_submit") and not session.get("observation_hold", True))

        def startup_consent(self, sid, screen):
            # A worker the crown started opens in a worktree this fixture's
            # board cut; the base only accepts the fixture's checkout.
            session = next((s for s in self.client.board()["sessions"] if s["id"] == sid), {})
            cwd = session.get("cwd") or ""
            if (self.args.case in CROWN and "Trust this folder?" in screen and "Trust and continue" in screen
                    and cwd.startswith(str(self.state / "worktrees") + "/")
                    and Path(cwd).name in "".join(line.strip() for line in screen.splitlines())):
                self.verify_native_home(sid)
                self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Enter")
                self.mark(action="native_fixture_worktree_trust", session=sid, cwd=cwd)
                time.sleep(0.3)
                return
            return super().startup_consent(sid, screen)

        def approval(self, session, screen):
            """A native approval on a crown fixture, answered as the person
            at the desk would (`approval_decision`), once, and recorded."""
            sid = session["id"]
            request = self.carried(sid)
            # Codex numbers requests per session (T-930: the worker's first is
            # `0`, as the crown's was), so a request is known by both.
            key = (sid, request.get("id")) if request else None
            if not request or request.get("kind") != "approval" or key in self.answered:
                return
            role = "crown" if sid == self.crown_sid else "worker"
            worktree = session.get("cwd") if role == "worker" else None
            if worktree and not (Path(worktree).resolve().is_relative_to((self.state / "worktrees").resolve())
                                 or Path(worktree).resolve() == self.repo.resolve()):
                worktree = None
            allow, what = approval_decision(request, role, worktree)
            if not allow and role == "worker" and worker_push(request) and dialog_allows(screen):
                self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Escape")
                self.answered[key] = what
                self.manifest.setdefault("declined", []).append(dict(session=sid, role=role, request=what))
                self.mark(action="person_declines_worker_push", session=sid, request=what)
                return
            if not allow:
                if key not in self.left:
                    self.left[key] = time.monotonic()
                    self.manifest.setdefault("approvals_left", []).append(dict(session=sid, role=role, request=what))
                    self.mark(action="native_approval_left_for_the_person", session=sid, role=role, request=what)
                if role == "crown" and time.monotonic() - self.left[key] > 8:
                    raise AssertionError("the crown waits on the person's approval of " + what)
                return
            if not dialog_allows(screen):
                return
            self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Enter")
            self.answered[key] = what
            self.manifest.setdefault("approvals", []).append(dict(session=sid, role=role,
                                                                  what=request.get("what"), allowed=what))
            self.mark(action="person_allows_native_approval_once", session=sid, role=role, allowed=what)

        def snapshot(self):
            return self.client.request("snapshot")

        def tend(self):
            """One look at every Codex seat on the board: startup consent and
            approvals for each, and every seat's history kept for the capture.
            The board comes back with the snapshot's worktrees and PRs."""
            snapshot = self.snapshot()
            board = dict(snapshot["board"], worktrees=snapshot.get("worktrees") or [],
                         prs=snapshot.get("prs") or [])
            for session in board["sessions"]:
                if session["kind"] != "codex":
                    continue
                if session["id"] not in self.sessions:
                    self.sessions.append(session["id"])
                if session["state"]["state"] in ("sleeping", "exited", "failed"):
                    continue
                _, _, screen = self.observe(session["id"])
                self.startup_consent(session["id"], screen)
                self.approval(session, screen)
            if self.looks is not None:
                look = self.look(board)
                if look != self.looks[-1]:
                    self.looks.append(look)
            return board

        def look(self, board):
            """The checkout as it is now, and whether the crown and a worker
            it started are each in a turn (`crown_changes`); a crown not on
            the board reads as in one."""
            busy = lambda s: s["state"]["state"] not in ("sleeping", "exited", "failed") \
                and not self.turn_ended(s)  # noqa: E731
            crown = next((s for s in board["sessions"] if s["id"] == self.crown_sid), None)
            return dict(state=checkout_state(self.repo), crown=crown is None or busy(crown),
                        worker=any(busy(s) for s in board["sessions"] if s.get("started_by") == self.crown
                                   and s["ticket"] != self.crown and s["kind"] != "bash"))

        def until(self, description, predicate, steady=1, **evidence):
            """Tend the board until `predicate(board)` holds `steady` looks in a
            row; its value is returned. The case deadline bounds it."""
            held = 0
            while True:
                board = self.tend()
                value = predicate(board)
                held = held + 1 if value else 0
                if held >= steady:
                    self.mark(assertion=description, passed=True, **evidence)
                    return value
                time.sleep(0.25)

        def crown_seat(self):
            """The crowned seat, on the checkout, its composer empty."""
            self.crown = self.request("create_ticket", column="TODO", title="Coordinate this board",
                                      workspace="shared_checkout")["id"]
            self.request("crown_ticket", id=self.crown)
            self.crown_sid = self.request("spawn_session", ticket=self.crown, kind="claude",
                                          submit_prompt=False)["id"]
            self.sessions.append(self.crown_sid)
            self.wait(self.crown_sid, "crowned Codex seat ready, its composer empty", lambda s, t, p:
                      s["state"]["state"] == "idle" and not s.get("pending_prefill", True)
                      and not s.get("observation_hold", True))
            self.tm("send-keys", "-t", self.crown_sid.replace("-", "")[:16], "C-u")

        def crown_rows(self, board):
            return self.history(self.seat(board, self.crown_sid))

        def crown_turn_on(self, board, words, since=0):
            """The crown's turn on the first user message from row `since`
            holding `words`: (row, ended) or None before it arrives."""
            rows = self.crown_rows(board)
            row = next((index for index, text in user_turns(rows) if index >= since and words in text), None)
            if row is None:
                return None
            return row, answered_after(rows, row) and self.turn_ended(self.seat(board, self.crown_sid))

        def worker_started(self, board, ask_words, ticket=None):
            """The seat the crown started (on `ticket`, if named), or an
            AssertionError once the crown's turn on the ask ended without one."""
            worker = next((s for s in board["sessions"] if s.get("started_by") == self.crown
                           and s["ticket"] != self.crown and ticket in (None, s["ticket"])), None)
            if worker:
                return worker
            turn = self.crown_turn_on(board, ask_words)
            if turn and turn[1]:
                calls = tool_calls(self.crown_rows(board), turn[0])
                raise AssertionError(
                    "the crown ended its turn on the request without starting a board worker; "
                    f"board tools {board_calls(calls)}, native children {len(spawned(calls))}, "
                    f"its own patches {len(patched(calls))}, checkout {self.porcelain()}")
            return None

        def record_calls(self, name, calls):
            self.manifest[name] = [dict(name=call["name"], board=board_calls([call]), text=call["text"][:400])
                                   for call in calls]

        def launched(self, board, sid):
            """The worker's seat once it has a cwd: at once on the checkout
            (judged by `delegation_problems`), else once the board lists the
            ticket's worktree, which a cut that is still running does not."""
            seat = self.seat(board, sid)
            if not seat.get("cwd"):
                return None
            if Path(seat["cwd"]).resolve() == self.repo.resolve():
                return seat
            return seat if any(w.get("ticket") == seat["ticket"] and w.get("path")
                               for w in board["worktrees"]) else None

        def implemented(self, calls, names, since):
            """Every sign the crown did the work itself: in its own calls a
            native child, an `apply_patch`, a shell command that writes naming
            the work (`shell_writes`); and any move of the checkout's HEAD or
            working tree that is the crown's, a tip the board merged from a
            worktree aside. `since` is a `checkout_state`, every move after it
            the crown's, or the case's looks (`crown_changes`), which leave a
            shared-checkout worker's own spans out."""
            snapshot = self.snapshot()
            merged = board_tips(self.repo, snapshot.get("worktrees"))
            looks = since if isinstance(since, list) else [dict(state=since, crown=True, worker=False)]
            signs = [f"native child: {c['text'][:120]}" for c in spawned(calls)]
            signs += [f"apply_patch: {c['text'][:120]}" for c in patched(calls)]
            signs += [f"shell write: {c['text'][:160]}" for c in shell_writes(calls, names)]
            signs += [f"checkout: {change}" for change in crown_changes(looks + [self.look(snapshot["board"])], merged)]
            return signs

        def delegate(self):
            """A person asks the crown for concrete work, in a person's words
            (DELEGATE_ASK, no tool named). Passes only when the crown's own
            session files a board ticket and starts a Codex worker on it, in a
            worktree of its own or on the shared checkout, the crown's choice
            (`delegation_problems`), and nothing shows the crown doing the work
            (`implemented`: no child, patch, shell write or checkout change of
            its own, a shared worker's spans aside); then the worker's turn
            ends, the board wakes the crown with it, and the crown's turn on
            that wake ends, still with nothing of the work its own."""
            self.crown_seat()
            # The crown is asked at once: the first span is its own.
            self.looks = [dict(state=checkout_state(self.repo), crown=True, worker=False)]
            mark = len(self.feed())
            names = ("greet.py", "test_greet.py")
            self.ask(self.crown_sid, DELEGATE_ASK)
            def started(board):
                try:
                    return self.worker_started(board, "greet.py")
                except AssertionError as kept:
                    raise AssertionError(f"{kept}; signs of the work done here: "
                                         f"{self.implemented(tool_calls(self.crown_rows(board)), names, self.looks)}")
            worker = self.until("the crown starts a board worker on the request", started)
            # The start may wait on the worktree's cut: the record's cwd and
            # the snapshot's worktree come with the worker's launch.
            worker = self.until("the worker's seat is launched", lambda board: self.launched(board, worker["id"]))
            board = self.tend()
            snapshot = self.snapshot()
            ticket = next(t for t in board["tickets"] if t["id"] == worker["ticket"])
            calls = tool_calls(self.crown_rows(board))
            self.record_calls("crown_calls_before_wake", calls)
            tree = next((w for w in board["worktrees"] if w.get("ticket") == ticket["id"]), {})
            self.manifest["delegation"] = dict(key=ticket["short_key"], title=ticket["title"],
                                               created_by=ticket.get("created_by"), created_from=ticket.get("created_from"),
                                               worker_kind=worker["kind"], cwd=worker.get("cwd"),
                                               branch=tree.get("branch"), worktree=tree.get("path"), tier=ticket.get("tier"))
            problems = delegation_problems(snapshot, self.feed()[mark:], self.crown, self.crown_sid, worker,
                                           self.repo, self.state / "worktrees", shared=True)
            if worker["kind"] != "codex":
                problems.append("the worker is not a Codex seat: " + worker["kind"])
            if problems:
                raise AssertionError("the crown's start is not a delegated worker: " + "; ".join(problems))
            signs = self.implemented(calls, names, self.looks)
            if signs:
                raise AssertionError("the crown started a worker and did the work itself as well: " + "; ".join(signs))
            self.mark(assertion="the crown's session filed the request and started its Codex worker on that "
                      "ticket, " + ("on the shared checkout" if not tree else "in its own worktree"),
                      passed=True, **self.manifest["delegation"])
            self.until("the worker's turn ends", lambda board: self.turn_ended(self.seat(board, worker["id"]))
                       and any(answered_after(self.history(self.seat(board, worker["id"])), row)
                               for row, _ in user_turns(self.history(self.seat(board, worker["id"])))[:1]))
            wake = self.until("the board wakes the crown with its worker's news", lambda board: next((
                line for line in self.feed()[mark:] if line.get("kind") == "crown_wake"
                and line.get("worker") == ticket["id"]), None))
            row, _ = self.until("the crown's turn on that wake ends", lambda board: (
                lambda turn: turn if turn and turn[1] else None)(
                self.crown_turn_on(board, ticket["short_key"], since=self.crown_turn_on(board, "greet.py")[0] + 1)),
                steady=4, cause=wake.get("cause"))
            board = self.client.board()
            every = tool_calls(self.crown_rows(board))
            self.record_calls("crown_calls_on_wake", tool_calls(self.crown_rows(board), row))
            cwd = Path(worker["cwd"])
            self.manifest["worker_files"] = {name: (cwd / name).is_file() for name in names}
            self.manifest["crown_board_calls"] = board_calls(every)
            # A worktree's path is the work's; the checkout's is the crown's own cwd.
            signs = self.implemented(every, names + ((str(cwd),) if tree else ()), self.looks)
            if signs:
                raise AssertionError("the crown did the work itself around its worker: " + "; ".join(signs))
            self.mark(assertion="delivery follow-through: the crown heard its worker and its turn on that ended, "
                      "nothing of the work its own", passed=True, wake=wake.get("cause"),
                      files=self.manifest["worker_files"], crown_board_calls_on_wake=board_calls(every[len(calls):]))

        def forge(self):
            """A person asks the crown to start a ticket and look after its
            pull request, getting CI fixed if it fails (FORGE_ASK). The stub
            forge opens PR 42 on the worker's branch, running, then fails its
            job `test-linux`. Passes when the crown started the worker in that
            ticket's own worktree (`delegation_problems`, the ticket a
            person's), the board wakes the crown with the forge's news, the
            crown's ask_agent reaches the worker and the words the worker reads
            carry the failure (`heard_the_ask`: the ask's own words, naming the
            job or the PR and that it failed), and no turn of the crown's from
            the news on waits on the pull request (a watch, a sleep, a poll)
            or fixes it itself (`implemented`)."""
            ticket = self.request("create_ticket", column="TODO", title=FORGE_WORKER, workspace=None)["id"]
            key = next(t for t in self.client.board()["tickets"] if t["id"] == ticket)["short_key"]
            self.crown_seat()
            start = len(self.feed())
            ask = FORGE_ASK.format(key=key)
            self.ask(self.crown_sid, ask)
            worker = self.until("the crown starts the worker", lambda board: self.worker_started(board, key, ticket))
            worker = self.until("the worker's seat is launched", lambda board: self.launched(board, worker["id"]))
            problems = delegation_problems(self.snapshot(), self.feed()[start:], self.crown, self.crown_sid, worker,
                                           self.repo, self.state / "worktrees", filed=False)
            if problems:
                raise AssertionError("the crown's start is not a worktree worker: " + "; ".join(problems))
            tree = next(w for w in self.snapshot()["worktrees"] if w.get("ticket") == ticket and w.get("branch"))
            branch, cwd = tree["branch"], tree["path"]
            mark = start
            def settled(board):
                feed = self.feed()[mark:]
                wakes = [line for line in feed if line.get("kind") == "crown_wake" and line.get("worker") == ticket]
                sent = [line for line in feed if line.get("cmd") == "crown_wake_sent" and line.get("ticket") == self.crown]
                rows = self.crown_rows(board)
                last = user_turns(rows)[-1][0] if user_turns(rows) else None
                # A worker whose push the person declined ends its turn
                # interrupted (T-930), which is its turn over all the same.
                seat = self.seat(board, worker["id"])
                over = self.turn_ended(seat) or self.turn_ended(
                    dict(seat, state={"state": "idle", "stop_reason": "end_turn"})) and seat["state"] == {
                    "state": "idle", "stop_reason": "interrupted"}
                return (wakes and len(sent) >= 1 and last is not None and answered_after(rows, last)
                        and over
                        and self.turn_ended(self.seat(board, self.crown_sid)))
            self.until("the worker's first turn and the crown's turn on its wake end", settled, steady=12)
            prs = self.forge_dir / "prs.json"
            def publish(pipeline):
                tip = subprocess.run(["git", "-C", str(self.repo), "rev-parse", branch], capture_output=True,
                                     text=True, timeout=5).stdout.strip()
                (self.forge_dir / "prs.tmp").write_text(pull_requests(branch, tip, pipeline))
                (self.forge_dir / "prs.tmp").replace(prs)
            publish("running")
            self.until("the stub forge's pull request binds to the worker's ticket", lambda board: any(
                p.get("ticket") == ticket and p.get("number") == 42 for p in board["prs"]))
            self.until("the crown is idle before the pipeline fails", lambda board:
                       self.turn_ended(self.seat(board, self.crown_sid)), steady=12)
            mark = len(self.feed())
            board = self.client.board()
            crown_from = len(self.crown_rows(board))
            worker_from = len(self.history(self.seat(board, worker["id"])))
            before = checkout_state(self.repo)
            branch_before = subprocess.run(["git", "-C", str(self.repo), "rev-parse", branch], capture_output=True,
                                           text=True, timeout=5).stdout.strip()
            publish("failed")
            wake = self.until("the board wakes the crown with the failed pipeline", lambda board: next((
                line for line in self.feed()[mark:] if line.get("kind") == "crown_wake"
                and line.get("worker") == ticket and line.get("cause") == "forge"), None))
            def told(board):
                line = next((line for line in self.feed()[mark:] if line.get("kind") == "board"
                             and line.get("actor") == "agent" and line.get("cmd") == "ask_agent"
                             and line.get("ticket") == ticket), None)
                if line:
                    return line
                turn = self.crown_turn_on(board, key, since=crown_from)
                if turn and turn[1]:
                    calls = tool_calls(self.crown_rows(board), crown_from)
                    self.record_calls("crown_calls_on_forge_news", calls)
                    raise AssertionError("the crown ended its turn on the failed pipeline without telling the "
                                         f"worker; board tools {board_calls(calls)}, polls {len(polling(calls))}, "
                                         f"its own work {self.implemented(calls, ('README', cwd, branch), before)}")
                return None
            line = self.until("the crown asks the worker on the failed pipeline", told)
            self.until("the crown's turn on the forge's news ends", lambda board: (
                lambda turn: turn and turn[1])(self.crown_turn_on(board, key, since=crown_from)), steady=4)
            calls = tool_calls(self.crown_rows(self.client.board()), crown_from)
            asked = ask_texts(calls)
            def heard(board):
                rows = self.history(self.seat(board, worker["id"]))
                prompts = [words for row, words in user_turns(rows) if row >= worker_from]
                return next((words for words in prompts if heard_the_ask(words, asked)), None) \
                    or (prompts and self.turn_ended(self.seat(board, worker["id"]))
                        and answered_after(rows, max(r for r, _ in user_turns(rows))) and prompts)
            words = self.until("the worker reads the crown's words as a prompt", heard)
            if isinstance(words, list):
                raise AssertionError("the words the worker read do not carry the failed pipeline: "
                                     + " | ".join(w[:200] for w in words) + f"; the crown asked {asked}")
            calls = tool_calls(self.crown_rows(self.client.board()), crown_from)
            self.record_calls("crown_calls_on_forge_news", calls)
            self.manifest["forge"] = dict(key=key, branch=branch, wake=wake.get("cause"), ask=line.get("outcome"),
                                          asked=[a[:400] for a in asked], asked_read_off_call=bool(asked),
                                          worker_heard=words[:400], crown_board_calls=board_calls(calls))
            if polling(calls):
                raise AssertionError("the crown waited on the pull request instead of ending its turn: "
                                     + "; ".join(call["text"][:120] for call in polling(calls)))
            signs = self.implemented(calls, ("README", cwd, branch), before)
            branch_after = subprocess.run(["git", "-C", str(self.repo), "rev-parse", branch], capture_output=True,
                                          text=True, timeout=5).stdout.strip()
            if signs:
                raise AssertionError("the crown fixed the pipeline itself instead of through its worker: "
                                     + "; ".join(signs))
            self.manifest["forge"]["branch_moved_after_news"] = branch_after != branch_before
            if not asked:
                # The worker's words carry the failure, but whether they are
                # the crown's ask is unproven: inconclusive, never a pass.
                raise Inconclusive("the ask_agent call's words could not be read off it, so the worker's "
                                   "prompt is not shown to be the crown's ask")
            self.mark(assertion="the crown told its worker of the failed job and yielded: the worker read the "
                      "failure in the crown's words, and no watch, sleep, poll or fix of the crown's own",
                      passed=True, **self.manifest["forge"])

        def collect(self):
            super().collect()
            feed = self.state / "activity.jsonl"
            if self.args.case in CROWN and feed.is_file() and feed.stat().st_size <= 16 * 1024 * 1024:
                shutil.copyfile(feed, self.out / "activity.jsonl")
            if self.args.case in CROWN:
                (self.out / "snapshot.json").write_text(json.dumps(self.snapshot(), indent=1) + "\n")

        def run(self):
            if self.args.case in CROWN:
                return getattr(self, self.args.case)()
            if self.args.case == "interrupt":
                sid = self.start("Run sleep 25 once, then reply exactly MESIMON_INTERRUPTED. No other tools.")
                self.wait(sid, "native model activity precedes interruption", lambda session, ticket, screen:
                          session["state"]["state"] == "running" and self.observed_tokens > 0
                          and session.get("codex_turn_id"))
                self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Escape")
                self.wait(sid, "interruption never promotes to REVIEW", lambda session, ticket, screen:
                          session["state"] == {"state": "idle", "stop_reason": "interrupted"}
                          and ticket["column"] == "IN PROGRESS")
                return
            if self.args.case == "steer":
                return self.steer()
            if self.args.case == "plan-launch":
                return self.plan_launch()
            if self.args.case != "queue":
                return super().run()
            sid = self.start("Run sleep 8 once, then reply exactly MESIMON_QUEUE_FIRST. No other tools.")
            session, _, _ = self.wait(sid, "queue fixture is working", lambda session, ticket, screen:
                                      session["state"]["state"] == "running")
            thread = session["codex_thread_id"]
            self.reserve_turn()
            self.calls += 1
            receipt = self.request("prompt_session", ticket=session["ticket"], queued=True,
                                   text="Reply exactly MESIMON_QUEUE_SECOND. Do not use tools.")
            if receipt.get("resp") != "queued":
                raise AssertionError("running turn did not queue the second prompt: " + str(receipt))
            final, _, _ = self.completed(sid, "MESIMON_QUEUE_SECOND")
            if final["codex_thread_id"] != thread:
                raise AssertionError("queued prompt changed native thread")
            rows = read_history(Path(final["transcript_path"]), self.home)
            messages = [row["payload"] for row in rows if row.get("type") == "response_item"
                        and row.get("payload", {}).get("role") == "assistant"]
            replies = [" ".join(part.get("text", "") for part in message.get("content", []))
                       for message in messages if message.get("phase") in (None, "final_answer")]
            first = [index for index, text in enumerate(replies) if text.strip() == "MESIMON_QUEUE_FIRST"]
            second = [index for index, text in enumerate(replies) if text.strip() == "MESIMON_QUEUE_SECOND"]
            if len(first) != 1 or len(second) != 1 or first[0] >= second[0]:
                raise AssertionError("queued turn did not produce exactly one ordered pair of final replies")
            self.mark(assertion="queued prompt completes once on the same thread after the first reply", passed=True)

        def plan_launch(self):
            """A start in plan mode on Codex (T-847): the board types /plan
            before the title, the title goes once the thread is planning, the
            turn stops on the native plan dialog, and the board's accept moves
            the same thread on to a turn that implements it."""
            self.reserve_turn()
            self.calls += 1
            title = ("Propose a minimal one-step plan to append the line PLAN_ACCEPTED to README.md. "
                     "Do not implement it yet. Do not run tools; this fixture is fully described here.")
            ticket = self.request("create_ticket", column="TODO", title=title, workspace=None)["id"]
            sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=True,
                               plan=True)["id"]
            self.sessions.append(sid)
            before = (self.repo / "README.md").read_text()
            feed = self.state / "activity.jsonl"
            session, _, _ = self.wait(sid, "the planned launch stops on its native plan dialog",
                                      lambda s, t, p: s["state"] == {"state": "requires_action", "reason": "plan"}
                                      and s.get("plan_note") and s.get("codex_plan_dialog_seen")
                                      and "Implement this plan?" in p, timeout=180)
            if '"cmd":"codex_plan_mode"' not in (feed.read_text() if feed.is_file() else ""):
                raise AssertionError("the board never saw the thread planning before the words went")
            if (self.repo / "README.md").read_text() != before:
                raise AssertionError("the planning turn changed the file it was asked to plan")
            note = self.request("read_note", ticket=ticket, note=session["plan_note"])
            if "PLAN_ACCEPTED" not in note.get("text", ""):
                raise AssertionError("the native plan did not reach the ticket note")
            self.mark(assertion="plan mode took before the title and the turn ended on a plan", passed=True)
            thread = session["codex_thread_id"]
            self.reserve_turn()
            self.calls += 1
            receipt = self.request("prompt_session", ticket=ticket, text="", queued=True, accept_plan=True)
            if receipt.get("resp") not in ("queued", "ok"):
                raise AssertionError("the board refused the plan accept: " + str(receipt))
            # The implementation is a turn of its own on the same thread. Its
            # file write asks the read-only fixture sandbox's permission,
            # which stays the person's: the turn moving on is the claim.
            turn = session["codex_turn_id"]
            self.wait(sid, "the board's accept moves the same thread on to a turn of its own",
                      lambda s, t, p: s.get("codex_thread_id") == thread
                      and s.get("codex_turn_id") not in (None, turn)
                      and s["state"] != {"state": "requires_action", "reason": "plan"}
                      and "Implement this plan?" not in p, timeout=90)

        def steer(self):
            """Words sent to a running Codex turn go into that turn (T-845): the
            board pastes them into the composer and presses Enter, Codex sends a
            turn/steer, and the same turn's reply follows them."""
            sid = self.start("Run sleep 15 once in the terminal, wait for it to finish, then reply "
                             "exactly MESIMON_STEER_FIRST. Do not modify files.")
            def active_tool(session, ticket, screen):
                path = Path(session.get("agent_preview_path") or "/does-not-exist")
                value = json.loads(path.read_text()) if path.is_file() else {}
                return (session["state"]["state"] == "running" and session.get("codex_turn_id")
                        and isinstance(value.get("activity"), dict) and "Tool" in value["activity"])
            session, _, _ = self.wait(sid, "a real tool runs in the turn", active_tool)
            turn = session["codex_turn_id"]
            self.reserve_turn()
            self.calls += 1
            sent = time.time()
            receipt = self.request("prompt_session", ticket=session["ticket"], queued=False,
                                   text="Change of plan: reply exactly MESIMON_STEERED instead.")
            if receipt.get("resp") != "ok":
                raise AssertionError("a running Codex turn refused the steer: " + str(receipt))
            feed = self.state / "activity.jsonl"
            self.wait(sid, "the board saw the running turn take the steer", lambda *_:
                      feed.is_file() and '"cmd":"codex_steered"' in feed.read_text(), timeout=20)
            final, _, _ = self.completed(sid, "MESIMON_STEERED")
            if final["codex_turn_id"] != turn:
                raise AssertionError("the steer started a turn of its own instead of joining the running one")
            rows = read_history(Path(final["transcript_path"]), self.home)
            stamp = lambda row: datetime.datetime.fromisoformat(row["timestamp"].replace("Z", "+00:00")).timestamp()
            steered = [row for row in rows if row.get("type") == "response_item"
                       and row.get("payload", {}).get("role") == "user"
                       and "MESIMON_STEERED" in json.dumps(row["payload"])]
            commands = [row for row in rows if row.get("type") == "event_msg"
                        and row.get("payload", {}).get("type") == "item_completed"
                        and row["payload"].get("item", {}).get("type") == "CommandExecution"]
            if not steered or not commands:
                raise RuntimeError("the history holds no steer or no finished command to order")
            read_at, command_done = stamp(steered[0]), stamp(commands[0])
            # What the help line says rests on this (T-845, 0.163): the steer
            # waits for the turn's next step, and never cuts a command short.
            self.manifest["steer_timing"] = dict(
                read_after_send_ms=int((read_at - sent) * 1000),
                command_done_after_send_ms=int((command_done - sent) * 1000),
                read_while_command_ran=read_at < command_done,
                command_exit_code=commands[0]["payload"]["item"].get("exit_code"))
            self.mark(assertion="the steer was read in the running turn, which replied to it", passed=True,
                      **self.manifest["steer_timing"])

        def audit(self):
            self.check_usage(force=True)
            if not self.observed_tokens:
                raise RuntimeError("native usage unavailable after model activity; cannot verify budget")
            contexts = []
            for path in self.home.glob("sessions/**/*.jsonl"):
                contexts.extend(row["payload"] for row in read_history(path, self.home)
                                if row.get("type") == "turn_context")
            if not contexts or any(row.get("model") != self.args.model or
                                   row.get("effort", row.get("reasoning_effort")) != self.args.effort
                                   for row in contexts):
                raise AssertionError("native turn contexts did not confirm the selected model and effort")
            self.manifest["submitted_user_turns"] = self.calls
            self.manifest["models"] = sorted({f"{row.get('model')}/{row.get('effort', row.get('reasoning_effort'))}"
                                              for row in contexts})
            if self.args.case in CROWN and getattr(self, "crown_sid", None):
                try:
                    self.manifest["crown_results"] = result_measures(self.crown_rows(self.snapshot()["board"]))
                except Exception as error:  # a measure never fails the audit
                    self.manifest["crown_results"] = dict(error=str(error)[:200])
            self.mark(assertion="actual native turns used the selected model and effort", passed=True)

    return Verification


def replace_text(path, text):
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(text)
    temporary.replace(path)


def report(directory, manifest):
    directory.mkdir(parents=True, exist_ok=True)
    replace_text(directory / "verdicts.json", json.dumps(manifest, indent=2) + "\n")
    lines = ["# Codex rig", "", f"{manifest['version']} · {manifest['model']} · {manifest['effort']}",
             "", "| Case | Outcome | Submitted turns | Observed tokens | Cleanup |",
             "| --- | --- | ---: | ---: | --- |"]
    for row in manifest["cases"]:
        lines.append(f"| {row['case']} | {row['outcome']} | {row.get('submitted_user_turns', 0)} | "
                     f"{row.get('observed_total_tokens', 0)} | {row.get('cleanup', False)} |")
    for row in manifest["cases"]:
        for key in ("error", "capture_error", "cleanup_error"):
            if row.get(key):
                lines.extend(["", f"{row['case']} ({key}): {row[key]}", ""])
    lines.extend(["", "Token readings may lag and omit native overhead; no dollar cap is claimed.",
                  "A Codex question is the crown's where the board lets it (T-846); approvals, secrets and forms stay the person's."])
    replace_text(directory / "verdicts.md", "\n".join(lines) + "\n")


def run(repo, args, parser):
    if args.reset or args.stale or args.flags_off:
        parser.error("Codex fixtures are supervised and reaped after every case; Claude maintenance flags do not apply")
    out = Path(repo) / "target/rig/codex"
    previous = json.loads((out / "verdicts.json").read_text()) if (out / "verdicts.json").exists() else None
    resolve_options(args, previous)
    try:
        cases = select_cases(args, previous)
    except (ValueError, KeyError) as error:
        parser.error(str(error))
    model = args.model
    print(f"Codex rig: {', '.join(cases) or 'nothing to rerun'}; {model}, {args.effort}; "
          f"at most {sum(TURNS[case] for case in cases)} scenario turns", flush=True)
    if args.lay or not cases:
        return 0
    if not args.live:
        parser.error("Codex model calls require --live; --lay lists the selection without starting anything")
    if not os.environ.get("MESIMON_TEST_RUN"):
        command = [sys.executable, "-B", str(ROOT / "ci/test-run.py"), "--timeout",
                   str(min(1200, 120 + sum(case_timeout(args, case) + 30 for case in cases))), "--", sys.executable,
                   "-B", str(ROOT / "ci/rig.py"), *sys.argv[1:]]
        os.execv(sys.executable, command)
    binary = Path(repo) / "target/debug/mesimon"
    if not args.no_build:
        subprocess.run(["cargo", "build", "-p", "mesimon"], cwd=repo, check=True)
    binary = binary.resolve(strict=True)
    executable = shutil.which("codex")
    tmux = os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux")
    if not executable or not tmux:
        parser.error("installed Codex and tmux are required")
    state = module("rig_codex_state", "codex-state-e2e.py")
    version = subprocess.check_output([executable, "--version"], text=True, timeout=10).strip()
    run_id = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    directory = out / run_id
    directory.mkdir(parents=True, mode=0o700)
    manifest = dict(version=version, model=model, effort=args.effort, run_id=run_id,
                    mesimon_sha256=state.probe.digest(binary), max_turns=args.max_turns,
                    runner_sha256={name: state.probe.digest(ROOT / name) for name in
                                   ("ci/rig.py", "ci/rig/codex.py", "ci/codex-state-e2e.py", "ci/codex-lifecycle-e2e.py")},
                    max_tokens=args.max_tokens, case_timeout=args.case_timeout,
                    cases=[dict(case=case, outcome="not_run", cleanup=False,
                                                          checkpoints=[]) for case in cases])
    report(directory, manifest)
    report(out, manifest)
    used_tokens = 0
    stopped = False
    for result in manifest["cases"]:
        case = result["case"]
        if stopped:
            report(directory, manifest)
            report(out, manifest)
            continue
        print(f"\n▶ Codex {case}", flush=True)
        capture = directory / case
        capture.mkdir(mode=0o700)
        result["capture"] = str(capture)
        result["case_timeout"] = case_timeout(args, case)
        result["outcome"] = "running"
        report(directory, manifest)
        report(out, manifest)
        case_args = argparse.Namespace(case=case, model=model, effort=args.effort, timeout=case_timeout(args, case),
            binary=binary, codex=executable, tmux=tmux, claude=None, claude_model="haiku",
            auth_file=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "auth.json",
            user_turn_budget=TURNS[case] - int(case in ("child", "stop", "compact")),
            remaining_tokens=min(args.max_tokens - used_tokens, CASE_TOKENS.get(case, args.max_tokens)))
        guard = verification = None
        try:
            guard = state.probe.Guard(case_timeout(args, case) + 25, tmux)
            base = module("rig_codex_lifecycle", "codex-lifecycle-e2e.py").Verification if case in LIFECYCLE else state.Verification
            verification = adapter(base)(case_args, capture, result, guard)
            print("  Watch from an external terminal: cd " + shlex.quote(str(verification.repo))
                  + " && " + shlex.quote(str(binary)), flush=True)
            verification.preflight()
            verification.run()
            verification.audit()
            if any(point.get("passed") is False for point in result["checkpoints"]):
                raise AssertionError("a native checkpoint failed")
            result["outcome"] = "passed"
            result.pop("error", None)
        except AssertionError as error:
            result.update(outcome="failed", error=str(error))
        except Exception as error:
            result.update(outcome="inconclusive", error=type(error).__name__ + ": " + str(error))
        finally:
            if verification:
                result["submitted_user_turns"] = verification.calls
                try:
                    verification.collect()
                    state.write(capture / "board.json", verification.client.board())
                except Exception as error:
                    result.update(outcome="inconclusive", capture_error=str(error))
                finally:
                    try:
                        verification.client.close()
                    except Exception as error:
                        result.update(outcome="inconclusive", capture_error=str(error))
            if guard:
                try:
                    result["cleanup"] = guard.close()
                except Exception as error:
                    result["cleanup_error"] = str(error)
            if not result["cleanup"]:
                result["outcome"] = "inconclusive"
            state.write(capture / "manifest.json", result)
            used_tokens += result.get("observed_total_tokens", 0)
            stopped = result["outcome"] != "passed" or used_tokens >= args.max_tokens
            report(directory, manifest)
            report(out, manifest)
            detail = result.get("error") or result.get("capture_error") or result.get("cleanup_error") or ""
            print(f"  {result['outcome']} · cleanup {result['cleanup']} · {detail}", flush=True)
    print("\nVerdicts: " + str(out / "verdicts.md"), flush=True)
    return int(any(row["outcome"] != "passed" for row in manifest["cases"]))
