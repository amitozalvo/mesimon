#!/usr/bin/env python3
"""A scripted stand-in for `claude`, for the README recording only.

record.sh points MESIMON_CLAUDE_BIN here, so the daemon launches this with
exactly the argv it gives claude (`--settings`, `--mcp-config`,
`--session-id`, ...) and pastes the prompt into its pane the same way. It
answers through the same two roads a real session does: the hook commands
named in its settings file, and the `mesimon mcp` tool shim named in its MCP
config. Nothing on the board is faked -- only the model is.

What it does is keyed off the ticket title in its first prompt (seed.py):
the demo ticket works, raises its hand with a question, takes the answer,
commits and stops; the crown clip's ticket works until a person crowns it,
then runs the board through the crown's tools; any other ticket just keeps
working.
"""

import json
import os
import select
import subprocess
import sys
import time
import traceback
import uuid

DEMO = "Stats as JSON"
QUESTION = "Include expired links?"
CROWN = "Plan 1.0"


def flag(name):
    return sys.argv[sys.argv.index(name) + 1]


SETTINGS = json.load(open(flag("--settings")))
SESSION = flag("--session-id")
MCP = next(iter(json.loads(flag("--mcp-config"))["mcpServers"].values()))
CWD = os.getcwd()
HOME = os.environ.get("MESIMON_CLAUDE_HOME", os.path.expanduser("~/.claude"))
SLUG = "".join(c if c.isalnum() else "-" for c in CWD)
TRANSCRIPT = os.path.join(HOME, "projects", SLUG, f"{SESSION}.jsonl")


def say(text=""):
    sys.stdout.write(text.replace("\n", "\r\n") + "\r\n")
    sys.stdout.flush()


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


def tool(name, description, until=None, **tool_input):
    """A tool call: its transcript lines, its hook, a line in the pane. It
    runs 1.2 s, or with `until`, up to 2 s and no longer than that is false."""
    use_id = f"toolu_{uuid.uuid4().hex[:24]}"
    record("assistant", [{"type": "tool_use", "id": use_id, "name": name,
                          "input": {"description": description, **tool_input}}])
    say(f"  ⏺ {name}  {description}")
    if until is None:
        time.sleep(1.2)
    else:
        end = time.time() + 2
        while time.time() < end and not until():
            time.sleep(0.25)
    record("user", [{"type": "tool_result", "tool_use_id": use_id, "content": "ok"}])
    hook("PostToolUse", tool_name=name, tool_input=tool_input, tool_use_id=use_id)


def text(words, last=False):
    extra = {"stop_reason": "end_turn"} if last else {}
    record("assistant", [{"type": "text", "text": words}], **extra)
    say(f"  {words}")


def submitted(words):
    record("user", words)
    hook("UserPromptSubmit", prompt=words)
    say(f"> {words.splitlines()[0]}")


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


class Keys:
    """The pane's input: bracketed pastes, and the Enter that submits one."""

    def __init__(self):
        import termios
        import tty
        # TCSANOW, not setraw's default flush: the daemon types the title
        # into the pane before this process is reading.
        tty.setraw(0, termios.TCSANOW)
        sys.stdout.write("\x1b[?2004h")  # the daemon pastes with -p
        sys.stdout.flush()
        self.buf = b""

    def prompt(self):
        """Block until a non-empty prompt is submitted; return its text."""
        typed = b""
        while True:
            select.select([0], [], [])
            self.buf += os.read(0, 65536)
            while self.buf:
                if self.buf.startswith(b"\x1b[200~"):
                    end = self.buf.find(b"\x1b[201~")
                    if end == -1:
                        break
                    typed += self.buf[6:end]
                    self.buf = self.buf[end + 6:]
                elif self.buf[:1] in (b"\r", b"\n"):
                    self.buf = self.buf[1:]
                    if typed.strip():
                        return typed.decode(errors="replace").strip()
                else:
                    typed += self.buf[:1]
                    self.buf = self.buf[1:]


