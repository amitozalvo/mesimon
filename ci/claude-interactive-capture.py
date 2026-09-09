#!/usr/bin/env python3
"""Explicit Haiku/Sonnet probes in a private tmux server, with a hard deadline.

Interactive Claude does not support the print-mode dollar/turn caps. Each probe
uses one tiny prompt and a wall-clock deadline; no retry/model fallback loop.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import shlex
import signal
import test_guard
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

EVENTS = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse",
          "PostToolUseFailure", "PermissionRequest", "PermissionDenied", "Stop",
          "StopFailure", "SessionEnd", "PreCompact", "PostCompact", "Notification"]
PROMPTS = {
    "complete": "Reply exactly LAB_DONE. Do not use tools.",
    "permission": "We are testing file-write approvals in this temporary repository. Please use Bash to run printf hello > greeting.txt, then reply LAB_DONE.",
    "interrupt": "We are testing cancellation of a long-running tool in this temporary repository. Run printf ready > tool-started.txt; sleep 60 in Bash, in the foreground. I will interrupt it. If it completes, say LAB_DONE.",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=PROMPTS, default="complete")
    parser.add_argument("--model", choices=["haiku", "sonnet"], default="haiku")
    parser.add_argument("--timeout", type=int, default=45)
    parser.add_argument("--output", type=Path, default=Path("target/state-lab/captures"))
    args = parser.parse_args()
    if not 10 <= args.timeout <= 120:
        parser.error("timeout must be between 10 and 120 seconds")
    os.umask(0o077)
    claude, tmux = shutil.which("claude"), shutil.which("tmux")
    if not claude or not tmux:
        parser.error("claude and tmux are required")
    sid = str(uuid.uuid4())
    version = subprocess.check_output([claude, "--version"], text=True, timeout=10).strip()
    out = args.output.resolve() / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-interactive-" + args.case + "-" + sid[:8])
    out.mkdir(parents=True)
    logger = Path(__file__).with_name("claude-capture.py").resolve()
    hooks = {event: [{"hooks": [{"type": "command", "command": shlex.join([
        sys.executable, str(logger), "hook", "--log", str(out / "hooks.jsonl"), "--event", event]), "timeout": 5}]}] for event in EVENTS}
    (out / "settings.json").write_text(json.dumps({"hooks": hooks}))
    manifest = {"schema": 1, "kind": "real_capture", "mode": "interactive_tmux", "case": args.case,
                "claude_version": version, "requested_model": args.model, "session_id": sid,
                "os": platform.platform(), "deadline_seconds": args.timeout, "result": "inconclusive",
                "checkpoints": [], "limitation": "Wall-clock bounded; print-mode budget flags do not apply"}
    with tempfile.TemporaryDirectory(prefix="msmn-live-") as scratch:
        repo = Path(scratch) / "repo"
        repo.mkdir()
        subprocess.run(["git", "init", "-q", str(repo)], check=True, timeout=10)
        sock = str(Path(scratch) / "t.sock")
        def tm(*arguments):
            return subprocess.run([tmux, "-S", sock, *arguments], capture_output=True, text=True, timeout=5)
        argv = [claude, "--model", args.model, "--setting-sources", "", "--settings", str(out / "settings.json"),
                "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--disable-slash-commands",
                "--tools", "" if args.case == "complete" else "Bash", "--permission-mode", "default", "--session-id", sid,
                PROMPTS[args.case]]
        if args.case == "interrupt":
            argv[1:1] = ["--allowedTools", "Bash(printf ready*)"]
        manifest["argv"] = argv
        manifest["cwd"] = str(repo)
        manifest["tmux_version"] = subprocess.check_output([tmux, "-V"], text=True, timeout=5).strip()
        env = {k: v for k, v in os.environ.items() if not k.startswith("MESIMON_")
               and k not in ("CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_ENTRYPOINT", "TMUX", "TMUX_PANE")}
        start = subprocess.run([tmux, "-S", sock, "-f", "/dev/null", "new-session", "-d", "-s", "probe",
                                "-x", "140", "-y", "42", "-c", str(repo), "sleep 120"], env=env,
                               capture_output=True, text=True, timeout=10)
        if start.returncode:
            raise RuntimeError(start.stderr)
        tm("set-option", "-w", "-t", "probe", "remain-on-exit", "on")
        launched = tm("respawn-pane", "-k", "-t", "probe", shlex.join(argv))
        if launched.returncode: raise RuntimeError(launched.stderr)
        started = time.monotonic()
        previous = None
        acknowledged_trust = False
        trust_action_at = 0
        acted = False
        allowed_once = False
        acted_at = None
        seen_status = set()
        frames = (out / "terminal.jsonl").open("w")
        statuses = (out / "status-files.jsonl").open("w")
        try:
            pane_pid = int(tm("display-message", "-p", "-t", "probe", "#{pane_pid}").stdout.strip())
            while time.monotonic() - started < args.timeout:
                elapsed = int((time.monotonic() - started) * 1000)
                screen = tm("capture-pane", "-p", "-t", "probe").stdout
                if screen != previous:
                    frames.write(json.dumps({"at_ms": elapsed, "screen": screen}) + "\n")
                    frames.flush()
                    previous = screen
                if tm("display-message", "-p", "-t", "probe", "#{pane_dead}").stdout.strip() == "1":
                    (out / "early-exit.txt").write_text(screen)
                    manifest["error"] = "Claude exited before the scenario checkpoint"
                    break
                if not acknowledged_trust and "Yes, I trust this folder" in screen and elapsed > 1500 and elapsed - trust_action_at > 700:
                    if "❯ No, exit" in screen:
                        tm("send-keys", "-t", "probe", "Down")
                        trust_action_at = elapsed
                    elif "❯ Yes, I trust this folder" in screen:
                        tm("send-keys", "-t", "probe", "Enter")
                        manifest["checkpoints"].append({"at_ms": elapsed, "action": "trust_fresh_lab_repo"})
                        acknowledged_trust = True
                    else:
                        manifest["error"] = "Unrecognized trust selection; no key sent"
                        break
                # Read only the session files for this pane process or its
                # descendants. Do not crawl the user's transcript directory.
                table = subprocess.check_output(["ps", "-axo", "pid=,ppid="], text=True, timeout=5)
                pairs = [tuple(map(int, line.split())) for line in table.splitlines() if len(line.split()) == 2]
                pids = {pane_pid}
                for _ in range(5):
                    pids.update(pid for pid, parent in pairs if parent in pids)
                home = Path(os.environ.get("CLAUDE_CONFIG_DIR", str(Path.home() / ".claude")))
                for pid in pids:
                    file = home / "sessions" / f"{pid}.json"
                    try:
                        value = json.loads(file.read_text())
                    except (OSError, ValueError):
                        continue
                    if value.get("sessionId") != sid:
                        continue
                    raw = value
                    value = {k: value[k] for k in ("sessionId", "pid", "status", "statusUpdatedAt", "version", "startedAt") if k in value}
                    signature = json.dumps(value, sort_keys=True)
                    if signature not in seen_status:
                        seen_status.add(signature)
                        statuses.write(json.dumps({"at_ms": elapsed, "value": value, "raw": raw}) + "\n")
                        statuses.flush()
                log = out / "hooks.jsonl"
                events = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
                if args.case in ("permission", "interrupt") and not allowed_once and "Do you want to proceed?" in screen:
                    (out / "permission-visible.txt").write_text(screen)
                    manifest["checkpoints"].append({"at_ms": elapsed, "observation": "permission_dialog_visible", "action": "allow_once"})
                    tm("send-keys", "-t", "probe", "Enter")
                    allowed_once = True
                    if args.case == "permission":
                        acted, acted_at = True, time.monotonic()
                if args.case == "interrupt" and not acted and any(r["event"] == "PreToolUse" for r in events):
                    # Independent checkpoint: the tool has written its marker
                    # to the actual terminal before we send Esc.
                    if (repo / "tool-started.txt").is_file() and (repo / "tool-started.txt").read_text() == "ready":
                        (out / "tool-visible.txt").write_text(screen)
                        tm("send-keys", "-t", "probe", "Escape")
                        manifest["checkpoints"].append({"at_ms": elapsed, "observation": "tool_running_visible", "action": "Escape"})
                        acted, acted_at = True, time.monotonic()
                finished = any(r["event"] == "Stop" for r in events) and "LAB_DONE" in screen
                if args.case == "complete" and finished:
                    manifest["result"] = "observed"
                    break
                if args.case == "permission" and acted and finished and (repo / "greeting.txt").is_file() and (repo / "greeting.txt").read_text() == "hello":
                    manifest["result"] = "observed"
                    break
                if args.case == "interrupt" and acted and time.monotonic() - acted_at > 3:
                    (out / "after-interrupt.txt").write_text(screen)
                    if "Interrupted" in screen or "interrupted" in screen:
                        manifest["result"] = "observed"
                        break
                time.sleep(0.25)
        finally:
            frames.close()
            statuses.close()
            # Capture the transcript before ending the process; SessionEnd
            # caused by our cleanup must not be mistaken for the scenario.
            log = out / "hooks.jsonl"
            events = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            manifest["events_before_cleanup"] = [r["event"] for r in events]
            for record in events:
                p = record["payload"]
                if record["event"] == "SessionStart" and p.get("session_id") == sid:
                    path = Path(p.get("transcript_path", ""))
                    if path.is_file() and path.stem == sid:
                        shutil.copyfile(path, out / "transcript.jsonl")
                    break
            table = test_guard.process_table()
            owned = test_guard.descendants(table, {pane_pid})
            tm("kill-server")
            for sig in (signal.SIGTERM, signal.SIGKILL):
                table = test_guard.process_table()
                for pid, before in owned.items():
                    if test_guard.same_process(before, table.get(pid)):
                        try: os.kill(pid, sig)
                        except ProcessLookupError: pass
                time.sleep(0.2)
            table = test_guard.process_table()
            survivors = [pid for pid, before in owned.items() if test_guard.same_process(before, table.get(pid))]
            manifest["cleanup_survivors"] = survivors
            if survivors: manifest["result"] = "cleanup_failed"
            manifest["duration_seconds"] = round(time.monotonic() - started, 2)
            (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"capture": str(out), "result": manifest["result"], "events": manifest["events_before_cleanup"]}, indent=2))
    return 0 if manifest["result"] == "observed" else 2


if __name__ == "__main__":
    sys.exit(main())
