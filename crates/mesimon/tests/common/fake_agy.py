#!/usr/bin/env python3
"""A stand-in for agy (T-1225, T-1233): it paints agy's screens, takes its
keys, and runs the hooks and the title command a mesimon launch registered,
in the shapes agy 1.5.9 draws and sends.

It reads `--add-dir <folder>` and `--settings <file>` off its argv, as agy
does, and appends that argv to `agy-argv.jsonl` beside itself. A folder
the settings' `trustedWorkspaces` does not name exactly gets the trust
dialog first (`Do you trust the contents of this project?`, the cursor `>`
on Yes, the feed reading `initializing` while it stands); `No, exit` exits.
Then the box: a rule, `> ` and the input (a placeholder before the cursor
while it is empty), a rule, the footer. Keys, in raw mode: a bracketed
paste goes into the box whole, Enter submits it, Ctrl+U clears it, a first
Ctrl+C only warns ("press ctrl+c again to exit") and a second exits, and,
mid-turn, Enter queues the words for a turn of their own after the `Stop`,
alt+m hands them to the running turn, and Esc interrupts it with no `Stop`.

Every prompt a turn ran is appended to `agy-prompts.jsonl` as
`{"how": "turn" | "queued" | "steer", "text": ...}`, and the box's text is
written to `agy-box.txt` on each change. Cue files in `agy-cue/` beside it
drive the rest: `ask`, `answer`, `stop`, `quit`. Anything a hook or the title
command prints goes to `agy-stdout.jsonl`, one line per handler that printed,
with its event: agy reads a printed `{}` on a tool hook as a refusal, so a
test asserts the brief's `SessionStart` handler is the only one there
(T-1232).

At launch it starts the server `.agents/mcp_config.json` names, as agy does
when it first lists its servers, and sends what agy 1.5.9 sends:
`server/discover` (protocol 2026-07-28), then `initialize` (2025-11-25),
`notifications/initialized` and `tools/list`. The replies go to
`agy-mcp.json`.
"""
import json
import os
import select
import shutil
import subprocess
import sys
import termios
import time
import tty

HERE = os.path.dirname(os.path.abspath(__file__))
CUES = os.path.join(HERE, "agy-cue")
MINTED = "11111111-2222-4333-8444-555555555555"
PLACEHOLDER = "Auto mode: edits & safe commands auto-approved (shift+tab to cycle)"
PASTE_START, PASTE_END = b"\x1b[200~", b"\x1b[201~"


def value(flag):
    args = sys.argv[1:]
    return args[args.index(flag) + 1] if flag in args else None


def mcp(folder):
    path = os.path.join(folder, ".agents", "mcp_config.json")
    if not os.path.exists(path):
        return
    server = json.load(open(path))["mcpServers"]["mesimon"]
    child = subprocess.Popen([server["command"], *server["args"]], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    replies = {}

    def send(message):
        child.stdin.write((json.dumps(message) + "\n").encode())
        child.stdin.flush()

    def ask(rid, method, params):
        send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        # A server that never answers is agy waiting forever: bounded here.
        if not select.select([child.stdout], [], [], 10)[0]:
            replies[method] = "no reply in 10 s"
            return
        replies[method] = json.loads(child.stdout.readline())

    meta = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "antigravity-client", "version": "v1.0.0"}}
    ask(1, "server/discover", {"_meta": meta})
    ask(2, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                          "clientInfo": {"name": "antigravity-client", "version": "v1.0.0"}})
    send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
    ask(3, "tools/list", {})
    child.kill()
    child.wait()
    with open(os.path.join(HERE, "agy-mcp.json"), "w") as out:
        json.dump(replies, out)


def log(name, record):
    with open(os.path.join(HERE, name), "a") as out:
        out.write(json.dumps(record) + "\n")


