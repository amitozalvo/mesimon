#!/usr/bin/env python3
"""The parity harness's stand-in for mesimon's mod (T-574).

An e2e stub cannot load a Claude Code mod, so under the mod road the harness
starts this beside the stub, inside the pane, where it inherits what the mod
would read: the `MESIMON_MOD_*` variables the daemon sets on a mod launch,
and tmux's `TMUX`/`TMUX_PANE`. It does what `register.ts`'s bridge loop does:

- runs the REAL `mesimon mod-bridge --speaks ping,submit,answer,fill`, stdin
  closed, and reads its stdout;
- drops a frame it has seen (delivery is at least once), records each new
  one as a line in `<fixture dir>/mod-<session>.ndjson`, and answers a
  `ping` with a `ModPong` relayed through the REAL `mesimon hook --road mod`;
- takes a `submit` (T-575) the way the engine hands a prompt to the model:
  the stub stands in for the model, and its stdin is what it reads, so the
  words go into its own pane (bracketed paste, then Enter, through the tmux
  the wrapper names) and a `ModSubmit` `entered` is relayed. It sends no
  `UserPromptSubmit`: a test acks a prompt itself, on both roads alike;
- takes a `fill` (T-601) the way the mod puts words in an empty composer
  for the daemon's send-now: the words go into its own pane (bracketed
  paste, NO Enter) and a `ModFill` `filled` is relayed, after which the
  daemon presses the send-now keys into the pane itself. A `mod-draft`
  file next to it stands for a person's draft in the box: the fill is
  refused (`draft`) and nothing is typed;
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
  `dropped` and delivers nothing), `mod-submit-aside` one whose engine takes
  a submit to the model past the box, as Claude Code 2.1.309 does (T-931:
  a draft in the box stays there, unsent), the words appended to
  `submitted.txt` and never typed; `mod-hold-entered` one whose engine holds
  a submit's result as Claude Code 2.1.309 holds a plugin's prompt to the
  running turn's end (T-938): the words are typed as ever, and `entered` is
  relayed only once a `mod-release` file appears (reading `dropped`, a late
  refusal instead: read at once, so a test writes it whole); `mod-queue-submit`
  one whose engine QUEUES a submit as Claude Code 2.1.310 queues a plugin's
  prompt behind the running turn (T-1025): nothing is typed until a
  `mod-release` file appears, and then the words go in and `entered` is
  relayed, or, the file reading `removed`, the engine's words for a queue a
  person's Esc cleared are relayed as `dropped` and nothing is typed (any
  switch file may be suffixed `-<session>` to stand for one pane's mod
  alone); and
  `mod-late` one whose
  bridge first polls the milliseconds its text names after the engine
  starts (T-879);
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


def deliver(text, enter=True):
    """The words into this pane, as `paste_text` puts them there; without
    the Enter, as a fill leaves them in the composer."""
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
    if not enter:
        return paste.returncode == 0
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


def held_entered(frame_id):
    """`entered` once the test releases it (`mod-hold-entered`); a release
    file reading `dropped` reports the engine's refusal instead, late.
    `mod-release-<frame id>` releases that frame alone (T-990)."""
    release = os.path.join(HERE, "mod-release")
    own = os.path.join(HERE, f"mod-release-{frame_id}")
    while not os.path.exists(release) and not os.path.exists(own):
        time.sleep(0.05)
    if os.path.exists(own):
        release = own
    with open(release) as f:
        dropped = f.read().strip() == "dropped"
    relay("ModSubmit", frame_id,
          {"outcome": "dropped", "reason": "a hook blocked it"} if dropped else {"outcome": "entered"})


def held_queue(frame_id, text):
    """A submit queued behind the running turn (`mod-queue-submit`, T-1025):
    typed and `entered` once the test releases it; a release reading
    `removed` is Claude Code's own report for a queue a person's Esc
    cleared, and the words reach nobody."""
    release = os.path.join(HERE, "mod-release")
    own = os.path.join(HERE, f"mod-release-{frame_id}")
    while not os.path.exists(release) and not os.path.exists(own):
        time.sleep(0.05)
    if os.path.exists(own):
        release = own
    with open(release) as f:
        removed = f.read().strip() == "removed"
    if removed:
        relay("ModSubmit", frame_id, {"outcome": "dropped",
              "reason": "the prompt was removed from the queue before it ran"})
        return
    ok = deliver(text)
    relay("ModSubmit", frame_id,
          {"outcome": "entered"} if ok else {"outcome": "rejected", "error": "no pane"})


def switch(name):
    """A switch file next to the stand-in, for every pane of the fixture,
    or `<name>-<session>` for this pane alone (T-1025: a crown's wake must
    not meet the worker's queued engine)."""
    return os.path.exists(os.path.join(HERE, name)) or os.path.exists(
        os.path.join(HERE, f"{name}-{SESSION}"))


def speaks():
    try:
        with open(os.path.join(HERE, "mod-speaks")) as f:
            return f.read().strip()
    except OSError:
        return "ping,submit,answer,fill"


def main():
    if switch("mod-silent"):
        return
    threading.Thread(target=watch_parent, daemon=True).start()
    with open(ENGINE_PID, "w") as f:
        f.write(str(os.getpid()))
    try:
        with open(os.path.join(HERE, "mod-late")) as f:
            time.sleep(int(f.read().strip()) / 1000)
    except (OSError, ValueError):
        pass
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
            if kind == "submit" and switch("mod-hold-submit"):
                # T-936: the submit stays pending until the test lets it go,
                # so a person can come into the pane meanwhile.
                deadline = time.time() + 20
                while not switch("mod-release-submit") and time.time() < deadline:
                    time.sleep(0.05)
            if kind == "ping":
                relay("ModPong", frame_id, {})
            elif kind == "submit" and switch("mod-drop-submit"):
                relay("ModSubmit", frame_id, {"outcome": "dropped", "reason": "a hook blocked it"})
            elif kind == "submit" and switch("mod-stopped-submit"):
                # Claude Code 2.1.308's words for a plugin's input after a
                # person's Esc (T-894): held until the person types.
                relay("ModSubmit", frame_id, {"outcome": "dropped",
                      "reason": "the session was stopped and nobody has sent input since"})
            elif kind == "submit" and switch("mod-submit-aside"):
                with open(os.path.join(HERE, "submitted.txt"), "a") as f:
                    f.write((frame.get("text") or "") + "\n")
                relay("ModSubmit", frame_id, {"outcome": "entered"})
            elif kind == "submit" and switch("mod-hold-entered"):
                deliver(frame.get("text") or "")
                threading.Thread(target=held_entered, args=(frame_id,), daemon=True).start()
            elif kind == "submit" and switch("mod-queue-submit"):
                threading.Thread(target=held_queue, args=(frame_id, frame.get("text") or ""),
                                 daemon=True).start()
            elif kind == "submit":
                ok = deliver(frame.get("text") or "")
                relay("ModSubmit", frame_id,
                      {"outcome": "entered"} if ok else {"outcome": "rejected", "error": "no pane"})
            elif kind == "fill" and switch("mod-draft"):
                relay("ModFill", frame_id, {"outcome": "refused", "reason": "draft"})
            elif kind == "fill":
                ok = deliver(frame.get("text") or "", enter=False)
                relay("ModFill", frame_id,
                      {"outcome": "filled"} if ok else {"outcome": "refused", "error": "no pane"})
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
