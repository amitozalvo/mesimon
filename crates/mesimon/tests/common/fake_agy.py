#!/usr/bin/env python3
"""A stand-in for agy (T-1225): it runs the hooks and the title command a
mesimon launch registered, on cue, in the payload shapes agy 1.5.8 sends.

It reads `--add-dir <folder>` and `--settings <file>` off its argv, as agy
does, appends that argv to `agy-argv.jsonl` beside itself, and then watches
`agy-cue/` beside itself for cue files: `prompt`, `ask`, `answer`, `stop`,
`quit`. Anything a hook or the title command prints goes to
`agy-stdout.log`: agy reads a printed `{}` on a tool hook as a refusal, so a
test asserts the file stays empty.
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CUES = os.path.join(HERE, "agy-cue")
MINTED = "11111111-2222-4333-8444-555555555555"


def value(flag):
    args = sys.argv[1:]
    return args[args.index(flag) + 1] if flag in args else None


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
            with open(os.path.join(HERE, "agy-stdout.log"), "ab") as out:
                out.write(done.stdout)

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
