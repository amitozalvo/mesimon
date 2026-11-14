#!/usr/bin/env python3
"""A stand-in for agy (T-1225): it runs the hooks and the title command a
mesimon launch registered, on cue, in the payload shapes agy 1.5.8 sends.

It reads `--add-dir <folder>` and `--settings <file>` off its argv, as agy
does, appends that argv to `agy-argv.jsonl` beside itself, and then watches
`agy-cue/` beside itself for cue files: `prompt`, `ask`, `answer`, `stop`,
`quit`. Anything a hook or the title command prints goes to
`agy-stdout.jsonl`, one line per handler that printed, with its event: agy
reads a printed `{}` on a tool hook as a refusal, so a test asserts the
brief's `SessionStart` handler is the only one there (T-1232).

At launch it starts the server `.agents/mcp_config.json` names, as agy does
when it first lists its servers, and sends what agy 1.5.9 sends:
`server/discover` (protocol 2026-07-28), then `initialize` (2025-11-25),
`notifications/initialized` and `tools/list`. The replies go to
`agy-mcp.json`.
"""
import json
import os
import select
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CUES = os.path.join(HERE, "agy-cue")
MINTED = "11111111-2222-4333-8444-555555555555"


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


def main():
    os.makedirs(CUES, exist_ok=True)
    with open(os.path.join(HERE, "agy-argv.jsonl"), "a") as log:
        log.write(json.dumps(sys.argv[1:]) + "\n")
    folder, settings = value("--add-dir"), value("--settings")
    hooks = json.load(open(os.path.join(folder, ".agents", "hooks.json")))["mesimon"]
    title = json.load(open(settings))["title"]["command"]
    state = {"conversation": value("--conversation")}

    def run(command, payload):
        done = subprocess.run(
            command,
            shell=True,
            cwd=os.path.join(folder, ".agents"),
            input=json.dumps(payload).encode(),
            stdout=subprocess.PIPE,
        )
        if done.stdout:
            line = {"event": payload.get("hookEventName", "title"), "command": command,
                    "stdout": done.stdout.decode(errors="replace")}
            with open(os.path.join(HERE, "agy-stdout.jsonl"), "a") as out:
                out.write(json.dumps(line) + "\n")

    def hook(event, extra):
        payload = {
            "conversationId": state["conversation"],
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
            "conversation_id": state["conversation"] or "",
            "version": "fake",
        }
        if pending:
            payload["tool_confirmation_pending"] = True
        run(title, payload)

    mcp(folder)
    report("authenticating")
    report("idle")
    print("fake agy ready", flush=True)
    deadline = time.time() + 120
    while time.time() < deadline:
        for cue in sorted(os.listdir(CUES)):
            os.remove(os.path.join(CUES, cue))
            if cue == "prompt":
                fresh = state["conversation"] is None
                state["conversation"] = state["conversation"] or MINTED
                if fresh:
                    hook("SessionStart", {})
                hook("PreInvocation", {"invocationNum": 0, "initialNumSteps": 1})
                report("working")
            elif cue == "ask":
                report("tool_use", pending=True)
            elif cue == "answer":
                report("working")
            elif cue == "stop":
                hook("Stop", {"terminationReason": "NO_TOOL_CALL", "fullyIdle": True, "error": ""})
                report("idle")
            elif cue == "quit":
                return
        time.sleep(0.05)


main()
