#!/usr/bin/env python3
"""The parity harness's stand-in for mesimon's mod (T-574).

An e2e stub cannot load a Claude Code mod, so under the mod road the harness
starts this beside the stub, inside the pane, where it inherits what the mod
would read: the `MESIMON_MOD_*` variables the daemon sets on a mod launch,
and tmux's `TMUX`/`TMUX_PANE`. It does what `register.ts`'s bridge loop does:

- runs the REAL `mesimon mod-bridge`, stdin closed, and reads its stdout;
- drops a frame it has seen (delivery is at least once), records each new
  one as a line in `<fixture dir>/mod-<session>.ndjson`, and answers a
  `ping` with a `ModPong` relayed through the REAL `mesimon hook --road mod`;
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


def main():
    threading.Thread(target=watch_parent, daemon=True).start()
    with open(ENGINE_PID, "w") as f:
        f.write(str(os.getpid()))
    seen = []
    backoff = 1.0
    while True:
        born = time.time()
        child = subprocess.Popen(
            [BIN, "mod-bridge", "--sock", ORCH_SOCK, "--session", SESSION],
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
            if frame.get("kind") == "ping":
                relay("ModPong", frame_id, {})
        code = child.wait()
        if code == BRIDGE_REFUSED_EXIT:
            return
        if time.time() - born >= 60:
            backoff = 1.0
        time.sleep(backoff)
        backoff = min(backoff * 2, 60.0)


main()
