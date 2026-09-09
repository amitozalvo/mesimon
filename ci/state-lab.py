#!/usr/bin/env python3
"""Isolated before/after Mesimon boards. Synthetic replay costs no model tokens.

The supervisor owns both daemons and their private tmux servers, and reaps only
those at stop or the one-hour deadline. Run the printed board launchers from an
ordinary terminal. Live evidence is collected separately with claude-capture.py.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
LAB = ROOT / "target/state-lab"


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def paths(repo):
    identity = hashlib.sha256(os.fsencode(repo.resolve())).hexdigest()[:16]
    return Path(f"/tmp/mesimon-{os.getuid()}/{identity}"), Path.home() / ".local/state/mesimon" / identity


class Client:
    def __init__(self, repo):
        runtime, _ = paths(repo)
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(8)
        self.sock.connect(str(runtime / "orch.sock"))
        self.file = self.sock.makefile("rb")
        self.request("hello", version=1, client="state-lab")

    def request(self, cmd, **fields):
        self.sock.sendall((json.dumps({"principal": {"kind": "local"}, "command": {"cmd": cmd, **fields}}) + "\n").encode())
        response = json.loads(self.file.readline())
        if response.get("resp") in ("error", "denied"):
            raise RuntimeError(response)
        return response

    def board(self):
        return self.request("snapshot")["board"]

    def close(self):
        self.file.close()
        self.sock.close()


def hook(repo, sid, event, payload=None, reason=None):
    runtime, _ = paths(repo)
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(5)
        sock.connect(str(runtime / "hook.sock"))
        header = {"v": 1, "session": sid, "event": event, "reason": reason}
        sock.sendall((json.dumps(header) + "\n" + json.dumps(payload or {})).encode())
        sock.shutdown(socket.SHUT_WR)


def wait_for(test, timeout=20):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            result = test()
            if result:
                return result
        except (OSError, ValueError):
            pass
        time.sleep(0.1)
    raise TimeoutError("lab checkpoint not reached")


def cases():
    return [
        ("normal-stop", "Finished turn", "Stop", {}, "idle", "end_turn", "REVIEW"),
        ("continued-stop", "Finished after Stop-hook continuation", "Stop", {"stop_hook_active": True}, "idle", "end_turn", "REVIEW"),
        ("question", "Waiting for your answer", "PreToolUse", {"tool_name": "AskUserQuestion"}, "requires_action", "question", "IN PROGRESS"),
        ("plan", "Waiting for plan approval", "PermissionRequest", {"tool_name": "ExitPlanMode"}, "requires_action", "plan", "IN PROGRESS"),
        ("permission-allow", "Waiting for tool permission", "PermissionRequest", {"tool_name": "Bash"}, "requires_action", "permission", "IN PROGRESS"),
        ("elicitation", "Waiting for MCP input", "Elicitation", {}, "requires_action", "elicitation", "IN PROGRESS"),
        ("authentication_failed", "Needs authentication", "StopFailure", {"error": "authentication_failed"}, "requires_action", "auth", "IN PROGRESS"),
        ("rate-limit", "Rate limited", "StopFailure", {"error": "rate_limit"}, "throttled", None, "IN PROGRESS"),
        ("server_error", "API failure", "StopFailure", {"error": "server_error"}, "failed", "server", "IN PROGRESS"),
        ("background-shell", "Waiting for a background build", "Stop", {"background_tasks": [{"type": "shell"}]}, "idle", "background", "IN PROGRESS"),
        ("dormant-monitor", "Finished with a dormant monitor", "Stop", {"background_tasks": [{"type": "monitor"}]}, "idle", "end_turn", "REVIEW"),
        ("teammate-working", "Waiting for a busy teammate", "Stop", {"background_tasks": [{"type": "teammate"}]}, "idle", "background", "IN PROGRESS"),
        ("teammate-idle", "Finished with an idle teammate", "Stop", {"background_tasks": [{"type": "teammate"}]}, "idle", "end_turn", "REVIEW"),
        ("nested-stop", "Child finished; parent still working", "Stop", {"agent_id": "child"}, "running", None, "IN PROGRESS"),
    ]


def seed(repo, home, binary):
    client = Client(repo)
    entries = []
    client.request("set_mcp_tools", on=False)
    for case, title, event, payload, state, reason, column in cases():
        created = client.request("create_ticket", column="IN PROGRESS", title=title, workspace=None)
        ticket = created["id"]
        spawned = client.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)
        sid = spawned["id"]
        transcript = repo / f"{sid}.jsonl"
        transcript.write_text("")
        hook(repo, sid, "SessionStart", {"session_id": sid, "source": "startup", "transcript_path": str(transcript)})
        wait_for(lambda: next(s for s in client.board()["sessions"] if s["id"] == sid)["state"]["state"] == "idle")
        hook(repo, sid, "UserPromptSubmit", {"session_id": sid})
        wait_for(lambda: next(s for s in client.board()["sessions"] if s["id"] == sid)["state"]["state"] == "running")
        if case == "teammate-idle":
            hook(repo, sid, "TeammateIdle", {"teammate_name": "reviewer"})
        hook(repo, sid, event, payload)
        expected = {"state": state, "column": column, "reason": reason}
        note = client.request("write_note", ticket=ticket, text=f"# {case}\n\nSynthetic, repeatable state scenario. No model is running in this pane.\n\nExpected: {state} / {reason or 'none'}, column {column}.\n\nThe corresponding replay fixture covers the transitions around this checkpoint.\n\nInspect evidence with:\n\n    {binary} state explain {sid[:8]} --repo {repo}\n")
        entries.append({"case": case, "ticket": ticket, "session": sid, "expected": expected, "note": note.get("note")})
    # Observe-only sessions exercise actual census, tail cursor and recovery.
    project = home / "projects/lab"
    project.mkdir(parents=True)
    for case, title, content, stop, state, reason in [
        ("modern-transcript-end", "Recovered modern completed transcript", [{"type": "text", "text": "LAB_DONE"}], "end_turn", "idle", "end_turn"),
        ("confidence-confirmation", "Real Stop confirms an inferred finish", [{"type": "text", "text": "LAB_DONE"}], "end_turn", "idle", "end_turn"),
        ("mixed-text-tool", "Silent tool with accompanying text", [{"type": "text", "text": "Running the tool"}, {"type": "tool_use", "id": "lab-tool", "name": "Bash", "input": {"command": "sleep 90"}}], "tool_use", "running", None),
    ]:
        sid = str(uuid.uuid4())
        transcript = project / f"{sid}.jsonl"
        records = [
            {"uuid": "prompt", "type": "user", "sessionId": sid, "cwd": str(repo), "version": "2.1.266", "message": {"content": "LAB synthetic scenario"}},
            {"uuid": "reply", "type": "assistant", "sessionId": sid, "cwd": str(repo), "message": {"content": content, "stop_reason": stop}},
        ]
        if case == "confidence-confirmation":
            records.append({"uuid": "legacy-close", "type": "system", "subtype": "turn_duration"})
        transcript.write_text("".join(json.dumps(r) + "\n" for r in records))
        os.utime(transcript, (time.time() - 90, time.time() - 90))
        ticket = client.request("create_ticket", column="IN PROGRESS", title=title, workspace=None)["id"]
        client.request("rescan_external")
        attached = client.request("attach_external", claude_session_id=sid, ticket=ticket)
        actual = next(s for s in client.board()["sessions"] if s["ticket"] == ticket)
        expected_column = "REVIEW" if case == "confidence-confirmation" else "IN PROGRESS"
        if case == "confidence-confirmation":
            wait_for(lambda: next(s for s in client.board()["sessions"] if s["id"] == actual["id"])["state"] == {"state": "idle", "stop_reason": "end_turn"})
            hook(repo, actual["id"], "Stop", {"stop_hook_active": False, "background_tasks": []})
        client.request("write_note", ticket=ticket, text=f"# {case}\n\nSynthetic transcript observed through production census/tailing.\n\nExpected: {state} / {reason}, {expected_column}.\n\nThis is an observe-only session, so it has no agent pane. An outstanding tool must stay working when its transcript is quiet. Confirming an inferred end with a real Stop must update confidence and movement.\n")
        entries.append({"case": case, "ticket": ticket, "session": actual["id"], "expected": {"state": state, "reason": reason, "column": expected_column}})
    client.close()
    return entries


def snapshot_result(repo, entries):
    client = Client(repo)
    board = client.board()
    client.close()
    result = []
    for entry in entries:
        session = next(s for s in board["sessions"] if s["id"] == entry["session"])
        ticket = next(t for t in board["tickets"] if t["id"] == entry["ticket"])
        state = session["state"]
        actual = {"state": state["state"], "reason": state.get("reason", state.get("stop_reason")), "column": ticket["column"]}
        result.append({"case": entry["case"], "session": session["id"], "expected": entry["expected"],
                       "actual": actual, "confidence": session["confidence"], "passed": actual == entry["expected"]})
    return result


def serve(run, lifetime):
    os.umask(0o077)
    stopping = False
    def stop(_sig, _frame):
        nonlocal stopping
        stopping = True
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    children = []
    manifest = {"schema": 1, "owner": "mesimon-state-lab", "supervisor_pid": os.getpid(),
                "created_at": time.time(), "deadline": time.time() + lifetime, "run": str(run), "boards": {}, "status": "starting"}
    write(run / "lab.json", manifest)
    try:
        tmux = shutil.which("tmux")
        if not tmux:
            raise RuntimeError("tmux is required")
        for side in ("before", "after"):
            repo = run / side
            repo.mkdir()
            subprocess.run(["git", "init", "-q", str(repo)], check=True, timeout=10)
            home = run / (side + "-claude-home")
            home.mkdir()
            stub = run / (side + "-agent.py")
            stub.write_text(f"#!{sys.executable}\nimport signal\nprint('Mesimon state lab: synthetic agent, no model calls', flush=True)\nsignal.pause()\n")
            stub.chmod(0o700)
            binary = LAB / "bin" / ("mesimon-" + side)
            if not binary.is_file():
                raise RuntimeError(f"missing binary: {binary}")
            env = {k: v for k, v in os.environ.items() if not k.startswith("MESIMON_") and k not in ("TMUX", "TMUX_PANE")}
            env.update(MESIMON_CLAUDE_BIN=str(stub), MESIMON_CLAUDE_HOME=str(home),
                       MESIMON_HOOK_BIN=str(binary), MESIMON_TMUX_BIN=tmux,
                       MESIMON_NO_DAEMON_AUTORESTART="1", MESIMON_PANE_QUIET_MS="3600000")
            logfile = (run / (side + "-daemon.log")).open("w")
            child = subprocess.Popen([str(binary), "daemon", "--repo", str(repo)], env=env,
                                     stdout=logfile, stderr=subprocess.STDOUT, start_new_session=True)
            children.append((child, repo, logfile, tmux))
            runtime, state = paths(repo)
            wait_for(lambda: (runtime / "orch.sock").exists() and child.poll() is None)
            entries = seed(repo, home, binary)
            manifest["boards"][side] = {"repo": str(repo), "binary": str(binary), "pid": child.pid,
                                        "runtime": str(runtime), "state_dir": str(state), "entries": entries}
            launcher = run / ("open-" + side + ".sh")
            launcher.write_text("#!/bin/sh\ncd " + shlex.quote(str(repo)) + "\nexec env MESIMON_NO_DAEMON_AUTORESTART=1 MESIMON_TMUX_BIN=" + shlex.quote(tmux) + " " + shlex.quote(str(binary)) + "\n")
            launcher.chmod(0o700)
            write(run / "lab.json", manifest)
        time.sleep(5)  # bounded settling and observe-tier polls
        for side, board in manifest["boards"].items():
            write(run / (side + "-results.json"), snapshot_result(Path(board["repo"]), board["entries"]))
        manifest["status"] = "ready"
        write(run / "lab.json", manifest)
        while not stopping and time.time() < manifest["deadline"] and all(c.poll() is None for c, *_ in children):
            time.sleep(0.5)
    except Exception as error:
        manifest["error"] = str(error)
        raise
    finally:
        errors = []
        for child, repo, log, tmux in children:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=5)
            runtime, _ = paths(repo)
            if (runtime / "tmux.sock").exists():
                result = subprocess.run([tmux, "-S", str(runtime / "tmux.sock"), "kill-server"], capture_output=True, text=True, timeout=5)
                if result.returncode and "no server" not in result.stderr:
                    errors.append(result.stderr)
            log.close()
        manifest["status"] = "cleanup_failed" if errors else "stopped"
        manifest["cleanup_errors"] = errors
        write(run / "lab.json", manifest)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["start", "check", "stop", "serve", "open"])
    parser.add_argument("--run", type=Path)
    parser.add_argument("--side", choices=["before", "after"], default="after")
    parser.add_argument("--lifetime", type=int, default=3600)
    args = parser.parse_args()
    os.umask(0o077)
    LAB.mkdir(parents=True, exist_ok=True)
    if args.command == "start":
        if not 30 <= args.lifetime <= 3600:
            parser.error("lifetime must be between 30 and 3600 seconds")
        latest = LAB / "latest.json"
        if latest.exists():
            previous = Path(json.loads(latest.read_text())["run"]) / "lab.json"
            if previous.exists() and json.loads(previous.read_text()).get("status") in ("starting", "ready"):
                raise RuntimeError("A lab is already active. Stop it with ci/state-lab.py stop before starting another.")
        run = LAB / ("demo-" + time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:6])
        run.mkdir()
        with (run / "supervisor.log").open("w") as log:
            child = subprocess.Popen([sys.executable, "-B", str(Path(__file__).resolve()), "serve", "--run", str(run), "--lifetime", str(args.lifetime)], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        write(LAB / "latest.json", {"run": str(run)})
        def ready():
            if child.poll() is not None:
                raise RuntimeError(f"lab supervisor exited; inspect {run / 'supervisor.log'}")
            path = run / "lab.json"
            return path.exists() and json.loads(path.read_text()).get("status") == "ready"
        try:
            wait_for(ready, 90)
        except Exception:
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=25)
            raise
        print(f"Ready. Synthetic cases use no model tokens. Expires in {args.lifetime}s.\nBefore: {run / 'open-before.sh'}\nAfter:  {run / 'open-after.sh'}\nCheck:  python3 -B ci/state-lab.py check\nStop:   python3 -B ci/state-lab.py stop")
        return 0
    run = (args.run or Path(json.loads((LAB / "latest.json").read_text())["run"])).resolve()
    if args.command == "serve":
        serve(run, args.lifetime)
        return 0
    manifest = json.loads((run / "lab.json").read_text())
    if manifest.get("owner") != "mesimon-state-lab" or manifest.get("run") != str(run):
        raise RuntimeError("not a lab-owned manifest")
    if args.command == "open":
        if manifest["status"] != "ready":
            raise RuntimeError("The lab is not running. Start it with ci/state-lab.py start.")
        launcher = run / ("open-" + args.side + ".sh")
        os.execv("/bin/sh", ["/bin/sh", str(launcher)])
    if args.command == "stop":
        if manifest["status"] == "stopped":
            print("Already stopped")
            return 0
        pid = manifest["supervisor_pid"]
        command = subprocess.check_output(["ps", "-p", str(pid), "-o", "command="], text=True).strip()
        expected = f"{Path(__file__).resolve()} serve --run {run}"
        if expected not in command:
            raise RuntimeError("supervisor identity changed; refusing to signal the PID")
        os.kill(pid, signal.SIGTERM)
        wait_for(lambda: json.loads((run / "lab.json").read_text())["status"] in ("stopped", "cleanup_failed"), 25)
        final = json.loads((run / "lab.json").read_text())
        print(final["status"])
        return 0 if final["status"] == "stopped" else 1
    failed = False
    for side, board in manifest["boards"].items():
        result = snapshot_result(Path(board["repo"]), board["entries"])
        write(run / (side + "-results.json"), result)
        differences = [r for r in result if not r["passed"]]
        print(f"{side}: {len(result) - len(differences)}/{len(result)} expected checkpoints")
        for row in differences:
            print(f"  {row['case']}: expected {row['expected']}, got {row['actual']}")
        if side == "after" and differences:
            failed = True
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
