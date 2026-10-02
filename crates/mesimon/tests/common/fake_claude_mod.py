#!/usr/bin/env python3
"""The parity harness's stand-in for mesimon's mod (T-574).

An e2e stub cannot load a Claude Code mod, so under the mod road the harness
starts this beside the stub, inside the pane, where it inherits what the mod
would read: the `MESIMON_MOD_*` variables the daemon sets on a mod launch,
and tmux's `TMUX`/`TMUX_PANE`. It does what `register.ts`'s bridge loop does:

- runs the REAL `mesimon mod-bridge --speaks ping,submit,answer`, stdin
  closed, and reads its stdout;
- drops a frame it has seen (delivery is at least once), records each new
  one as a line in `<fixture dir>/mod-<session>.ndjson`, and answers a
  `ping` with a `ModPong` relayed through the REAL `mesimon hook --road mod`;
- takes a `submit` (T-575) the way the engine hands a prompt to the model:
  the stub stands in for the model, and its stdin is what it reads, so the
  words go into its own pane (bracketed paste, then Enter, through the tmux
  the wrapper names) and a `ModSubmit` `entered` is relayed. It sends no
  `UserPromptSubmit`: a test acks a prompt itself, on both roads alike;
- takes an `answer` (T-576) the way the mod returns one in the dialog's
  place: `fake_claude_dialog.py`'s dialog, when one stands, is closed with
  the answers it would have written, and a `ModAnswer` `answered` is relayed
  with the `PostToolUse`-shaped body (no `PostToolUse` fires for it). A
  `mod-person-answered-<session>` file next to it stands for the person
  having answered first: the answer finds nothing held;
- and, for `mod_turns_e2e`, three files next to it stand for three other
  mods: `mod-silent` one that never comes up (the engine exits at once),
  `mod-speaks` an older one (its text is the `--speaks` list), and
  `mod-drop-submit` one whose engine drops every submit (it reports
  `dropped` and delivers nothing);
- respawns a bridge that died, with the mod's backoff, and not one that was
  refused for good (exit 3, `BRIDGE_REFUSED_EXIT`);
- exits when its parent (the stub, standing in for Claude Code) goes.

The relay of the hook set's events is the harness's other half: under the
mod road `hook_send` sends each event's mod twin itself, before the hook
set's frame, so a test's order stays its own.
"""
import json
import os
import subprocess
import sys
import threading
import time

ENV = os.environ
SESSION = ENV.get("MESIMON_MOD_SESSION")
TICKET = ENV.get("MESIMON_TICKET", "none")
TMUX_BIN = ENV.get("MESIMON_FAKE_TMUX", "tmux")
if not SESSION:
    sys.exit(0)
BIN = ENV["MESIMON_MOD_BIN"]
HOOK_SOCK = ENV["MESIMON_MOD_HOOK_SOCK"]
ORCH_SOCK = ENV["MESIMON_MOD_ORCH_SOCK"]
HERE = os.path.dirname(os.path.abspath(__file__))
RECORD = os.path.join(HERE, f"mod-{SESSION}.ndjson")
BRIDGE_PID = os.path.join(HERE, f"mod-bridge-{SESSION}.pid")
ENGINE_PID = os.path.join(HERE, f"mod-engine-{SESSION}.pid")
BRIDGE_REFUSED_EXIT = 3
PARENT = os.getppid()


def watch_parent():
    while True:
        time.sleep(0.5)
        if os.getppid() != PARENT:
            os._exit(0)


def relay(event, reason, body):
    argv = [BIN, "hook", "--sock", HOOK_SOCK, "--session", SESSION, "--event", event]
    if reason is not None:
        argv += ["--reason", reason]
    argv += ["--road", "mod"]
    subprocess.run(
        argv,
        input=json.dumps(body).encode(),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=5,
        check=False,
    )


def deliver(text):
    """The words into this pane, as `paste_text` puts them there."""
    sock = ENV.get("TMUX", "").split(",")[0]
    pane = ENV.get("TMUX_PANE")
    if not sock or not pane:
        return False
    tmux = [TMUX_BIN, "-S", sock]
    buf = f"msmn-mod-{SESSION[:8]}"
    load = subprocess.run(tmux + ["load-buffer", "-b", buf, "-"], input=text.encode(),
                          capture_output=True, check=False)
    if load.returncode != 0:
        return False
    paste = subprocess.run(tmux + ["paste-buffer", "-p", "-b", buf, "-d", "-t", pane],
                           capture_output=True, check=False)
    time.sleep(0.05)
    enter = subprocess.run(tmux + ["send-keys", "-t", pane, "Enter"], capture_output=True,
                           check=False)
    return paste.returncode == 0 and enter.returncode == 0


def answer(frame):
    call = frame.get("tool_use_id") or ""
    answers = frame.get("answers") or {}
    if os.path.exists(os.path.join(HERE, f"mod-person-answered-{SESSION}")):
        relay("ModAnswer", "nothing_held", {"tool_use_id": call})
        return
    dialog = os.path.join(HERE, f"dialog-{TICKET}.json")
    questions = None
    try:
        with open(dialog) as f:
            questions = json.load(f).get("questions")
        with open(os.path.join(HERE, f"answered-{TICKET}.json"), "w") as f:
            json.dump(answers, f)
        os.remove(dialog)
    except (OSError, ValueError, AttributeError):
        pass
    result = {"questions": questions, "answers": answers}
    relay("ModAnswer", "answered", {
        "hook_event_name": "PostToolUse",
        "tool_name": "AskUserQuestion",
        "tool_use_id": call,
        "tool_input": {"questions": questions},
        "tool_response": result,
    })


def switch(name):
    return os.path.exists(os.path.join(HERE, name))


def speaks():
    try:
        with open(os.path.join(HERE, "mod-speaks")) as f:
            return f.read().strip()
    except OSError:
        return "ping,submit,answer"


def main():
    if switch("mod-silent"):
        return
    threading.Thread(target=watch_parent, daemon=True).start()
    with open(ENGINE_PID, "w") as f:
        f.write(str(os.getpid()))
    seen = []
    backoff = 1.0
    while True:
        born = time.time()
        child = subprocess.Popen(
            [BIN, "mod-bridge", "--sock", ORCH_SOCK, "--session", SESSION,
             "--speaks", speaks()],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
        )
        with open(BRIDGE_PID, "w") as f:
            f.write(str(child.pid))
        for raw in child.stdout:
            line = raw.decode().strip()
            if not line:
                continue
            try:
                frame = json.loads(line)
            except ValueError:
                continue
            frame_id = frame.get("id")
            if not isinstance(frame_id, str) or frame_id in seen:
                continue
            seen.append(frame_id)
            del seen[:-64]
            with open(RECORD, "a") as f:
                f.write(line + "\n")
            kind = frame.get("kind")
            if kind == "ping":
                relay("ModPong", frame_id, {})
            elif kind == "submit" and switch("mod-drop-submit"):
                relay("ModSubmit", frame_id, {"outcome": "dropped", "reason": "a hook blocked it"})
            elif kind == "submit":
                ok = deliver(frame.get("text") or "")
                relay("ModSubmit", frame_id,
                      {"outcome": "entered"} if ok else {"outcome": "rejected", "error": "no pane"})
            elif kind == "answer":
                answer(frame)
        code = child.wait()
        if code == BRIDGE_REFUSED_EXIT:
            return
        if time.time() - born >= 60:
            backoff = 1.0
        time.sleep(backoff)
        backoff = min(backoff * 2, 60.0)


main()
