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
commits and stops; any other ticket just keeps working.
"""

import json
import os
import select
import subprocess
import sys
import time
import uuid

DEMO = "Stats as JSON"
QUESTION = "Include expired links?"


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


def tool(name, description, **tool_input):
    """A tool call: its transcript lines, its hook, a line in the pane."""
    use_id = f"toolu_{uuid.uuid4().hex[:24]}"
    record("assistant", [{"type": "tool_use", "id": use_id, "name": name,
                          "input": {"description": description, **tool_input}}])
    say(f"  ⏺ {name}  {description}")
    time.sleep(1.2)
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


def raise_hand(reason):
    """Call the board's `raise_hand` tool through the MCP shim, as claude does."""
    shim = subprocess.Popen([MCP["command"], *MCP["args"]], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    msgs = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "demo-stub", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
         "params": {"name": "raise_hand", "arguments": {"reason": reason}}},
    ]
    for m in msgs:
        shim.stdin.write((json.dumps(m) + "\n").encode())
        shim.stdin.flush()
        if "id" in m:
            shim.stdout.readline()
    shim.stdin.close()
    shim.wait(timeout=5)


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


def busy(first):
    submitted(first)
    text("Adding a token bucket per client IP in front of /shorten.")
    steps = ["Read server.py", "Add the limiter", "Run the tests", "Tune the burst"]
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
    if first.startswith(DEMO):
        demo(keys, first)
    else:
        busy(first)
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    main()
