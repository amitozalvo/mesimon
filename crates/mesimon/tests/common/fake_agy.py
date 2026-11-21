#!/usr/bin/env python3
"""A stand-in for agy (T-1225, T-1233, T-1235): it paints agy's screens, takes
its keys, and runs the hooks and the title command a mesimon launch registered,
in the shapes agy 1.5.8–1.5.13 draws and sends.

`agy models` (T-1234) prints what 1.5.9 prints, a progress line and then
`id<TAB>label` lines, counts the call in `agy-models.log` and exits.

Otherwise it reads `--add-dir <folder>` and `--settings <file>` off its argv, as agy
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
drive the rest: `ask`, `answer`, `stop`, `quit`, and `dialog.json` (T-1235:
runs `PreToolUse` with `toolCall {id, name, args}`, opens the captured
confirmation or `ask_question` screen, walks keys, and logs the outcome to
`agy-decisions.jsonl`). Anything a hook or the title command prints goes to
`agy-stdout.jsonl`, one line per handler that printed, with its event: agy
reads a printed `{}` on `PreToolUse` as a refusal, so `PreToolUse` printing
any bytes denies the call instead of opening the dialog.
"""
import json
import os
import re
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
EMAIL = "person@example.invalid"


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


def transcript(conversation):
    return os.path.join(HERE, "agy-brain", conversation, ".system_generated", "logs",
                        "transcript_full.jsonl")


def step(conversation, record):
    path = transcript(conversation)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    with open(path, "a") as out:
        out.write(json.dumps({"status": "DONE", "created_at": at, **record}) + "\n")


def log(name, record):
    with open(os.path.join(HERE, name), "a") as out:
        out.write(json.dumps(record) + "\n")


def models():
    log("agy-models.log", "models")
    override = os.path.join(HERE, "agy-models.txt")
    if os.path.exists(override):
        print("Fetching available models...")
        text = open(override).read()
        if text:
            sys.stdout.write(text)
        return
    print("Fetching available models...")
    print("gemini-3.8-flash-high\tGemini 3.8 Flash (High)")
    print("gemini-3.8-flash-low\tGemini 3.8 Flash (Low)")


def build_confirm_dialog(spec):
    name, args = spec["name"], spec["args"]
    cur = spec.get("initial_cursor", 0)
    if name == "run_command":
        cmd = args["CommandLine"]
        first = cmd.split()[0] if cmd.split() else cmd
        bypass = args.get("BypassSandbox") is True
        prompt_lines = (
            ["🔓 Allow sandbox bypass for command execution?",
             "⚠️ Confirm the command is safe to run outside of the sandbox with full network and disk access.",
             ""]
            if bypass else ["Run this command?"]
        )
        return {
            "kind": "confirm",
            "id": spec["id"],
            "name": name,
            "esc_disabled": False,
            "header": "Command",
            "preview": ["Requesting permission for:", "   " + cmd, ""] + prompt_lines,
            "choices": [
                "Yes, run command",
                f"Yes, and always allow in this conversation for commands that start with '{first}'",
                f"Yes, and always allow for commands that start with '{first}' (Persist to settings.json)",
                "No, cancel",
            ],
            "nav": "  ↑/↓ Navigate · tab Amend · ctrl+g edit/expand command",
            "cursor": cur,
        }
    if name in ("write_to_file", "write_file"):
        path = args.get("TargetFile") or args.get("Path")
        return {
            "kind": "confirm",
            "id": spec["id"],
            "name": name,
            "esc_disabled": False,
            "header": "Create file",
            "preview": [f"{path}  +1", "  1  +  hello world", "Allow creation of this file?"],
            "choices": ["Yes, allow creation", "No, deny creation"],
            "nav": "  ↑/↓ Navigate · f full diff",
            "cursor": cur,
        }
    if name in ("replace_file_content", "multi_replace_file_content", "sed_file", "edit_file"):
        path = (args.get("TargetFile") or args.get("target_file")
                or args.get("Path") or args.get("File"))
        choices = ["Yes, accept this change"]
        if spec.get("auto_mode_choice"):
            choices.append("Yes, and use Auto mode until you exit (auto-approve edits and safe commands)")
        choices.append("No, reject this change")
        return {
            "kind": "confirm",
            "id": spec["id"],
            "name": name,
            "esc_disabled": True,
            "header": "Pending edit",
            "preview": [
                f"{path}  +1 -1",
                "  1  -  old",
                "  1  +  new",
                "  shift+tab to auto-approve file edits",
                "Accept this file edit?",
            ],
            "choices": choices,
            "nav": "  ↑/↓ Navigate · f full diff",
            "cursor": cur,
        }
    if name in ("view_file", "read_file"):
        path = args.get("AbsolutePath") or args.get("Path")
        return {
            "kind": "confirm",
            "id": spec["id"],
            "name": name,
            "esc_disabled": False,
            "header": "File access",
            "preview": [f"Read: {path}", "Reason: outside workspace", "Allow access to this file?"],
            "choices": [
                "Yes, allow access",
                "Yes, and always allow non-workspace access",
                "No, deny access",
            ],
            "nav": "  ↑/↓ Navigate",
            "cursor": cur,
        }
    if name == "call_mcp_tool":
        server, tool = args["ServerName"], args["ToolName"]
        return {
            "kind": "confirm",
            "id": spec["id"],
            "name": name,
            "esc_disabled": False,
            "header": "MCP",
            "preview": [
                "Requesting permission for:",
                f"   {tool} (server: {server})",
                "",
                "Allow calling this tool?",
            ],
            "choices": [
                "Yes, allow tool call",
                f"Yes, and always allow tool '{server}/{tool}' in this conversation",
                f"Yes, and always allow tool '{server}/{tool}' (Persist to settings.json)",
                "No, deny tool call",
            ],
            "nav": "  ↑/↓ Navigate",
            "cursor": cur,
        }
    raise ValueError(f"unsupported fake_agy tool: {name}")


