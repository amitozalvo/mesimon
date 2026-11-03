#!/usr/bin/env python3
"""A scripted stand-in for `claude`, for the README recordings only.

record.sh points MESIMON_CLAUDE_BIN here, so the daemon launches this with
exactly the argv it gives claude (`--settings`, `--mcp-config`,
`--session-id`, ...) and pastes the prompt into its pane the same way. It
answers through the same two roads a real session does: the hook commands
named in its settings file, and the `mesimon mcp` tool shim named in its MCP
config. Nothing on the board is faked -- only the model is.

What it does is keyed off the ticket title in its first prompt (seed.py):
the demo ticket works, raises its hand with a question, takes the answer,
commits and stops; the crown clip's ticket works until a person crowns it,
then runs the board through the crown's tools; the ticket-page clip's
ticket reads the code, leaves a note with two links and stops; the
pull-request clip's crown starts two workers and answers what the forge
says of them, and its workers open pull requests through the stand-in `gh`
(forge.py); any other ticket keeps working, and answers what is typed into
its pane without stopping (agent.tape steps in and does that).
"""

import json
import os
import re
import select
import signal
import subprocess
import sys
import termios
import time
import traceback
import tty
import uuid

DEMO = "Stats as JSON"
QUESTION = "Include expired links?"
CROWN = "Plan 1.0"
NOTED = "Retire old links"
SLUGS = "Custom slugs"
SLUGS_QUESTION = "Reserve admin and api?"
SPRINT = "Ship 1.1"
LIMITER = "Rate limiting"
SLASH = "Fix slash 404"

# The note the ticket-page take opens: markdown, one link into the repo and
# one out of it, so `^k` has both kinds to list.
NOTE = """# Where it goes

The check is one line in `links()`, ./store.py:20 — skip a link whose last
click is older than 30 days, and free its slug.

A freed slug should answer 410 rather than 404, so a client can tell it from
a typo: [RFC 9110, 410 Gone](https://www.rfc-editor.org/rfc/rfc9110#section-15.5.11).
"""

# What agent.tape types into the busy agent's pane, and the answer.
ASIDE = "Also log each 429"
ASIDE_REPLY = "Will do: every 429 also goes to stderr, with the client's IP."


def flag(name):
    return sys.argv[sys.argv.index(name) + 1]


SETTINGS = json.load(open(flag("--settings")))
SESSION = flag("--session-id")
MCP = next(iter(json.loads(flag("--mcp-config"))["mcpServers"].values()))
CWD = os.getcwd()
HOME = os.environ.get("MESIMON_CLAUDE_HOME", os.path.expanduser("~/.claude"))
SLUG = "".join(c if c.isalnum() else "-" for c in CWD)
TRANSCRIPT = os.path.join(HOME, "projects", SLUG, f"{SESSION}.jsonl")

# One escape sequence on the input (tmux's replies and reports); the stub
# acts on none of them.
ESCAPE = re.compile(rb"\x1b(\[[0-9;:?<>=]*[@-~]|[^\[])")