def demo(keys, first):
    submitted(first)
    text("I'll add a --json flag to `shortlink stats`.")
    tool("Read", "Read stats.py", file_path=os.path.join(CWD, "stats.py"))
    tool("Edit", "Add the --json flag", file_path=os.path.join(CWD, "stats.py"))
    write_json_flag(expired=False)
    tool("Bash", "Run the stats tests", command="python3 -m pytest -q")
    text(f"--json works. {QUESTION} They are hidden from the table today.", last=True)
    raise_hand(QUESTION)
    hook("Stop", stop_hook_active=False)

    submitted(keys.prompt())
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
    steps = ["Read the open tickets", "Check the release checklist"]
    n = 0
    # Nothing is typed into a pane when a person crowns its ticket (the
    # crown adds no token to the conversation); a model finds out by
    # asking, so between steps this asks every quarter second.
    crowned = lambda: board.call("get_ticket")["crowned"]
    while not crowned():
        tool("Read", steps[n % len(steps)], until=crowned)
        n += 1

    keys = {t["title"]: t["key"] for t in board.call("list_board")["tickets"]}
    text("I wear the crown now. Filing what 1.0 is missing.")
    filed = crown_tool(board, "create_ticket", "File: health check endpoint",
                       title="Health check endpoint", column="TODO",
                       description="GET /healthz answers 200 while the store is reachable.")
    notes = crown_tool(board, "create_ticket", "File: 1.0 release notes",
                       title="1.0 release notes", column="TODO",
                       description="What changed since 0.9, for the release page.")
    stale = keys["Custom slugs"]
    crown_tool(board, "move_ticket", f"Move {stale} back to TODO: nobody is on it",
               key=stale, to_column="TODO", seen=seen(board, stale))
    crown_tool(board, "tag_ticket", f"Tag {notes['key']} MILESTONE",
               key=notes["key"], name="MILESTONE")
    expire = keys["Expire old links"]
    started = crown_tool(board, "start_agent", f"Start an agent on {expire}",
                         key=expire, seen=seen(board, expire))
    left = started["budget_left"]
    text(f"Filed {filed['key']} and {notes['key']}, moved {stale} back to TODO, tagged "
         f"{notes['key']} MILESTONE and started an agent on {expire}: "
         f"{left} {'start' if left == 1 else 'starts'} left in the crown's budget.",
         last=True)
    hook("Stop", stop_hook_active=False)


def seen(board, key):
    """The stamp a keyed edit carries: proof the crown read the card first."""
    return board.call("get_ticket", key=key)["seen"]


def crown_tool(board, tool, doing, /, **arguments):
    """One crown edit, in the transcript and the pane as claude shows it."""
    use_id = f"toolu_{uuid.uuid4().hex[:24]}"
    record("assistant", [{"type": "tool_use", "id": use_id, "name": f"mcp__mesimon__{tool}",
                          "input": arguments}])
    say(f"  ⏺ {tool}  {doing}")
    answer = board.call(tool, **arguments)
    record("user", [{"type": "tool_result", "tool_use_id": use_id,
                     "content": json.dumps(answer)}])
    time.sleep(1.2)
    return answer


# A working agent's first words and its steps, by its ticket's title.
WORK = {
    "Rate limiting": ("Adding a token bucket per client IP in front of /shorten.",
                      ["Read server.py", "Add the limiter", "Run the tests", "Tune the burst"]),
    "Expire old links": ("Stamping each link's last click, so old ones can expire.",
                         ["Read store.py", "Add last_clicked", "Run the tests", "Add the sweep"]),
}


def busy(first):
    submitted(first)
    words, steps = WORK.get(first.splitlines()[0].strip(),
                            ("Working on it.", ["Read the code", "Make the change"]))
    text(words)
    for n in range(10_000):
        tool("Bash" if n % 2 else "Edit", steps[n % len(steps)])
        time.sleep(2)


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


def write_json_flag(expired):
    fields = '{"slug": link.slug, "clicks": link.clicks, "target": link.target'
    fields += ', "expired": link.expired}' if expired else "}"
    with open(os.path.join(CWD, "stats.py"), "w") as f:
        f.write(STATS.replace("FIELDS", fields))


def main():
    # A pane title, as claude sets one: the board's startup probe reads a
    # titleless pane that stays quiet as a stuck startup dialog.
    sys.stdout.write("\x1b]0;✳ Claude Code\x07")
    say("\x1b[2mdemo stand-in for claude: scripted, no model (assets/demo)\x1b[0m")
    say()
    hook("SessionStart", reason="startup", source="startup")
    keys = Keys()
    first = keys.prompt()
    try:
        if first.startswith(DEMO):
            demo(keys, first)
        elif first.startswith(CROWN):
            crown(first)
        else:
            busy(first)
    except Exception:
        # The pane stays up with the reason, for the person re-recording.
        say(traceback.format_exc())
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    main()