def main():
    os.makedirs(CUES, exist_ok=True)
    log("agy-argv.jsonl", sys.argv[1:])
    folder, settings_path = value("--add-dir"), value("--settings")
    hooks = json.load(open(os.path.join(folder, ".agents", "hooks.json")))["mesimon"]
    settings = json.load(open(settings_path))
    title = settings["title"]["command"]
    here = os.path.realpath(os.getcwd())
    trusted = any(os.path.realpath(t) == here for t in settings.get("trustedWorkspaces", []))
    st = {
        "conversation": value("--conversation"),
        "box": "",
        "dialog": not trusted,
        "yes": True,
        "working": False,
        "queued": [],
        "history": [],
        "footer": "? for shortcuts",
        "ctrl_c": False,
    }

    def run(command, payload):
        done = subprocess.run(
            command,
            shell=True,
            cwd=os.path.join(folder, ".agents"),
            input=json.dumps(payload).encode(),
            stdout=subprocess.PIPE,
        )
        if done.stdout:
            log("agy-stdout.jsonl", {"event": payload.get("hookEventName", "title"),
                                     "command": command,
                                     "stdout": done.stdout.decode(errors="replace")})

    def hook(event, extra):
        payload = {
            "conversationId": st["conversation"],
            "workspacePaths": [os.getcwd(), folder],
            "modelName": "fake",
            "hookEventName": event,
            "cwd": os.getcwd(),
        }
        payload.update(extra)
        for handler in hooks.get(event, []):
            run(handler["command"], payload)

    def report(agent_state, pending=False):
        payload = {
            "agent_state": agent_state,
            "conversation_id": st["conversation"] or "",
            "version": "fake",
        }
        if pending:
            payload["tool_confirmation_pending"] = True
        run(title, payload)

    out = sys.stdout

    def paint():
        width = max(shutil.get_terminal_size((80, 24)).columns, 40)
        rule = "─" * width
        rows = []
        cursor = (0, 0)
        if st["dialog"]:
            rows = [
                "Accessing workspace:",
                "",
                os.getcwd(),
                "",
                "Do you trust the contents of this project?",
                "",
                "Antigravity CLI requires permission to read, edit, and execute files here.",
                "",
                ("> " if st["yes"] else "  ") + "Yes, I trust this folder",
                ("  " if st["yes"] else "> ") + "No, exit",
                "",
                "  ↑/↓ Navigate · enter Confirm",
            ]
        else:
            rows = ["  fake agy 1.5.9", ""] + st["history"][-8:]
            for words in st["queued"]:
                rows.append("▸ " + words.replace("\n", " ") + " (alt+m to send immediately)")
            rows.append(rule)
            lines = st["box"].split("\n") if st["box"] else [""]
            top = len(rows)
            for i, line in enumerate(lines):
                rows.append(("> " if i == 0 else "  ") + line)
            if st["box"]:
                cursor = (len(rows) - 1, 2 + len(lines[-1]))
            else:
                rows[top] = "> " + PLACEHOLDER
                cursor = (top, 2)
            rows.append(rule)
            rows.append(st["footer"])
        frame = "\x1b[H\x1b[2J" + "\r\n".join(rows)
        frame += "\x1b[%d;%dH" % (cursor[0] + 1, cursor[1] + 1)
        out.write(frame)
        out.flush()
        with open(os.path.join(HERE, "agy-box.txt"), "w") as box:
            box.write(st["box"])

    def begin(words, how):
        fresh = st["conversation"] is None
        st["conversation"] = st["conversation"] or MINTED
        if fresh:
            hook("SessionStart", {"lastUserInput": words})
        st["working"] = True
        st["footer"] = "esc to cancel"
        st["history"].append("> " + words.replace("\n", " "))
        log("agy-prompts.jsonl", {"how": how, "text": words})
        hook("PreInvocation", {"invocationNum": 0, "initialNumSteps": 1, "lastUserInput": words})
        report("working")

    def submit():
        words = st["box"]
        if not words.strip():
            return
        st["box"] = ""
        if st["working"]:
            st["queued"].append(words)
            st["footer"] = "  Press up to edit queued messages"
        else:
            begin(words, "turn")

    def stop():
        if not st["working"]:
            return
        st["working"] = False
        st["footer"] = "? for shortcuts"
        hook("Stop", {"terminationReason": "NO_TOOL_CALL", "fullyIdle": True, "error": ""})
        report("idle")
        if st["queued"]:
            begin(st["queued"].pop(0), "queued")

    def key(data):
        """One read's worth of keys; returns False to exit."""
        i = 0
        while i < len(data):
            if data.startswith(PASTE_START, i):
                end = data.find(PASTE_END, i)
                end = len(data) if end < 0 else end
                text = data[i + len(PASTE_START) : end].decode("utf-8", "replace")
                st["box"] += text.replace("\r\n", "\n").replace("\r", "\n")
                i = end + len(PASTE_END)
                continue
            if st["dialog"]:
                if data.startswith(b"\x1b[A", i) or data.startswith(b"\x1b[B", i):
                    st["yes"] = not st["yes"]
                    i += 3
                    continue
                if data[i : i + 1] in (b"\r", b"\n"):
                    if not st["yes"]:
                        return False
                    st["dialog"] = False
                    report("idle")
                i += 1
                continue
            if data.startswith(b"\x1bm", i):
                if st["working"] and st["box"].strip():
                    log("agy-prompts.jsonl", {"how": "steer", "text": st["box"]})
                    st["box"] = ""
                    hook("PreInvocation", {"invocationNum": 1, "lastUserInput": ""})
                elif st["box"].strip():
                    submit()
                i += 2
                continue
            if data.startswith(b"\x1b[", i):
                i += 3
                continue
            byte = data[i : i + 1]
            if byte == b"\x1b":
                if st["working"]:
                    st["working"] = False
                    st["history"].append("  ⎿  Interrupted · What should Antigravity CLI do instead?")
                    st["footer"] = "? for shortcuts"
                    report("idle")
            elif byte in (b"\r", b"\n"):
                submit()
            elif byte == b"\x15":
                st["box"] = st["box"].rsplit("\n", 1)[0] if "\n" in st["box"] else ""
            elif byte == b"\x03":
                if st["ctrl_c"]:
                    return False
                st["ctrl_c"] = True
                st["footer"] = "press ctrl+c again to exit"
            elif byte == b"\x7f":
                st["box"] = st["box"][:-1]
            elif byte >= b" ":
                # One UTF-8 character at a time.
                j = i + 1
                while j < len(data) and (data[j] & 0xC0) == 0x80:
                    j += 1
                st["box"] += data[i:j].decode("utf-8", "replace")
                i = j
                continue
            i += 1
        return True

    mcp(folder)
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    tty.setraw(fd)
    out.write("\x1b[?2004h")
    try:
        report("authenticating")
        if st["dialog"]:
            report("initializing")
        else:
            report("idle")
        paint()
        pending = b""
        deadline = time.time() + 120
        while time.time() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if ready:
                pending += os.read(fd, 65536)
                # A paste split across reads waits for its end.
                if pending.rfind(PASTE_START) > pending.rfind(PASTE_END):
                    continue
                data, pending = pending, b""
                if not key(data):
                    return
                paint()
            for cue in sorted(os.listdir(CUES)):
                os.remove(os.path.join(CUES, cue))
                if cue == "ask":
                    report("tool_use", pending=True)
                elif cue == "answer":
                    report("working")
                elif cue == "stop":
                    stop()
                    paint()
                elif cue == "quit":
                    return
    finally:
        out.write("\x1b[?2004l")
        out.flush()
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)


main()