class Pane:
    """The pane as a terminal agent keeps it: output scrolls, and under it a
    prompt line takes what is typed or pasted. Enter submits a non-empty
    prompt. The line over the box says what this is, so a recording that
    steps into the pane never passes the stub off as claude.

    The box is drawn the way the daemon reads Claude Code's composer
    (daemon/src/agents/claude/composer.rs): a rule, the `❯ ` row, a rule. A
    launch's words go only into a composer it has seen (T-570, T-987)."""

    RULE = " demo stand-in for claude · scripted, no model "

    def __init__(self):
        # TCSANOW, not setraw's default flush: the daemon types the title
        # into the pane before this process is reading.
        tty.setraw(0, termios.TCSANOW)
        # A clean screen, and bracketed paste: the daemon pastes with -p.
        self.write("\x1b[H\x1b[2J\x1b[?2004h")
        self.buf = b""
        self.line = b""
        self.drawn = False
        # A resize (the board attaching at its own size) redraws the prompt
        # at the new width; the handler only wakes `wait`, which draws.
        self.winch, w = os.pipe()
        os.set_blocking(self.winch, False)
        os.set_blocking(w, False)
        signal.signal(signal.SIGWINCH, lambda *_: None)
        signal.set_wakeup_fd(w)
        self.redraw()

    @staticmethod
    def write(s):
        sys.stdout.write(s)
        sys.stdout.flush()

    def prompt(self):
        cols = os.get_terminal_size(1).columns
        label = ("──" + self.RULE).ljust(cols - 1, "─")[: cols - 1]
        rule = "─" * (cols - 1)
        typed = self.line.decode(errors="replace").replace("\n", " ")
        return (f"\x1b[2m{label}\r\n{rule}\x1b[0m\r\n"
                f"\x1b[1m❯\x1b[0m {typed[: cols - 3]}\r\n\x1b[2m{rule}\x1b[0m")

    def erase(self):
        """Take the box off the screen, cursor at its first row; its four
        rows are the last ones written."""
        if self.drawn:
            self.write("\r\x1b[3A\x1b[J")

    def say(self, text=""):
        self.erase()
        self.write(text.replace("\n", "\r\n") + "\r\n" + self.prompt())
        self.drawn = True

    def redraw(self):
        self.erase()
        self.write(self.prompt())
        self.drawn = True

    def wait(self, seconds=None):
        """Take keys for `seconds`, or until a prompt is submitted with no
        limit. Returns the submitted prompt, or None when the time ran out."""
        deadline = None if seconds is None else time.monotonic() + seconds
        while True:
            words = self.take()
            if words:
                return words
            left = None if deadline is None else deadline - time.monotonic()
            if left is not None and left <= 0:
                return None
            ready, _, _ = select.select([0, self.winch], [], [], left)
            if self.winch in ready:
                while True:
                    try:
                        os.read(self.winch, 64)
                    except BlockingIOError:
                        break
                self.redraw()
            if 0 in ready:
                self.buf += os.read(0, 65536)

    def take(self):
        """Move buffered input onto the prompt line; a submitted prompt, if
        an Enter ended a non-empty one."""
        before = self.line
        words = None
        while self.buf and words is None:
            if self.buf.startswith(b"\x1b[200~"):
                end = self.buf.find(b"\x1b[201~")
                if end == -1:
                    break
                self.line += self.buf[6:end]
                self.buf = self.buf[end + 6:]
            elif self.buf[:1] == b"\x1b":
                seq = ESCAPE.match(self.buf)
                if not seq:
                    break
                self.buf = self.buf[seq.end():]
            elif self.buf[:1] in (b"\r", b"\n"):
                self.buf = self.buf[1:]
                if self.line.strip():
                    words = self.line.decode(errors="replace").strip()
                    self.line = b""
            elif self.buf[:1] in (b"\x7f", b"\x08"):
                self.buf = self.buf[1:]
                self.line = self.line[:-1]
            elif self.buf[:1] == b"\x03":
                # Ctrl+C empties the box, as it does claude's (T-570).
                self.buf = self.buf[1:]
                self.line = b""
            else:
                if self.buf[0] >= 0x20:
                    self.line += self.buf[:1]
                self.buf = self.buf[1:]
        if self.line != before:
            self.redraw()
        return words


PANE = None


def say(text=""):
    PANE.say(text)


def hook(event, reason=None, **body):
    """Run the observer entry the settings file names for `event`."""
    for entry in SETTINGS["hooks"].get(event, []):
        h = entry["hooks"][0]
        if h["args"][0] != "hook":
            continue
        if reason is not None and entry.get("matcher") != reason:
            continue
        payload = {
            "session_id": SESSION,
            "transcript_path": TRANSCRIPT,
            "cwd": CWD,
            "hook_event_name": event,
            **body,
        }
        subprocess.run(
            [h["command"], *h["args"]],
            input=json.dumps(payload).encode(),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
        return


def record(kind, content, **message):
    """Append one line to the transcript, in Claude Code's shape. The board
    reads its preview from here: the newest text, and the running tool's
    description."""
    os.makedirs(os.path.dirname(TRANSCRIPT), exist_ok=True)
    line = {
        "type": kind,
        "uuid": str(uuid.uuid4()),
        "sessionId": SESSION,
        "cwd": CWD,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime()),
        "message": {"role": kind, "content": content, **message},
    }
    with open(TRANSCRIPT, "a") as f:
        f.write(json.dumps(line) + "\n")


