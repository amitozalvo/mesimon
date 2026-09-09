#!/usr/bin/env python3
"""Occasional, bounded real-Claude evidence. Never invoked by ordinary tests.

Only sessions created by this command are read. Raw captures are private local
artifacts, not automatically promoted into checked-in regression fixtures.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def collect_hook(args):
    payload = json.load(sys.stdin)
    entry = {"arrival_ms": time.time_ns() // 1_000_000, "event": args.event,
             "payload": payload}
    # A single append syscall; command hooks may run concurrently.
    fd = os.open(args.log, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        os.write(fd, (json.dumps(entry) + "\n").encode())
    finally:
        os.close(fd)
    if args.continue_once and args.event == "Stop" and not payload.get("stop_hook_active"):
        print(json.dumps({"decision": "block", "reason": "Reply LAB_SECOND once, then stop."}))


EVENTS = ["SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse",
          "PostToolUse", "PostToolUseFailure", "PermissionRequest", "PermissionDenied",
          "Stop", "StopFailure", "SubagentStart", "SubagentStop", "TeammateIdle",
          "PreCompact", "PostCompact", "Elicitation", "ElicitationResult", "Notification"]
PROMPTS = {
    "complete": "Reply with exactly LAB_DONE. Do not use tools.",
    "tool": "Run exactly this Bash command: sleep 2; printf LAB_TOOL_DONE. Then reply LAB_DONE. Do nothing else.",
    "stop-continuation": "Reply with exactly LAB_FIRST. Do not use tools.",
}


def capture(args):
    import shlex
    os.umask(0o077)
    binary = shutil.which(args.claude)
    if not binary:
        raise SystemExit("Claude executable unavailable; capture not run")
    version = subprocess.check_output([binary, "--version"], text=True, timeout=15).strip()
    root = Path(args.output).resolve()
    root.mkdir(parents=True, exist_ok=True)
    sid = str(uuid.uuid4())
    out = root / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
                  + "-" + args.case + "-" + sid[:8])
    out.mkdir()
    log = out / "hooks.jsonl"
    logger = [sys.executable, str(Path(__file__).resolve()), "hook", "--log", str(log)]
    hooks = {}
    for event in EVENTS:
        command = logger + ["--event", event]
        if args.case == "stop-continuation":
            command += ["--continue-once"]
        hooks[event] = [{"hooks": [{"type": "command", "command": shlex.join(command), "timeout": 5}]}]
    settings = out / "settings.json"
    write_json(settings, {"hooks": hooks})
    # A fresh /tmp repo avoids parent CLAUDE.md, plugins, or live board context.
    with tempfile.TemporaryDirectory(prefix="mesimon-claude-capture-") as scratch:
        subprocess.run(["git", "init", "-q", scratch], check=True, timeout=15)
        argv = [binary, "--model", args.model, "--setting-sources", "",
                "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
                "--settings", str(settings), "--session-id", sid,
                "--disable-slash-commands", "--tools", "Bash" if args.case == "tool" else "",
                "--allowedTools", "Bash(sleep 2*)", "--max-turns", "4",
                "--max-budget-usd", str(args.budget), "--output-format", "json",
                "-p", PROMPTS[args.case]]
        env = os.environ.copy()
        for key in list(env):
            if key.startswith("MESIMON_") or key in ("CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_ENTRYPOINT"):
                env.pop(key)
        manifest = {"schema": 1, "kind": "real_capture", "case": args.case,
                    "claude_version": version, "requested_model": args.model,
                    "mode": "print", "session_id": sid, "os": platform.platform(),
                    "argv": argv, "cwd": scratch, "budget_usd": args.budget,
                    "deadline_seconds": args.timeout, "started_ms": time.time_ns() // 1_000_000,
                    "result": "inconclusive", "limitations": ["Print mode does not verify interactive dialogs or Esc"]}
        write_json(out / "manifest.json", manifest)
        with (out / "stdout.json").open("w") as stdout, (out / "stderr.txt").open("w") as stderr:
            process = subprocess.Popen(argv, cwd=scratch, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                code = process.wait(timeout=args.timeout)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
                code = None
        manifest["exit_code"] = code
        manifest["ended_ms"] = time.time_ns() // 1_000_000
        records = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
        manifest["events"] = [record["event"] for record in records]
        # Session identity is checked before reading its reported transcript.
        for record in records:
            payload = record["payload"]
            if record["event"] == "SessionStart" and payload.get("session_id") == sid:
                path = Path(payload.get("transcript_path", ""))
                if path.is_file() and path.stem == sid:
                    shutil.copyfile(path, out / "transcript.jsonl")
                    manifest["transcript_captured"] = True
                break
        try:
            response = json.loads((out / "stdout.json").read_text())
        except (ValueError, OSError):
            response = {}
        manifest["model_usage"] = response.get("modelUsage", {})
        manifest["cost_usd"] = response.get("total_cost_usd")
        stops = [r for r in records if r["event"] == "Stop"]
        reached = "LAB_DONE" in response.get("result", "")
        if args.case == "stop-continuation":
            reached = "LAB_SECOND" in response.get("result", "") and any(
                r["payload"].get("stop_hook_active") for r in stops)
        if args.case == "tool":
            reached = reached and any(r["event"] == "PostToolUse" and
                                     r["payload"].get("tool_name") == "Bash" for r in records)
        manifest["result"] = "observed" if code == 0 and not response.get("is_error") and reached and stops else "inconclusive"
        manifest["stop_active_values"] = [r["payload"].get("stop_hook_active") for r in stops]
        write_json(out / "manifest.json", manifest)
    print(json.dumps({"capture": str(out), "result": manifest["result"], "version": version,
                      "events": manifest["events"], "cost_usd": manifest["cost_usd"]}, indent=2))
    return 0 if manifest["result"] == "observed" else 2


def positive(value):
    number = float(value)
    if not 0 < number <= 300:
        raise argparse.ArgumentTypeError("must be positive and at most 300")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    live = sub.add_parser("capture", help="explicit paid run; default Haiku, bounded")
    live.add_argument("--case", choices=PROMPTS, default="complete")
    live.add_argument("--model", choices=["haiku", "sonnet"], default="haiku")
    live.add_argument("--claude", default="claude")
    live.add_argument("--budget", type=positive, default=0.25)
    live.add_argument("--timeout", type=positive, default=60)
    live.add_argument("--output", default="target/state-lab/captures")
    hook = sub.add_parser("hook", help=argparse.SUPPRESS)
    hook.add_argument("--log", required=True)
    hook.add_argument("--event", required=True)
    hook.add_argument("--continue-once", action="store_true")
    args = parser.parse_args()
    if args.command == "hook":
        collect_hook(args)
        return 0
    return capture(args)


if __name__ == "__main__":
    sys.exit(main())