def build_question_dialog(spec):
    qs = spec["args"]["questions"]
    return {
        "kind": "question",
        "id": spec["id"],
        "name": "ask_question",
        "questions": qs,
        "q_idx": 0,
        "cursor": 0,
        "ticks": [[False] * len(q["options"]) for q in qs],
        "write_in": None,
        "answers": [],
    }


def main():
    if sys.argv[1:] == ["models"]:
        return models()
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
        "tool_dialog": None,
        "yes": True,
        "working": False,
        "queued": [],
        "history": [],
        "footer": "? for shortcuts",
        "ctrl_c": False,
        "steps": 0,
        "words": "",
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
        return done

    def pretool(name, args):
        payload = {
            "conversationId": st["conversation"] or MINTED,
            "workspacePaths": [os.getcwd(), folder],
            "modelName": "fake",
            "hookEventName": "PreToolUse",
            "cwd": os.getcwd(),
            "toolCall": {"id": "call_gate", "name": name, "args": args},
            "stepIdx": st["steps"],
        }
        runs = []
        for group in hooks.get("PreToolUse", []):
            matcher = group.get("matcher", "")
            if matcher in ("", "*") or re.search(f"^(?:{matcher})$", name):
                for handler in group.get("hooks", []):
                    runs.append(run(handler["command"], payload))
        code = next((d.returncode for d in runs if d.returncode != 0), 0)
        stdout = b"".join(d.stdout for d in runs).decode(errors="replace")
        log("agy-gate.jsonl", {"tool": name, "code": code, "stdout": stdout})

    def hook(event, extra):
        payload = {
            "conversationId": st["conversation"],
            "workspacePaths": [os.getcwd(), folder],
            "modelName": "fake",
            "hookEventName": event,
            "cwd": os.getcwd(),
        }
        if st["conversation"]:
            payload["transcriptPath"] = transcript(st["conversation"])
        payload.update(extra)
        tool_name = payload.get("toolCall", {}).get("name", "")
        printed = []
        for entry in hooks.get(event, []):
            if "hooks" in entry:
                matcher = entry.get("matcher", "")
                if tool_name and not (matcher in ("", "*") or re.search(f"^(?:{matcher})$", tool_name)):
                    continue
                handlers = entry["hooks"]
            else:
                handlers = [entry]
            for handler in handlers:
                done = run(handler["command"], payload)
                if done.stdout:
                    printed.append(done.stdout)
        return printed

    def report(agent_state, pending=False):
        payload = {
            "agent_state": agent_state,
            "conversation_id": st["conversation"] or "",
            "version": "fake",
            # agy's own wrong path: the CLI writes nothing under it.
            "transcript_path": os.path.join(HERE, "antigravity", "brain", "transcript.jsonl"),
            "context_window": {"context_window_size": 1048576 if st["steps"] else 0,
                               "current_usage": None},
            "quota": {"gemini-5h": {"remaining_fraction": 0.75, "reset_time": "2099-01-01T00:00:00Z",
                                    "reset_in_seconds": 3600},
                      "gemini-weekly": {"remaining_fraction": 0.9,
                                        "reset_time": "2099-01-07T00:00:00Z",
                                        "reset_in_seconds": 86400}},
            "plan_tier": "Google AI Pro",
            "email": EMAIL,
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
        elif st["tool_dialog"] is not None:
            td = st["tool_dialog"]
            if td["kind"] == "confirm":
                rows = [td["header"], rule, ""] + list(td["preview"])
                for idx, ch in enumerate(td["choices"]):
                    mark = "> " if idx == td["cursor"] else "  "
                    rows.append(f"{mark}{idx + 1}. {ch}")
                rows.append(td["nav"])
                rows.append("esc to cancel")
            else:
                q_idx = td["q_idx"]
                total = len(td["questions"])
                q = td["questions"][q_idx]
                multi = bool(q.get("is_multi_select"))
                opts = q["options"]
                rows = ["Question", rule, "", f"Question {q_idx + 1}/{total}: {q['question']}", ""]
                for idx, opt in enumerate(opts):
                    mark = "> " if idx == td["cursor"] else "  "
                    if multi:
                        box = "[x] " if td["ticks"][q_idx][idx] else "[ ] "
                    else:
                        box = ""
                    rows.append(f"{mark}{idx + 1}. {box}{opt}")
                w_mark = "> " if td["cursor"] == len(opts) else "  "
                rows.append(f"{w_mark}{len(opts) + 1}. Write-in...")
                rows.append("")
                if td["write_in"] is not None:
                    rows.append("  Your answer:")
                    rows.append("  " + td["write_in"])
                    rows.append("")
                    rows.append("  enter Submit · esc Back")
                else:
                    if td["cursor"] == len(opts):
                        act = "enter Write-in"
                    elif multi:
                        act = "space Toggle · enter Submit"
                    elif q_idx + 1 == total and total > 1:
                        act = "enter Submit All"
                    else:
                        act = "enter Select"
                    rows.append(f"  ↑/↓ Navigate · {act} · esc Skip")
                rows.append("esc to cancel")
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
        step(st["conversation"], {"step_index": st["steps"], "source": "USER_EXPLICIT",
                                  "type": "USER_INPUT",
                                  "content": "<USER_REQUEST>\n" + words + "\n</USER_REQUEST>\n"
                                  "<ADDITIONAL_METADATA>\nfake\n</ADDITIONAL_METADATA>"})
        st["steps"] += 1
        st["words"] = words
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
        st["tool_dialog"] = None
        st["footer"] = "? for shortcuts"
        step(st["conversation"], {"step_index": st["steps"], "source": "MODEL",
                                  "type": "PLANNER_RESPONSE", "model": "gemini-3.8-flash-high",
                                  "input_tokens": 2000, "cache_read_tokens": 12000,
                                  "output_tokens": 30, "content": "done: " + st["words"]})
        st["steps"] += 1
        hook("Stop", {"terminationReason": "NO_TOOL_CALL", "fullyIdle": True, "error": ""})
        report("idle")
        if st["queued"]:
            begin(st["queued"].pop(0), "queued")

    def finish_question(td):
        log("agy-decisions.jsonl", {"id": td["id"], "name": "ask_question", "answers": td["answers"]})
        st["tool_dialog"] = None
        paint()
        report("working")
        hook("PreInvocation", {"invocationNum": 1, "lastUserInput": ""})

    def advance_question(td, ans):
        td["answers"].append(ans)
        if td["q_idx"] + 1 < len(td["questions"]):
            td["q_idx"] += 1
            td["cursor"] = 0
            td["write_in"] = None
        else:
            finish_question(td)

    def handle_tool_key(td, token, pasted=None):
        if td["kind"] == "confirm":
            n = len(td["choices"])
            if token == "up":
                td["cursor"] = max(0, td["cursor"] - 1)
            elif token == "down":
                td["cursor"] = min(n - 1, td["cursor"] + 1)
            elif token == "enter":
                chosen = td["choices"][td["cursor"]]
                if "Persist to settings.json" in chosen:
                    home = os.environ.get("MESIMON_AGY_HOME")
                    if home:
                        os.makedirs(home, exist_ok=True)
                        with open(os.path.join(home, "settings.json"), "w") as bad:
                            bad.write('{"poisoned_by_option_3": true}')
                log("agy-decisions.jsonl", {
                    "id": td["id"],
                    "name": td["name"],
                    "choice_index": td["cursor"],
                    "choice": chosen,
                })
                st["tool_dialog"] = None
                paint()
                report("working")
            elif token == "esc" and not td["esc_disabled"]:
                log("agy-decisions.jsonl", {"id": td["id"], "name": td["name"], "choice": "esc"})
                st["tool_dialog"] = None
                st["working"] = False
                st["footer"] = "? for shortcuts"
                paint()
                report("idle")
            return

        # ask_question dialog
        q_idx = td["q_idx"]
        q = td["questions"][q_idx]
        opts = q["options"]
        multi = bool(q.get("is_multi_select"))
        if td["write_in"] is not None:
            if pasted is not None:
                td["write_in"] += pasted.replace("\r", "").replace("\n", "")
            elif token == "esc":
                td["write_in"] = None
            elif token == "enter":
                advance_question(td, td["write_in"] or "User Skipped")
            elif token == "char":
                pass
            return
        if token == "up":
            td["cursor"] = max(0, td["cursor"] - 1)
        elif token == "down":
            td["cursor"] = min(len(opts), td["cursor"] + 1)
        elif token == "space" and multi and td["cursor"] < len(opts):
            td["ticks"][q_idx][td["cursor"]] = not td["ticks"][q_idx][td["cursor"]]
        elif token == "enter":
            if td["cursor"] == len(opts):
                td["write_in"] = ""
            elif multi:
                picked = [o for i, o in enumerate(opts) if td["ticks"][q_idx][i]]
                advance_question(td, ", ".join(picked))
            else:
                advance_question(td, opts[td["cursor"]])
        elif token == "esc":
            advance_question(td, "User Skipped")

    def key(data):
        """One read's worth of keys; returns False to exit."""
        i = 0
        while i < len(data):
            if data.startswith(PASTE_START, i):
                end = data.find(PASTE_END, i)
                end = len(data) if end < 0 else end
                text = data[i + len(PASTE_START) : end].decode("utf-8", "replace")
                if st["tool_dialog"] is not None:
                    handle_tool_key(st["tool_dialog"], "paste", text)
                else:
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
            if st["tool_dialog"] is not None:
                td = st["tool_dialog"]
                if data.startswith(b"\x1b[A", i):
                    handle_tool_key(td, "up")
                    i += 3
                    continue
                if data.startswith(b"\x1b[B", i):
                    handle_tool_key(td, "down")
                    i += 3
                    continue
                if data.startswith(b"\x1b[", i):
                    i += 3
                    continue
                byte = data[i : i + 1]
                if byte == b"\x1b":
                    handle_tool_key(td, "esc")
                elif byte in (b"\r", b"\n"):
                    handle_tool_key(td, "enter")
                elif byte == b" ":
                    handle_tool_key(td, "space")
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
                if pending.rfind(PASTE_START) > pending.rfind(PASTE_END):
                    continue
                data, pending = pending, b""
                if not key(data):
                    return
                paint()
            for cue in sorted(os.listdir(CUES)):
                cue_path = os.path.join(CUES, cue)
                if cue == "dialog.json":
                    spec = json.load(open(cue_path))
                    os.remove(cue_path)
                    printed = hook("PreToolUse", {
                        "stepIdx": st["steps"],
                        "toolCall": {
                            "id": spec["id"],
                            "name": spec["name"],
                            "args": spec["args"],
                        },
                    })
                    # Measured trap: any stdout on PreToolUse (including `{}` or
                    # a gate refusal) denies the call instead of asking the person.
                    if printed:
                        log("agy-decisions.jsonl", {
                            "id": spec["id"],
                            "name": spec["name"],
                            "choice": "hook_denied",
                        })
                        continue
                    if spec["name"] == "ask_question":
                        st["tool_dialog"] = build_question_dialog(spec)
                        paint()
                        report("tool_use", pending=False)
                    else:
                        st["tool_dialog"] = build_confirm_dialog(spec)
                        paint()
                        report("tool_use", pending=True)
                    continue
                body = open(cue_path).read()
                os.remove(cue_path)
                if cue == "ask":
                    report("tool_use", pending=True)
                elif cue == "answer":
                    report("working")
                elif cue == "stop":
                    stop()
                    paint()
                elif cue.startswith("gate"):
                    spec = json.loads(body)
                    pretool(spec["name"], spec["args"])
                elif cue == "quit":
                    return
    finally:
        out.write("\x1b[?2004l")
        out.flush()
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)


main()