def tool(name, description, until=None, result="ok", **tool_input):
    """A tool call: its transcript lines, its hook, a line in the pane. It
    runs 1.2 s, or with `until`, up to 2 s and no longer than that is false.
    `result` is what the tool answered, as the transcript keeps it (the
    board reads a `gh pr create`'s URL from there). Returns a prompt
    submitted while it ran, if one was."""
    use_id = f"toolu_{uuid.uuid4().hex[:24]}"
    record("assistant", [{"type": "tool_use", "id": use_id, "name": name,
                          "input": {"description": description, **tool_input}}])
    say(f"  \x1b[1m{name}\x1b[0m  {description}")
    if until is None:
        asked = PANE.wait(1.2)
    else:
        asked = None
        end = time.time() + 2
        while asked is None and time.time() < end and not until():
            asked = PANE.wait(0.25)
    record("user", [{"type": "tool_result", "tool_use_id": use_id, "content": result}])
    hook("PostToolUse", tool_name=name, tool_input=tool_input, tool_use_id=use_id)
    return asked


def text(words, last=False):
    extra = {"stop_reason": "end_turn"} if last else {}
    record("assistant", [{"type": "text", "text": words}], **extra)
    say(f"  {words}")


def submitted(words):
    record("user", words)
    hook("UserPromptSubmit", prompt=words)
    say(f"\x1b[1m>\x1b[0m {words.splitlines()[0]}")


class Board:
    """A session with the `mesimon mcp` shim, held open the way claude holds
    one: initialize once, then one `tools/call` per board tool."""

    def __init__(self):
        self.shim = subprocess.Popen([MCP["command"], *MCP["args"]],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL)
        self.next_id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "demo-stub", "version": "0"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, msg):
        self.shim.stdin.write((json.dumps(msg) + "\n").encode())
        self.shim.stdin.flush()

    def rpc(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        return json.loads(self.shim.stdout.readline())["result"]

    def call(self, tool, /, **arguments):
        """One board tool. A JSON answer comes back parsed; a refusal raises,
        because the take has no model to read it and try again."""
        result = self.rpc("tools/call", {"name": tool, "arguments": arguments})
        body = result["content"][0]["text"]
        if result.get("isError"):
            raise RuntimeError(f"{tool}: {body}")
        try:
            return json.loads(body)
        except ValueError:
            return body

    def close(self):
        self.shim.stdin.close()
        self.shim.wait(timeout=5)


def raise_hand(reason):
    board = Board()
    board.call("raise_hand", reason=reason)
    board.close()


def column_of(board, title):
    return next((t["column"] for t in board.call("list_board")["tickets"]
                 if t["title"] == title), None)


def paced(title, column, steps):
    """The reddit clip (DEMO_PACED) shows one thing at a time: a worker
    keeps working, a step at a time, until the ticket `title` is in
    `column`, and raises its hand a beat later. Elsewhere, no wait."""
    if not os.environ.get("DEMO_PACED"):
        return
    board = Board()
    there = lambda: column_of(board, title) == column
    n = 0
    while not there():
        tool(*steps[n % len(steps)], until=there)
        n += 1
    board.close()
    PANE.wait(2.5)


def demo(first):
    submitted(first)
    text("I'll add a --json flag to `shortlink stats`.")
    tool("Read", "Read stats.py", file_path=os.path.join(CWD, "stats.py"))
    tool("Edit", "Add the --json flag", file_path=os.path.join(CWD, "stats.py"))
    write_json_flag(expired=False)
    tool("Bash", "Run the stats tests", command="python3 -m pytest -q")
    # On the reddit board, the hand goes up once the crown has started its
    # own worker, so the crown's turn is over when the person answers.
    paced(SLUGS, "IN PROGRESS", [("Edit", "Cover --json in the tests"),
                                 ("Bash", "Run the stats tests")])
    text(f"--json works. {QUESTION} They are hidden from the table today.", last=True)
    raise_hand(QUESTION)
    hook("Stop", stop_hook_active=False)

    submitted(PANE.wait())
    tool("Edit", "Add an expired field", file_path=os.path.join(CWD, "stats.py"))
    write_json_flag(expired=True)
    tool("Bash", "Run the stats tests", command="python3 -m pytest -q")
    subprocess.run(["git", "commit", "-qam", "stats: add --json output"], cwd=CWD,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tool("Bash", "Commit", command="git commit -am 'stats: add --json output'")
    text("Done: `stats --json` prints one object per link, expired ones included "
         "with \"expired\": true. Committed on this ticket's branch.", last=True)
    hook("Stop", stop_hook_active=False)


def crown(first):
    """Work until a person crowns this ticket, then run the board: file two
    tickets, move one, tag one, start an agent on another. Every edit is a
    keyed tool call through the shim, so the board lights each card itself."""
    submitted(first)
    text("Reading the board for what 1.0 still needs.")
    board = Board()
    keys = {t["title"]: t["key"] for t in board.call("list_board")["tickets"]}
    # The reddit clip's board (T-1125) has no "Expire old links": there the
    # first turn reads the board and ends, the person crowns the idle agent
    # and tells it what to do, and that ask is the turn that acts. The
    # crown clip's agent acts the moment it is crowned.
    reddit = "Expire old links" not in keys
    steps = ["Read the open tickets", "Check the release checklist"]
    n = 0
    # Nothing is typed into a pane when a person crowns its ticket (the
    # crown adds no token to the conversation); a model finds out by
    # asking, so between steps this asks every quarter second.
    crowned = lambda: board.call("get_ticket")["crowned"]
    if reddit:
        for step in steps:
            tool("Read", step)
        text("1.0 is missing a health check endpoint and release notes. I can file "
             "them and start the work once I wear the crown.", last=True)
        hook("Stop", stop_hook_active=False)
        submitted(PANE.wait())
        text(f"On it: filing both, then starting {SLUGS}.")
    else:
        while not crowned():
            tool("Read", steps[n % len(steps)], until=crowned)
            n += 1
        text("I wear the crown now. Filing what 1.0 is missing.")
    filed = crown_tool(board, "create_ticket", "File: health check endpoint",
                       title="Health check endpoint", column="TODO",
                       description="GET /healthz answers 200 while the store is reachable.",
                       workspace="worktree")
    notes = crown_tool(board, "create_ticket", "File: 1.0 release notes",
                       title="1.0 release notes", column="TODO",
                       description="What changed since 0.9, for the release page.",
                       workspace="worktree")
    stale = keys["Custom slugs"]
    if not reddit:
        crown_tool(board, "move_ticket", f"Move {stale} back to TODO: nobody is on it",
                   key=stale, to_column="TODO", seen=seen(board, stale))
    crown_tool(board, "tag_ticket", f"Tag {notes['key']} MILESTONE",
               key=notes["key"], name="MILESTONE")
    # The crown clip's board has "Expire old links"; the reddit clip's crown
    # starts "Custom slugs", whose agent asks the crown and is merged.
    expire = keys.get("Expire old links") or keys[SLUGS]
    started = crown_tool(board, "start_agent", f"Start an agent on {expire}",
                         key=expire, seen=seen(board, expire), workspace="worktree")
    left = started["budget_left"]
    moved = "" if reddit else f"moved {stale} back to TODO, "
    text(f"Filed {filed['key']} and {notes['key']}, {moved}tagged "
         f"{notes['key']} MILESTONE and started an agent on {expire}: "
         f"{left} {'start' if left == 1 else 'starts'} left in the crown's budget.",
         last=True)
    hook("Stop", stop_hook_active=False)
    if reddit:
        heard(board, {
            "hand": (f"Answered {expire}: reserve admin and api.",
                     lambda _: crown_tool(board, "ask_agent", f"Answer {expire}",
                                        key=expire, seen=seen(board, expire),
                                        text="Reserve both: answer 409 for admin and api.")),
            # A merged worker is done: parked, its worktree is reclaimed.
            "merged": (f"{expire} merged and parked; 1.0's open work is filed in TODO.",
                       lambda _: crown_tool(board, "sleep_agent", f"Park {expire}: merged",
                                            key=expire, seen=seen(board, expire))),
        }, {expire})


def heard(board, says, workers=(), idle="Nothing else needs me."):
    """The crown after its turn: each wake the board types into its pane
    (an agent it started ended a turn, raised its hand, merged; the forge
    said something of its pull request) is one short turn. `says` maps a
    word in the wake to what the crown says, and what it does first; a
    wake with none of them gets `idle`. A clause about one of `workers`
    (the crown's own) is read before one about any other ticket."""
    while True:
        wake = PANE.wait()
        submitted(wake)
        if os.environ.get("DEMO_WAKES"):
            # What the board typed, for whoever tunes the words above.
            with open(os.environ["DEMO_WAKES"], "a") as f:
                f.write(wake + "\n")
        PANE.wait(0.8)
        # One wake can speak of several tickets, one clause each (`; `
        # between them): the key is the one in the clause with the word.
        clauses = [c for line in wake.splitlines() for c in line.split("; ")]
        clauses.sort(key=lambda c: not any(re.search(rf"\b{k}\b", c) for k in workers))
        for word, (said, act) in says.items():
            line = next((c for c in clauses if word in c), None)
            if line is None:
                continue
            key = re.search(r"\bT-\d+\b", line) or re.search(r"\bT-\d+\b", wake)
            key = key.group() if key else "it"
            if act:
                act(key)
            text(said.replace("{key}", key), last=True)
            break
        else:
            text(idle, last=True)
        hook("Stop", stop_hook_active=False)


def slugs(first):
    """The reddit clip's crown-started worker: works, asks the crown (a
    raised hand on an agent the crown started goes to the crown), takes the
    answer the crown sends, commits and stops."""
    submitted(first)
    text("Letting /shorten take a slug of the caller's choosing.")
    tool("Read", "Read store.py", file_path=os.path.join(CWD, "store.py"))
    tool("Edit", "Take a slug from the request", file_path=os.path.join(CWD, "store.py"))
    write_slugs(reserved=False)
    tool("Bash", "Run the tests", command="python3 -m pytest -q")
    # Its question waits for the person's worker to land: one beat at a time.
    paced(DEMO, "DONE", [("Edit", "Cover a chosen slug in the tests"),
                         ("Bash", "Run the tests")])
    text(f"Custom slugs work. {SLUGS_QUESTION} Nobody should be able to take them.",
         last=True)
    raise_hand(SLUGS_QUESTION)
    hook("Stop", stop_hook_active=False)

    submitted(PANE.wait())
    tool("Edit", "Reserve admin and api", file_path=os.path.join(CWD, "slugs.py"))
    write_slugs(reserved=True)
    tool("Bash", "Run the tests", command="python3 -m pytest -q")
    subprocess.run(["git", "add", "-A"], cwd=CWD, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "commit", "-qm", "shorten: custom slugs, admin and api reserved"],
                   cwd=CWD, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tool("Bash", "Commit", command="git commit -am 'shorten: custom slugs'")
    text("Done: a request may name its slug; admin and api answer 409. Committed on "
         "this ticket's branch.", last=True)
    hook("Stop", stop_hook_active=False)


def sprint(first):
    """The pull-request clip's crown: starts a worker on each open ticket,
    then hears what the forge says of their pull requests. A pipeline that
    fails sends the worker back with the job's name; the rest is noted on
    the card."""
    submitted(first)
    text("Two items open for 1.1. Starting a worker on each; the board tells me what "
         "the forge makes of their pull requests.")
    board = Board()
    keys = {t["title"]: t["key"] for t in board.call("list_board")["tickets"]}
    for title in (LIMITER, SLASH):
        crown_tool(board, "start_agent", f"Start an agent on {keys[title]} ({title})",
                   key=keys[title], seen=seen(board, keys[title]), workspace="worktree")
    text(f"Started {keys[LIMITER]} and {keys[SLASH]}. Waiting on the forge.", last=True)
    hook("Stop", stop_hook_active=False)

    def send_back(key):
        crown_tool(board, "ask_agent", f"Send {key} back to the failed job",
                   key=key, seen=seen(board, key), deliver="idle",
                   text="The tests job failed on your PR. Fix it and push.")

    workers = {keys[LIMITER], keys[SLASH]}

    def park(key):
        # A merged worker is done: parked, its seat in the budget is free.
        # A card with no agent (one that came with its PR) has none to park.
        if key in workers:
            crown_tool(board, "sleep_agent", f"Park {key}: merged",
                       key=key, seen=seen(board, key))

    # In the order the words settle a wake: a line can say merged and
    # passed, or deployed and merged, in one breath.
    heard(board, {
        "pipeline failed": ("Sent {key} back: its tests job failed.", send_back),
        "deployed to production": ("{key} is in production.", None),
        "deployed to staging": ("{key} is on staging; production is next.", None),
        "merged": ("{key} merged; the deploys say where it goes.", park),
        "pipeline passed": ("{key} is green; a teammate merges it from here.", None),
        "opened": ("{key} opened its pull request; the pipeline runs.", None),
    }, workers, idle="Heard the forge; nothing for me to do.")


def go(delay):
    """Hold until the take has begun (the DEMO_GO file kitty-term.py
    makes), plus `delay` seconds: the pull-request clip's workers are
    started before the recording and work during it."""
    path = os.environ.get("DEMO_GO")
    while path and not os.path.exists(path):
        PANE.wait(0.2)
    PANE.wait(delay)


def opens_pr(title, body, fails=None):
    """Commit the work and open its pull request through `gh` (forge.py on
    this take's PATH), as a real agent would, printing the PR's URL into
    the transcript for the board to read."""
    subprocess.run(["git", "add", "-A"], cwd=CWD, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "commit", "-qm", title], cwd=CWD,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tool("Bash", "Commit", command=f"git commit -am '{title}'")
    env = {**os.environ, "MSMN_FORGE_FAILS": fails} if fails else os.environ
    url = subprocess.run(["gh", "pr", "create", "--title", title, "--body", body],
                         cwd=CWD, capture_output=True, text=True, env=env).stdout.strip()
    tool("Bash", "Open a pull request", result=url + "\n",
         command=f"git push -u origin HEAD && gh pr create --title '{title}' --body '{body}'")
    return url


def limiter(first):
    """The pull-request clip's clean worker: works, opens its PR, stops."""
    submitted(first)
    text("Adding a token bucket per client IP in front of /shorten.")
    go(1.5)
    tool("Read", "Read store.py", file_path=os.path.join(CWD, "store.py"))
    tool("Edit", "Add the limiter", file_path=os.path.join(CWD, "limiter.py"))
    with open(os.path.join(CWD, "limiter.py"), "w") as f:
        f.write(LIMITER_PY)
    tool("Bash", "Run the tests", command="python3 -m pytest -q")
    url = opens_pr("Rate limit /shorten per client IP",
                   "A token bucket per client IP: 20 a minute, 429 past that.")
    text(f"Opened #{url.rsplit('/', 1)[-1]}: a token bucket per IP, 20 a minute, 429 "
         "past that.", last=True)
    hook("Stop", stop_hook_active=False)


def slash(first):
    """The pull-request clip's other worker: its PR fails its pipeline, the
    crown sends it back, it fixes and pushes."""
    submitted(first)
    text("A trailing slash falls through to 404. Normalising the path first.")
    go(4)
    tool("Read", "Read store.py", file_path=os.path.join(CWD, "store.py"))
    tool("Grep", "Find where a slug resolves", pattern="def resolve")
    tool("Edit", "Strip the trailing slash", file_path=os.path.join(CWD, "store.py"))
    with open(os.path.join(CWD, "store.py"), "a") as f:
        f.write("\n\ndef normalise(path):\n    return path.rstrip('/') or '/'\n")
    tool("Bash", "Run the tests", command="python3 -m pytest -q")
    tool("Edit", "Add a test for /abc/", file_path=os.path.join(CWD, "test_store.py"))
    url = opens_pr("Redirect /abc/ like /abc", "Strip the trailing slash before resolving.",
                   fails="tests")
    text(f"Opened #{url.rsplit('/', 1)[-1]}: /abc/ resolves like /abc.", last=True)
    hook("Stop", stop_hook_active=False)

    submitted(PANE.wait())
    text("The test expects 301 for /abc/ and the redirect answers 302. Fixing it.")
    tool("Edit", "Answer 301 for a normalised path", file_path=os.path.join(CWD, "store.py"))
    with open(os.path.join(CWD, "store.py"), "a") as f:
        f.write("\n\nREDIRECT = 301\n")
    tool("Bash", "Run the tests", command="python3 -m pytest -q")
    subprocess.run(["git", "commit", "-qam", "Answer 301 for a normalised path"], cwd=CWD,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["gh", "__push"], cwd=CWD, stdout=subprocess.DEVNULL)
    tool("Bash", "Push the fix", command="git push")
    text("Pushed: 301 for /abc/, tests green.", last=True)
    hook("Stop", stop_hook_active=False)


LIMITER_PY = '''"""A token bucket per client IP, in front of /shorten."""

import time

RATE = 20 / 60  # links a minute
BURST = 20
_buckets = {}


def allow(ip, now=None):
    now = time.monotonic() if now is None else now
    level, at = _buckets.get(ip, (BURST, now))
    level = min(BURST, level + (now - at) * RATE)
    if level < 1:
        _buckets[ip] = (level, now)
        return False
    _buckets[ip] = (level - 1, now)
    return True
'''


def noted(first):
    """Read the code, leave a note on this ticket through the board's own
    tool, and stop."""
    submitted(first)
    text("I'll find where links are kept before changing anything.")
    tool("Read", "Read store.py", file_path=os.path.join(CWD, "store.py"))
    tool("Grep", "Find where a slug resolves", pattern="def links")
    board = Board()
    board.call("write_note", text=NOTE)
    board.close()
    text("Left a note on the ticket: where the check goes, and why 410.", last=True)
    hook("Stop", stop_hook_active=False)


def seen(board, key):
    """The stamp a keyed edit carries: proof the crown read the card first."""
    return board.call("get_ticket", key=key)["seen"]


def crown_tool(board, tool, doing, /, **arguments):
    """One crown edit, in the transcript and the pane as claude shows it."""
    use_id = f"toolu_{uuid.uuid4().hex[:24]}"
    record("assistant", [{"type": "tool_use", "id": use_id, "name": f"mcp__mesimon__{tool}",
                          "input": arguments}])
    say(f"  \x1b[1m{tool}\x1b[0m  {doing}")
    answer = board.call(tool, **arguments)
    record("user", [{"type": "tool_result", "tool_use_id": use_id,
                     "content": json.dumps(answer)}])
    PANE.wait(1.2)
    return answer


# A working agent's first words and its steps, by its ticket's title.
WORK = {
    "Rate limiting": ("Adding a token bucket per client IP in front of /shorten.",
                      [("Read", "Read server.py"), ("Edit", "Add the limiter"),
                       ("Bash", "Run the tests"), ("Edit", "Tune the burst")]),
    "Expire old links": ("Stamping each link's last click, so old ones can expire.",
                         [("Read", "Read store.py"), ("Edit", "Add last_clicked"),
                          ("Bash", "Run the tests"), ("Edit", "Add the sweep")]),
}


def busy(first):
    """Work without end, and answer what is typed in between: the reply,
    then the step it asked for, then the work goes on."""
    submitted(first)
    words, steps = WORK.get(first.splitlines()[0].strip(),
                            ("Working on it.", [("Read", "Read the code"),
                                                ("Edit", "Make the change")]))
    text(words)
    for n in range(10_000):
        asked = tool(*steps[n % len(steps)]) or PANE.wait(2)
        while asked:
            submitted(asked)
            PANE.wait(0.6)
            if asked == ASIDE:
                text(ASIDE_REPLY)
                asked = tool("Edit", "Log each 429")
            else:
                text("Noted; carrying on.")
                asked = None


STATS = '''"""`shortlink stats`: click counts per link, as a table or as JSON."""

import json
import sys

from store import links


def main(argv=sys.argv[1:]):
    rows = sorted(links(), key=lambda link: -link.clicks)
    if "--json" in argv:
        print(json.dumps([FIELDS for link in rows]))
        return
    print(f"{'slug':<10} {'clicks':>7}  target")
    for link in rows:
        print(f"{link.slug:<10} {link.clicks:>7}  {link.target}")


if __name__ == "__main__":
    main()
'''


SLUGS_PY = '''"""A slug a request names, or a random one."""

import secrets

RESERVED = RESERVED_VALUE


def choose(wanted=None):
    if wanted is None:
        return secrets.token_urlsafe(4)
    if wanted in RESERVED:
        raise ValueError(f"{wanted} is reserved")  # 409 at the handler
    return wanted
'''


def write_slugs(reserved):
    with open(os.path.join(CWD, "slugs.py"), "w") as f:
        f.write(SLUGS_PY.replace("RESERVED_VALUE", '{"admin", "api"}' if reserved else "set()"))


def write_json_flag(expired):
    fields = '{"slug": link.slug, "clicks": link.clicks, "target": link.target'
    fields += ', "expired": link.expired}' if expired else "}"
    with open(os.path.join(CWD, "stats.py"), "w") as f:
        f.write(STATS.replace("FIELDS", fields))


def main():
    global PANE
    # A pane title, as claude sets one: the board's startup probe reads a
    # titleless pane that stays quiet as a stuck startup dialog. The board
    # shows it as the session's name, so it says what this is.
    sys.stdout.write("\x1b]0;demo stand-in\x07")
    # Raw before SessionStart: the daemon pastes the prompt on that edge,
    # and a pane still in cooked mode would echo it onto the screen.
    PANE = Pane()
    hook("SessionStart", reason="startup", source="startup")
    first = PANE.wait()
    try:
        flows = {DEMO: demo, CROWN: crown, NOTED: noted, SLUGS: slugs,
                 SPRINT: sprint, LIMITER: limiter, SLASH: slash}
        flow = next((f for title, f in flows.items() if first.startswith(title)), None)
        # "Rate limiting" is the busy agent of the other clips; it opens a
        # pull request only where there is a forge to open it on.
        if flow is limiter and not os.environ.get("MSMN_FORGE"):
            flow = None
        (flow or busy)(first)
    except Exception:
        # The pane stays up with the reason, for the person re-recording.
        say(traceback.format_exc())
        while True:
            PANE.wait()
    done_worker()


def done_worker():
    """A worker whose work is done, at its prompt: the merge train asks a
    branch that fell behind main to rebase, and that is done for real;
    anything else typed in gets a nod."""
    while True:
        words = PANE.wait()
        submitted(words)
        if os.environ.get("DEMO_WAKES"):
            with open(os.environ["DEMO_WAKES"], "a") as f:
                f.write(f"[worker] {words}\n")
        if "rebase" in words.lower():
            subprocess.run(["git", "rebase", "main"], cwd=CWD,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            tool("Bash", "Rebase onto main", command="git rebase main")
            text("Rebased onto main; the tests still pass.", last=True)
        elif "merged" in words.lower():
            # The board's word that the branch landed.
            text("Landed on main; nothing left on this ticket.", last=True)
        else:
            text("Noted.", last=True)
        hook("Stop", stop_hook_active=False)


if __name__ == "__main__":
    main()
