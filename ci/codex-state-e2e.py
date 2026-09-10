#!/usr/bin/env python3
"""Opt-in connected verification of production Codex sessions through Mesimon.

Run only under ci/test-run.py, for example:
  python3 -B ci/test-run.py --timeout 180 -- python3 -B ci/codex-state-e2e.py --live

Uses a supervised disposable repository and an isolated Codex home containing
only existing authentication and fixture configuration. No synthetic provider
events are sent. The native terminal remains the only prompt/approval interface.
"""

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import shlex
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


lab = module("provider_state_lab", ROOT / "ci/state-lab.py")
probe = module("provider_runtime_probe", ROOT / "ci/codex-runtime-probe.py")


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


class Inconclusive(Exception):
    pass


def patch_invocations(records):
    """Native code mode nests the actual tool inside the persisted exec call."""
    patches = []
    for record in records:
        payload = record.get("payload", {})
        if record.get("type") != "response_item" or payload.get("type") not in ("custom_tool_call", "function_call"):
            continue
        text = str(payload.get("input") or payload.get("arguments") or "")
        if payload.get("name") == "apply_patch" or (payload.get("name") == "exec" and "tools.apply_patch(" in text):
            patches.append(payload)
    return patches


def validate_guard_capture(directory):
    """Review a retained owned capture; preserve its original runner outcome."""
    directory = directory.resolve(strict=True)
    manifest_path = directory / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("case") != "guard" or not manifest.get("cleanup"):
        raise AssertionError("not a cleanly reaped guard capture")
    files = list(directory.glob("*.history.jsonl"))
    if len(files) != 1 or files[0].stat().st_size > 16 * 1024 * 1024:
        raise AssertionError("expected exactly one bounded owned native history")
    records = [json.loads(line) for line in files[0].read_text().splitlines()]
    checkpoints = manifest["checkpoints"]
    required = ["explicit column sandbox and approval policies reached native configuration",
                "effective native config points to isolated home before trust",
                "production guard permits ordinary multi-file patch",
                "structured native title reaches board session metadata"]
    if any(not any(c.get("assertion") == name and c.get("passed") for c in checkpoints) for name in required):
        raise AssertionError("capture lacks required live configuration/edit/title checks")
    config = next(c for c in checkpoints if c.get("assertion") == required[1])
    fixture = Path(config["config_path"]).parent.parent
    metadata = next(record["payload"] for record in records if record.get("type") == "session_meta")
    if Path(metadata["cwd"]) != fixture / "repo":
        raise AssertionError("retained history does not match the proved isolated fixture")
    patches = patch_invocations(records)
    outputs = {record["payload"].get("call_id"): record["payload"].get("output")
               for record in records if record.get("type") == "response_item"
               and record.get("payload", {}).get("type") in ("custom_tool_call_output", "function_call_output")}
    ordinary = [patch for patch in patches if "safe-a.txt" in str(patch) and "safe-b.txt" in str(patch)]
    protected = [patch for patch in patches if "*** Move to:" in str(patch) and ".mesimon/guard-denied.txt" in str(patch)]
    if not ordinary or not protected:
        raise AssertionError("capture lacks ordinary multi-file patch or protected rename")
    if not any("Script completed" in str(outputs.get(patch.get("call_id"))) for patch in ordinary):
        raise AssertionError("ordinary patch did not complete in retained native output")
    denial = "Command blocked by PreToolUse hook: mesimon owns .mesimon/"
    if not any(denial in str(outputs.get(patch.get("call_id"))) for patch in protected):
        raise AssertionError("protected rename has no actual production gate denial")
    replies = [part.get("text", "") for record in records if record.get("type") == "response_item"
               and record.get("payload", {}).get("role") == "assistant"
               for part in record["payload"].get("content", []) if part.get("type") == "output_text"]
    if any(not any(marker in reply for reply in replies) for marker in ("MESIMON_SAFE_PATCH_OK", "MESIMON_GUARD_DENIED")):
        raise AssertionError("capture lacks both actual completed assistant replies")
    result = dict(outcome="retrospectively_verified", original_runner_outcome=manifest["outcome"],
                  original_runner_error=manifest.get("error"), model_calls_added=0,
                  evidence_sha256={path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                   for path in [manifest_path, files[0], directory / "timeline.jsonl", directory / "terminal.jsonl"]},
                  checks=required + ["native code-mode invokes tools.apply_patch for both patches",
                                     "native tool output records actual protected-rename Mesimon gate denial",
                                     "both actual assistant replies retained"],
                  limitation="Original runner remains inconclusive. Live protected-file invariants and GateDenied feed checks preceded its history-parser assertion; the transient files/feed were not retained independently.")
    write(directory / "retrospective-verification.json", result)
    return result


def validate_plan_capture(directory):
    """Check retained plan evidence without converting a failed runner to pass."""
    directory = directory.resolve(strict=True)
    files = list(directory.glob("*.history.jsonl"))
    paths = [directory / name for name in ("manifest.json", "timeline.jsonl", "terminal.jsonl")]
    if len(files) != 1 or any(path.stat().st_size > 16 * 1024 * 1024 for path in paths + files):
        raise AssertionError("expected one bounded owned plan history and evidence set")
    manifest = json.loads(paths[0].read_text())
    if manifest.get("case") != "plan" or not manifest.get("cleanup"):
        raise AssertionError("not a cleanly reaped plan capture")
    required = ["effective native config points to isolated home before trust",
                "native plan mode enabled without a model turn",
                "completed plan requires native action and holds checkout",
                "plan preview and persisted ticket note are available before decision",
                "queued prompt remains withheld throughout native plan dialog"]
    checkpoints = manifest["checkpoints"]
    if any(not any(c.get("assertion") == name and c.get("passed") for c in checkpoints) for name in required):
        raise AssertionError("capture lacks required live plan/configuration/note/queue checks")
    config = next(c for c in checkpoints if c.get("assertion") == required[0])
    fixture = Path(config["config_path"]).parent.parent
    records = [json.loads(line) for line in files[0].read_text().splitlines()]
    metadata = next(record["payload"] for record in records if record.get("type") == "session_meta")
    if Path(metadata["cwd"]) != fixture / "repo":
        raise AssertionError("native history does not match the proved isolated fixture")
    timeline = [json.loads(line) for line in paths[1].read_text().splitlines()]
    plan = next(row for row in timeline if row["state"] == {"state":"requires_action", "reason":"plan"}
                and row.get("observation_hold") and row["column"] == "IN PROGRESS")
    declined_at = next(c["at_ms"] for c in checkpoints if c.get("action") == "native_decline_plan_stay_in_plan_mode")
    idle = next(row for row in timeline if row["at_ms"] > declined_at
                and row["state"] == {"state":"idle", "stop_reason":"unknown"}
                and row["column"] == "IN PROGRESS" and row["turn_id"] == plan["turn_id"]
                and (not row.get("observation_hold") or row.get("pending_submit")))
    done = next(row for row in timeline if row["at_ms"] > idle["at_ms"]
                and row["state"] == {"state":"idle", "stop_reason":"end_turn"}
                and row["column"] == "REVIEW" and row["turn_id"] != plan["turn_id"]
                and row["thread_id"] == plan["thread_id"] and not row.get("observation_hold")
                and not row.get("pending_submit") and not row.get("pending_prefill"))
    if any(row["column"] == "REVIEW" for row in timeline if row["at_ms"] <= idle["at_ms"]):
        raise AssertionError("plan or dismissal prematurely promoted the ticket")
    replies = [part.get("text", "") for record in records if record.get("type") == "response_item"
               and record.get("payload", {}).get("role") == "assistant"
               for part in record["payload"].get("content", []) if part.get("type") == "output_text"]
    if not any("<proposed_plan>" in text and "PLAN_ACCEPTED" in text for text in replies):
        raise AssertionError("actual native proposed plan is missing")
    if "MESIMON_PLAN_DECLINED" not in replies:
        raise AssertionError("actual queued-prompt assistant reply is missing")
    result = dict(outcome="retrospectively_verified", original_runner_outcome=manifest["outcome"],
                  original_runner_error=manifest.get("error"), model_calls_added=0,
                  evidence_sha256={path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in paths + files},
                  checks=required + ["decline yields IdleUnknown without automove",
                                     "pending queued delivery legitimately retains checkout hold",
                                     "same native thread completes queued prompt before REVIEW"],
                  timestamps_ms=dict(plan=plan["at_ms"], declined_idle=idle["at_ms"], queued_done=done["at_ms"]),
                  limitation="Original runner remains inconclusive. Its note-body marker assertion passed, but the full read_note response was not retained. The optional final README unchanged assertion was not reached and is not claimed.")
    write(directory / "retrospective-verification.json", result)
    return result


def isolated_claude_auth(env, directory):
    """Existing authentication only; no personal trust or conversation files."""
    directory.mkdir(mode=0o700)
    credentials = Path(os.environ.get("CLAUDE_CONFIG_DIR", str(Path.home() / ".claude"))) / ".credentials.json"
    if not env.get("CLAUDE_CODE_OAUTH_TOKEN") and not env.get("ANTHROPIC_API_KEY"):
        value = None
        if credentials.is_file():
            value = json.loads(credentials.read_text())
        elif sys.platform == "darwin":
            # Exactly the CLI's named service. Captured output stays in memory,
            # is never printed, and no credential/trust setting is changed.
            result = subprocess.run(["security", "find-generic-password", "-s", "Claude Code-credentials", "-w"],
                                    capture_output=True, text=True, timeout=10)
            if result.returncode == 0:
                value = json.loads(result.stdout)
        token = (value or {}).get("claudeAiOauth", {}).get("accessToken")
        if not token:
            raise Inconclusive("existing isolated Claude authentication unavailable; no login or personal configuration change attempted")
        env["CLAUDE_CODE_OAUTH_TOKEN"] = token
    env["CLAUDE_CONFIG_DIR"] = str(directory)
    # This is fixture onboarding, not repository/hook permission trust.
    write(directory / ".claude.json", {"hasCompletedOnboarding": True, "theme": "dark"})


class Verification:
    def __init__(self, args, out, manifest, guard):
        self.args, self.out, self.manifest, self.guard = args, out, manifest, guard
        self.started = time.monotonic()
        self.tmux = args.tmux
        self.repo = guard.root / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True, timeout=5)
        (self.repo / "README.md").write_text("Disposable production-provider acceptance fixture.\n")
        self.home = guard.root / "codex-home"
        self.home.mkdir(mode=0o700)
        if not args.auth_file.is_file():
            raise Inconclusive("existing Codex auth.json unavailable; no login attempted")
        shutil.copyfile(args.auth_file, self.home / "auth.json")
        (self.home / "auth.json").chmod(0o600)
        (self.home / "config.toml").write_text(
            'cli_auth_credentials_store = "file"\ncheck_for_update_on_startup = false\nmodel_reasoning_effort = "low"\n'
            + "model = " + json.dumps(args.model) + "\n")
        if args.case == "brief":
            self.developer = "MESIMON_DEVELOPER_" + uuid.uuid4().hex[:10]
            with (self.home / "config.toml").open("a") as config:
                config.write("developer_instructions = " + json.dumps(
                    "The fixture developer sentinel is " + self.developer + ".") + "\n")
        if args.case == "child":
            with (self.home / "config.toml").open("a") as config:
                config.write("[agents]\nmax_threads = 1\n")
        if args.case == "guard":
            for name, value in {"safe-a.txt": "alpha\n", "safe-b.txt": "beta\n",
                                "rename-source.txt": "source\n"}.items():
                (self.repo / name).write_text(value)
        # tmux and the production login-shell capture do not guarantee daemon
        # environment propagation. Pin fixture isolation at the actual CLI exec.
        codex_wrapper = guard.root / "codex-isolated"
        codex_wrapper.write_text("#!/bin/sh\nexport CODEX_HOME=" + shlex.quote(str(self.home))
                                 + "\nexec " + shlex.quote(args.codex) + ' "$@"\n')
        codex_wrapper.chmod(0o700)
        self.runtime, self.state = lab.paths(self.repo)
        guard.request(op="register", repo=str(self.repo), state=str(self.state),
                      runtime=str(self.runtime), sock=str(self.runtime / "tmux.sock"))
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("MESIMON_", "CODEX_", "OPENAI_"))
                    and key not in ("TMUX", "TMUX_PANE")}
        self.env.update(CODEX_HOME=str(self.home), TERM="xterm-256color",
                        MESIMON_TMUX_BIN=self.tmux, MESIMON_CODEX_BIN=str(codex_wrapper),
                        MESIMON_HOOK_BIN=str(args.binary), MESIMON_NO_DAEMON_AUTORESTART="1")
        if args.case == "switch":
            if not args.claude:
                raise Inconclusive("installed Claude Code required for the switching case")
            isolated_claude_auth(self.env, guard.root / "claude-home")
            auth_path = guard.root / "claude-home" / "fixture-auth.json"
            write(auth_path, {key: self.env[key] for key in ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY") if self.env.get(key)})
            auth_path.chmod(0o600)
            wrapper = guard.root / "claude-economical"
            wrapper.write_text("#!" + sys.executable + "\nimport json,os,sys\n"
                + "os.environ.update(json.load(open(" + repr(str(auth_path)) + ")))\n"
                + "os.environ['CLAUDE_CONFIG_DIR']=" + repr(str(guard.root / "claude-home")) + "\n"
                + "os.execv(" + repr(args.claude) + ", [" + repr(args.claude) + "] + sys.argv[1:] + ['--model', " + repr(args.claude_model) + "])\n")
            wrapper.chmod(0o700)
            self.env["MESIMON_CLAUDE_BIN"] = str(wrapper)
            self.manifest["claude_version"] = subprocess.check_output([args.claude, "--version"], text=True, timeout=10).strip()
        guard.request(op="spawn", argv=[str(args.binary), "daemon", "--repo", str(self.repo)], env=self.env)
        lab.wait_for(lambda: (self.runtime / "orch.sock").exists(), timeout=15)
        self.client = lab.Client(self.repo)
        self.client.request("set_agent_provider", provider="codex")
        self.client.request("set_mcp_tools", on=args.case in ("mcp", "brief"))
        if args.case == "brief":
            self.client.request("set_system_prompt", on=True)
        self.sessions = []
        self.child_histories = {}
        self.previous = {}
        self.calls = 0
        self.checked_config = set()
        self.hook_review = set()
        self.hook_trusted = set()

    def request(self, command, **fields):
        response = self.client.request(command, **fields)
        if response.get("resp") in ("err", "error", "denied"):
            raise AssertionError(f"production {command} refused: {response.get('message', response.get('resp'))}")
        return response

    def tm(self, *args):
        result = subprocess.run([self.tmux, "-S", str(self.runtime / "tmux.sock"), *args],
                                capture_output=True, text=True, timeout=5)
        if result.returncode:
            raise Inconclusive("owned tmux command failed: " + result.stderr.strip())
        return result.stdout

    def mark(self, **fields):
        self.manifest["checkpoints"].append(dict(at_ms=int((time.monotonic() - self.started) * 1000), **fields))

    def observe(self, sid):
        if time.monotonic() - self.started > self.args.timeout:
            raise Inconclusive("whole verification deadline exceeded")
        board = self.client.board()
        session = next(s for s in board["sessions"] if s["id"] == sid)
        ticket = next(t for t in board["tickets"] if t["id"] == session["ticket"])
        target = sid.replace("-", "")[:16]
        screen = ""
        if session["state"]["state"] not in ("sleeping", "exited"):
            screen = self.tm("capture-pane", "-p", "-t", target)
        row = dict(session=sid, provider=session["kind"], state=session["state"],
                   column=ticket["column"], confidence=session["confidence"],
                   observation_hold=session.get("observation_hold"),
                   thread_id=session.get("codex_thread_id"), turn_id=session.get("codex_turn_id"),
                   pending_prefill=session.get("pending_prefill"), pending_submit=session.get("pending_submit"))
        if row != self.previous.get((sid, "row")):
            with (self.out / "timeline.jsonl").open("a") as log:
                log.write(json.dumps(dict(at_ms=int((time.monotonic() - self.started) * 1000), **row)) + "\n")
            self.previous[(sid, "row")] = row
        if screen != self.previous.get((sid, "screen")):
            with (self.out / "terminal.jsonl").open("a") as log:
                log.write(json.dumps(dict(session=sid, screen=screen)) + "\n")
            self.previous[(sid, "screen")] = screen
        return session, ticket, screen

    def native_read(self, sid, method, params):
        config = json.loads((self.state / "hooks" / (sid + ".codex.json")).read_text())
        if config["executable"] != str(self.guard.root / "codex-isolated"):
            raise AssertionError("fixture executable does not enforce isolated Codex home")
        ws = probe.WebSocket(config["upstream_socket"])
        try:
            for ident, name, fields in [
                (1, "initialize", {"clientInfo": {"name": "mesimon_fixture_audit", "version": "1"}, "capabilities": {"experimentalApi": True}}),
                (2, method, params),
            ]:
                ws.send(json.dumps(dict(id=ident, method=name, params=fields)))
                end = time.monotonic() + 8
                result = None
                while time.monotonic() < end:
                    frame = ws.receive(0.1)
                    if frame and frame.get("id") == ident and "method" not in frame:
                        result = frame
                        break
                if not result or "error" in result:
                    with (self.out / "native-read-errors.jsonl").open("a") as errors:
                        errors.write(json.dumps(dict(method=name, params=fields, response=result)) + "\n")
                    raise Inconclusive("read-only fixture " + name + " audit was refused")
                if ident == 1:
                    ws.send(json.dumps(dict(method="initialized", params={})))
            return result["result"]
        finally:
            ws.close()

    def verify_native_home(self, sid):
        if sid in self.checked_config:
            return
        value = self.native_read(sid, "config/read", {"includeLayers": True, "cwd": str(self.repo)})
        origin = value.get("origins", {}).get("model", {}).get("name", {})
        if value.get("config", {}).get("model") != self.args.model or origin.get("file") != str(self.home / "config.toml"):
            raise AssertionError("effective native model/config origin is outside the isolated fixture")
        if value.get("config", {}).get("model_reasoning_effort") != "low":
            raise AssertionError("economical fixture reasoning effort is not low")
        if self.args.case == "brief" and self.developer not in (value.get("config", {}).get("developer_instructions") or ""):
            raise AssertionError("fixture developer sentinel is absent before trust or submission")
        if self.args.case == "guard":
            policy = value.get("config", {})
            if policy.get("sandbox_mode") != "workspace-write" or policy.get("approval_policy") != "on-request":
                raise AssertionError("production column sandbox/approval settings did not reach native configuration")
            self.mark(assertion="explicit column sandbox and approval policies reached native configuration",
                      sandbox=policy["sandbox_mode"], approvals=policy["approval_policy"], passed=True)
        if self.args.case == "child":
            agents = value.get("config", {}).get("agents", {})
            if agents.get("max_concurrent_threads_per_session", agents.get("max_threads")) != 1:
                raise AssertionError("native fixture child concurrency bound is absent before submission")
        self.checked_config.add(sid)
        self.mark(assertion="effective native config points to isolated home before trust", session=sid,
                  config_path=origin["file"], model=self.args.model, passed=True)

    def startup_consent(self, sid, screen):
        target = sid.replace("-", "")[:16]
        session = next(s for s in self.client.board()["sessions"] if s["id"] == sid)
        if session["kind"] == "codex" and any(marker in screen for marker in ("trust this folder", "Do you trust", "Hooks need review", "Press t to trust")):
            self.verify_native_home(sid)
        if "❯ No, exit" in screen and "Yes, I trust this folder" in screen:
            self.tm("send-keys", "-t", target, "Down")
            self.mark(action="native_select_fixture_repository_trust", session=sid)
            time.sleep(0.3)
            return
        if "Yes, I trust this folder" in screen or "Do you trust" in screen:
            self.tm("send-keys", "-t", target, "Enter")
            self.mark(action="native_fixture_repository_trust", session=sid)
            time.sleep(0.3)
            return
        if self.args.case == "brief" and "Hooks need review" in screen:
            hooks = self.native_read(sid, "hooks/list", {"cwds": [str(self.repo)]})
            enabled = [hook for group in hooks.get("data", []) for hook in group.get("hooks", []) if hook.get("enabled")]
            allowed = {"'" + str(self.args.binary) + "' gate --provider codex --from-env", "'" + str(self.args.binary) + "' agent-brief"}
            if not enabled or any(hook.get("source") != "sessionFlags" or hook.get("command") not in allowed for hook in enabled):
                raise AssertionError("brief fixture hook review contains an unknown handler")
            if "Trust all and continue" not in screen:
                raise Inconclusive("native startup hook trust choices changed")
            self.tm("send-keys", "-t", target, "2")
            self.tm("send-keys", "-t", target, "Enter")
            self.hook_trusted.add(sid)
            self.mark(action="native_trust_actual_fixture_gate_and_brief_before_initial_thread", session=sid,
                      hooks=[dict(command=h["command"], event=h["eventName"], source=h["source"]) for h in enabled])
            time.sleep(0.3)
            return
        if "Hooks need review" in screen and sid not in self.hook_review:
            self.tm("send-keys", "-t", target, "Enter")
            self.hook_review.add(sid)
            self.mark(action="native_review_fixture_hooks_before_trust", session=sid)
            return
        if "Press t to trust all;" in screen and sid in self.hook_review and sid not in self.hook_trusted:
            history = session.get("transcript_path")
            if not history:
                return
            try:
                Path(history).resolve().relative_to(self.home.resolve())
            except ValueError:
                raise AssertionError("native thread path escaped isolated fixture before hook trust")
            result = self.native_read(sid, "hooks/list", {"cwds": [str(self.repo)]})
            hooks = [hook for group in result.get("data", []) for hook in group.get("hooks", []) if hook.get("enabled")]
            allowed = {"'" + str(self.args.binary) + "' gate --provider codex --from-env", "'" + str(self.args.binary) + "' agent-brief"}
            if not hooks or any(hook.get("source") != "sessionFlags" or hook.get("command") not in allowed for hook in hooks):
                raise Inconclusive("native hook review found a definition outside this fixture's known Mesimon gate/brief")
            self.mark(action="reviewed_fixture_hook_definitions", session=sid, history_path=history,
                      hooks=[dict(command=h["command"], event=h["eventName"], source=h["source"]) for h in hooks])
            self.tm("send-keys", "-t", target, "t")
            self.hook_trusted.add(sid)
            self.mark(action="native_trust_reviewed_fixture_hooks", session=sid)
            return
        if "Press enter to view hooks; esc to close" in screen and sid in self.hook_trusted:
            self.tm("send-keys", "-t", target, "Escape")
            return
        # Only the explicitly requested, read-only scoped tool can be approved.
        # Unknown permissions or hook-trust screens remain visible/inconclusive.
        if self.args.case in ("mcp", "brief") and "get_ticket" in screen and "mesimon" in screen.lower() and "Allow" in screen:
            self.tm("send-keys", "-t", target, "Enter")
            self.mark(action="native_allow_scoped_get_ticket_once", session=sid)
            time.sleep(0.3)

    def wait(self, sid, description, predicate, timeout=55, consent=True):
        end = time.monotonic() + min(timeout, self.args.timeout)
        last = None
        while time.monotonic() < end:
            row = self.observe(sid)
            last = row[0]["state"]
            if last["state"] in ("exited", "failed"):
                raise AssertionError(f"native provider stopped before {description}: {last}")
            if predicate(*row):
                self.mark(assertion=description, session=sid, passed=True)
                return row
            if consent:
                self.startup_consent(sid, row[2])
            time.sleep(0.1)
        raise Inconclusive(f"checkpoint not reached: {description}; last state {last}")

    def start(self, prompt):
        self.calls += 1
        if self.calls > 6:
            raise Inconclusive("six-turn economical verification budget reached")
        ticket = self.request("create_ticket", column="TODO", title=prompt, workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=True)["id"]
        self.sessions.append(sid)
        self.mark(action="production_spawn_and_submit", session=sid, ticket=ticket)
        return sid

    def completed(self, sid, sentinel):
        def final(session, ticket, screen):
            if session["state"] != {"state": "idle", "stop_reason": "end_turn"} or ticket["column"] != "REVIEW":
                return False
            if session.get("pending_submit") or session.get("pending_prefill"):
                return False
            if session["kind"] == "codex" and session.get("observation_hold", True):
                return False
            preview = Path(session.get("agent_preview_path") or "/does-not-exist")
            preview_text = preview.read_text() if preview.is_file() else ""
            if session["kind"] == "codex":
                # A native echoed prompt may contain the same marker. The
                # provider artifact contains the completed assistant item.
                return bool(preview_text and sentinel in (json.loads(preview_text).get("text") or ""))
            return sentinel in screen or sentinel in preview_text
        return self.wait(sid, "native reply and high-confidence completion in REVIEW", final)

    def ask(self, sid, prompt):
        self.calls += 1
        if self.calls > 6:
            raise Inconclusive("six-turn economical verification budget reached")
        session, _, _ = self.observe(sid)
        self.request("prompt_session", ticket=session["ticket"], text=prompt, queued=False)
        self.mark(action="production_prompt_existing_session", session=sid)

    def run(self):
        if self.args.case == "faults":
            self.faults()
            return
        if self.args.case == "child":
            self.child()
            return
        if self.args.case in ("queue-external", "external"):
            self.queue_external()
            return
        if self.args.case == "brief":
            self.brief()
            return
        if self.args.case == "plan":
            self.plan()
            return
        if self.args.case == "guard":
            self.guard_edits()
            return
        if self.args.case == "switch":
            self.switch()
            return
        sentinel = "MESIMON_CODEX_ACCEPTED"
        if self.args.case == "mcp":
            prompt = "Use mesimon get_ticket to read this ticket, then reply exactly " + sentinel + ". Do not modify files."
        elif self.args.case == "interrupt":
            prompt = "Run sleep 25 in the terminal, wait until it finishes, then reply exactly " + sentinel + "."
        else:
            prompt = "Reply exactly " + sentinel + ". Do not use tools."
        sid = self.start(prompt)
        if self.args.case == "interrupt":
            self.wait(sid, "real turn working", lambda session, ticket, screen: session["state"]["state"] == "running")
            self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Escape")
            self.mark(action="native_escape_interrupt", session=sid)
            self.wait(sid, "interruption never promotes to REVIEW", lambda session, ticket, screen:
                      session["state"] == {"state":"idle", "stop_reason":"interrupted"} and ticket["column"] == "IN PROGRESS")
            return
        session, _, _ = self.completed(sid, sentinel)
        if self.args.case == "mcp":
            if not session.get("ticket_read"):
                raise AssertionError("native reply completed but scoped get_ticket did not mark the ticket read")
            self.mark(assertion="scoped MCP reached the actual Codex session principal", passed=True)
        if self.args.case == "handover":
            thread = session.get("codex_thread_id")
            target = sid.replace("-", "")[:16]
            pane_pid = self.tm("display-message", "-p", "-t", target, "#{pane_pid}").strip()
            self.client.close()
            time.sleep(0.5)
            self.client = lab.Client(self.repo)
            self.wait(sid, "board closure preserves live native conversation", lambda s,t,p:
                      s.get("codex_thread_id") == thread and s["state"]["state"] == "idle"
                      and not s.get("observation_hold", True))
            self.request("shutdown")
            self.client.close()
            lab.wait_for(lambda: not (self.runtime / "orch.sock").exists(), timeout=10)
            self.guard.request(op="spawn", argv=[str(self.args.binary), "daemon", "--repo", str(self.repo)], env=self.env)
            lab.wait_for(lambda: (self.runtime / "orch.sock").exists(), timeout=15)
            self.client = lab.Client(self.repo)
            self.wait(sid, "replacement daemon adopts observed exact session", lambda s,t,p:
                      s.get("codex_thread_id") == thread and s["state"]["state"] == "idle"
                      and not s.get("observation_hold", True))
            if self.tm("display-message", "-p", "-t", target, "#{pane_pid}").strip() != pane_pid:
                raise AssertionError("daemon handover replaced the native terminal process")
            self.ask(sid, "Reply exactly MESIMON_HANDOVER_ALIVE. Do not use tools.")
            self.completed(sid, "MESIMON_HANDOVER_ALIVE")
            self.mark(assertion="same process and thread survive board closure and daemon handover", passed=True)
        if self.args.case == "sleep-resume":
            thread = session.get("codex_thread_id")
            if not thread:
                raise AssertionError("completed Codex session lacks exact native thread ID")
            self.request("sleep_session", id=sid)
            self.wait(sid, "Codex sleeping", lambda session, ticket, screen: session["state"]["state"] == "sleeping" and not session.get("codex_stopping", False), consent=False)
            self.request("set_agent_provider", provider="claude_code")
            self.request("wake_session", id=sid)
            self.wait(sid, "old Codex resumes after project provider switch", lambda session, ticket, screen:
                      session["kind"] == "codex" and session.get("codex_thread_id") == thread
                      and session["state"]["state"] == "idle" and not session.get("observation_hold", True))
            self.ask(sid, "Reply exactly MESIMON_CODEX_RESUMED. Do not use tools.")
            self.completed(sid, "MESIMON_CODEX_RESUMED")
            self.mark(assertion="same native thread after resumed turn", passed=self.observe(sid)[0].get("codex_thread_id") == thread)

    def faults(self):
        """One paid turn followed by exact, owned native failure injection."""
        if self.args.model != "gpt-5.6-luna":
            raise AssertionError("fault acceptance requires the economical Luna fixture")
        ownership = module("fault_fixture_ownership", ROOT / "ci/test_guard.py")
        sid = self.start("Reply exactly MESIMON_FAULT_FIXTURE. Do not use tools.")
        original, ticket, _ = self.completed(sid, "MESIMON_FAULT_FIXTURE")
        thread = original["codex_thread_id"]
        target = sid.replace("-", "")[:16]
        self.request("move_ticket", id=ticket["id"], column="IN PROGRESS", before=None)
        config_path = self.state / "hooks" / (sid + ".codex.json")

        def owned_processes():
            config = json.loads(config_path.read_text())
            if config["session"] != sid or config["executable"] != str(self.guard.root / "codex-isolated"):
                raise AssertionError("fault target runtime does not belong to the isolated fixture")
            pane = int(self.tm("display-message", "-p", "-t", target, "#{pane_pid}").strip())
            table = ownership.process_table()
            descendants = ownership.descendants(table, {pane})
            supervisors = {pid: row for pid, row in descendants.items()
                           if "agent-runtime" in shlex.split(row[3]) and str(config_path) in shlex.split(row[3])}
            if len(supervisors) != 1:
                raise Inconclusive("cannot uniquely identify the owned native supervisor")
            supervisor = next(iter(supervisors))
            descendants = ownership.descendants(table, {supervisor})
            endpoint = "unix://" + config["upstream_socket"]
            servers = {pid: row for pid, row in descendants.items()
                       if "app-server" in shlex.split(row[3]) and endpoint in shlex.split(row[3])}
            # npm's launcher can remain between the supervisor and actual CLI.
            # Select the unique innermost app-server, never its process group.
            leaves = {pid: row for pid, row in servers.items()
                      if not any(other != pid and other in ownership.descendants(table, {pid}) for other in servers)}
            if len(leaves) != 1:
                raise Inconclusive("cannot uniquely identify the owned app-server process")
            return pane, supervisor, next(iter(leaves)), table, config

        def fault_state(phase):
            board = self.client.board()
            session = next(row for row in board["sessions"] if row["id"] == sid)
            ticket = next(row for row in board["tickets"] if row["id"] == session["ticket"])
            with (self.out / "fault-observations.jsonl").open("a") as log:
                log.write(json.dumps(dict(at_ms=int((time.monotonic()-self.started)*1000), phase=phase,
                    board=dict(state=session["state"], observation_hold=session.get("observation_hold"),
                    stopping=session.get("codex_stopping"), column=ticket["column"]))) + "\n")
            return session, ticket

        pane, supervisor, server, table, config = owned_processes()
        pane_identity = table[pane]
        daemon_pid = self.request("hello", version=2, client="fault-fixture")["daemon_pid"]
        registry = json.loads((self.guard.root / "owner.json").read_text())
        daemon = table.get(daemon_pid)
        if (daemon_pid not in registry["children"] or daemon is None
                or daemon[0] != self.guard.process.pid or str(self.repo) not in shlex.split(daemon[3])
                or "daemon" not in shlex.split(daemon[3])):
            raise Inconclusive("daemon crash target is not a directly registered fixture child")
        if not ownership.same_process(daemon, ownership.process_table().get(daemon_pid)):
            raise Inconclusive("owned daemon identity changed before fault injection")
        self.client.close()
        os.kill(daemon_pid, signal.SIGKILL)
        self.mark(action="kill_exact_owned_daemon", pid=daemon_pid, started=daemon[2])
        lab.wait_for(lambda: not ownership.same_process(daemon, ownership.process_table().get(daemon_pid)), timeout=10)
        replacement = self.guard.request(op="spawn", argv=[str(self.args.binary), "daemon", "--repo", str(self.repo)], env=self.env)
        def reconnect():
            try:
                candidate = lab.Client(self.repo)
                hello = candidate.request("hello", version=2, client="fault-fixture")
                if hello["daemon_pid"] != replacement:
                    candidate.close()
                    return False
                self.client = candidate
                return True
            except (OSError, ValueError):
                return False
        lab.wait_for(reconnect, timeout=15)
        resumed, resumed_ticket, _ = self.wait(sid, "hard-crashed daemon recovers exact running native seat", lambda session,ticket,screen:
            session.get("codex_thread_id") == thread and session.get("codex_generation") == original.get("codex_generation")
            and session["state"]["state"] == "idle" and not session.get("observation_hold", True)
            and "MESIMON_FAULT_FIXTURE" in screen)
        new_pane, _, _, current, _ = owned_processes()
        if new_pane != pane or not ownership.same_process(pane_identity, current.get(pane)):
            raise AssertionError("daemon crash/restart replaced the native terminal process")
        if resumed_ticket["column"] != "IN PROGRESS":
            raise AssertionError("daemon restart replayed an old completion as new automation")
        self.mark(assertion="SIGKILL daemon restart preserves native process, generation, history and provider", passed=True,
                  previous_daemon=daemon_pid, replacement_daemon=replacement, pane_pid=pane, thread_id=thread)

        # Native EOF is a UI action, never a model prompt or a server shutdown RPC.
        self.tm("send-keys", "-t", target, "C-d")
        self.mark(action="native_quit_ctrl_d", session=sid)
        def parked():
            session, _ = fault_state("native_quit")
            if session["state"]["state"] == "exited":
                raise AssertionError("clean native quit failed instead of parking exact history")
            return session["state"]["state"] == "sleeping" and not session.get("codex_stopping", False)
        lab.wait_for(parked, timeout=20)
        self.mark(assertion="native quit parks exact history and acknowledges cleanup", passed=True)
        if ownership.same_process(table[server], ownership.process_table().get(server)):
            raise AssertionError("native quit left its owned app-server alive")
        self.request("wake_session", id=sid)
        woke, woke_ticket, _ = self.wait(sid, "native quit wakes the exact original conversation without another turn", lambda session,ticket,screen:
            session.get("codex_thread_id") == thread and session["state"]["state"] == "idle"
            and not session.get("observation_hold", True) and "MESIMON_FAULT_FIXTURE" in screen)
        if woke_ticket["column"] != "IN PROGRESS":
            raise AssertionError("native wake manufactured completion automation")
        self.mark(assertion="native quit parks and exact wake restores prior history without a new turn", passed=True, thread_id=thread)

        _, _, server, current, config = owned_processes()
        server_identity = current[server]
        if not ownership.same_process(server_identity, ownership.process_table().get(server)):
            raise Inconclusive("owned app-server identity changed before fault injection")
        os.kill(server, signal.SIGKILL)
        self.mark(action="kill_exact_owned_app_server", pid=server, started=server_identity[2])
        snapshot_path = Path(config["snapshot_path"])
        deadline = min(self.started + self.args.timeout, time.monotonic() + 20)
        while time.monotonic() < deadline:
            session, current_ticket = fault_state("app_server_loss")
            if session["state"] == {"state":"idle", "stop_reason":"end_turn"} or current_ticket["column"] != "IN PROGRESS":
                raise AssertionError("app-server loss manufactured a successful completion")
            raw = json.loads(snapshot_path.read_text())
            with (self.out / "fault-observations.jsonl").open("a") as log:
                log.write(json.dumps(dict(snapshot=raw, board=dict(state=session["state"],
                    observation_hold=session.get("observation_hold"), stopping=session.get("codex_stopping")))) + "\n")
            if session["state"]["state"] == "exited" and not session.get("codex_stopping", False):
                if (not raw.get("stopped") or raw.get("state", {}).get("state") != "unknown"
                        or not raw.get("observation_hold")):
                    raise AssertionError("app-server death lacks conservative observation-loss cleanup evidence")
                break
            time.sleep(0.1)
        else:
            raise Inconclusive("owned app-server failure did not reach acknowledged cleanup within deadline")
        if ownership.same_process(server_identity, ownership.process_table().get(server)):
            raise AssertionError("killed owned app-server remains alive")
        if any(Path(config[name]).exists() for name in ("upstream_socket", "proxy_socket")):
            raise AssertionError("owned server sockets survived acknowledged cleanup")
        history = Path(session["transcript_path"])
        history.resolve().relative_to(self.home.resolve())
        turns = {record["payload"]["turn_id"] for record in (json.loads(line) for line in history.read_text().splitlines())
                 if record.get("payload", {}).get("type") == "task_started"}
        if self.calls != 1 or len(turns) != 1:
            raise AssertionError("fault verification exceeded its one-turn billing budget")
        self.mark(assertion="app-server death produces conservative failure and owned cleanup without completion", passed=True,
                  model_turns=1, thread_id=thread)

    def child(self):
        sid = self.start('I explicitly request one child. Execute this exact JavaScript in exec once: '
            'text(await tools.multi_agent_v1__spawn_agent({model:"gpt-5.6-luna",reasoning_effort:"low",'
            'message:"Run sleep 4 once, then reply MESIMON_CHILD_DONE. No edits, other tools, or agents."})); '
            'Use the message field exactly as written. Await only the spawn call, not child completion. '
            'After receiving the child ID, reply MESIMON_PARENT_DONE. No other calls, prompts, or responses.')
        self.manifest["child_turn_budget"] = 1
        thread = None
        child_id = None
        child_history = None
        saw_held_child = False
        terminal_child = False
        saw_late_child = False
        child_turns = None
        while time.monotonic() - self.started < min(self.args.timeout, 120):
            session, ticket, screen = self.observe(sid)
            self.startup_consent(sid, screen)
            thread = session.get("codex_thread_id")
            if (not thread or session.get("pending_prefill") or session.get("pending_submit")
                    or session["state"]["state"] in ("spawning", "requires_action")):
                time.sleep(0.15)
                continue
            parent = self.native_read(sid, "thread/turns/list", {"threadId": thread, "limit": 3, "itemsView": "full"})
            if parent.get("nextCursor") or len(parent.get("data", [])) > 1:
                raise AssertionError("child fixture exceeded its single foreground parent turn")
            items = [item for turn in parent.get("data", []) for item in turn.get("items", [])]
            spawns = [item for item in items if item.get("type") == "collabAgentToolCall" and item.get("tool") == "spawnAgent"]
            if len(spawns) > 1:
                raise AssertionError("native parent spawned more than one child")
            identities = {ident for item in spawns for ident in item.get("receiverThreadIds", [])}
            if len(identities) > 1:
                raise AssertionError("native spawn exceeded one-child budget")
            if identities:
                child_id = next(iter(identities))
                # Spawn announces an ID before child initialization/history is
                # necessarily readable. Poll read-only metadata within the
                # same task deadline; never submit another model request.
                for history in self.home.glob("sessions/**/*" + child_id + ".jsonl"):
                    history.resolve().relative_to(self.home.resolve())
                    self.child_histories[child_id] = history
                try:
                    child = self.native_read(sid, "thread/read", {"threadId": child_id, "includeTurns": False})["thread"]
                except Inconclusive:
                    if session["state"] == {"state":"idle", "stop_reason":"end_turn"} or ticket["column"] == "REVIEW":
                        raise AssertionError("parent completed before child initialization could be independently observed")
                    time.sleep(0.2)
                    continue
                if child.get("path"):
                    child_history = Path(child["path"])
                    child_history.resolve().relative_to(self.home.resolve())
                    self.child_histories[child_id] = child_history
                status = child.get("status", {})
                active = status.get("type") == "active"
                if status.get("type") in ("idle", "notLoaded"):
                    child_turns = self.native_read(sid, "thread/turns/list", {"threadId": child_id, "limit": 3, "itemsView": "full"})
                    terminal_child |= any(turn.get("status") == "completed" for turn in child_turns.get("data", []))
                saw_late_child |= active and any(turn.get("status") == "completed" for turn in parent.get("data", []))
                # A completed spawn RPC still leaves the child's work outstanding.
                # Observe this exact window; seeing only a busy parent is insufficient.
                if active and any(item.get("status") == "completed" for item in spawns):
                    session, ticket, _ = self.observe(sid)
                    if session["state"] == {"state":"idle", "stop_reason":"end_turn"} or ticket["column"] == "REVIEW":
                        raise AssertionError("active native child released the parent's checkout or completed it")
                    saw_held_child |= session.get("observation_hold", True)
                with (self.out / "child-observations.jsonl").open("a") as log:
                    log.write(json.dumps(dict(at_ms=int((time.monotonic()-self.started)*1000),
                        parent=parent, child=child, child_turns=child_turns, board=dict(state=session["state"],
                        observation_hold=session.get("observation_hold"), column=ticket["column"]))) + "\n")
            if session["state"] == {"state":"idle", "stop_reason":"end_turn"}:
                break
            if session["state"]["state"] in ("exited", "failed"):
                raise AssertionError("native child fixture parent exited or failed")
            time.sleep(0.15)
        else:
            raise Inconclusive("child acceptance exceeded its 120-second deadline")
        if not child_id or not saw_held_child:
            raise Inconclusive("no completed-spawn / live-child hold window was captured; no retry attempted")
        if not terminal_child:
            raise AssertionError("parent completed without native child terminal evidence")
        child_turns = self.native_read(sid, "thread/turns/list", {"threadId": child_id, "limit": 3, "itemsView": "full"})
        if child_turns.get("nextCursor") or len(child_turns.get("data", [])) != 1:
            raise AssertionError("child exceeded its one-turn task budget")
        replies = [item.get("text", "") for turn in child_turns["data"] for item in turn.get("items", []) if item.get("type") == "agentMessage"]
        if not any("MESIMON_CHILD_DONE" in reply for reply in replies):
            raise AssertionError("actual native child reply marker is missing")
        if child_history is None or not child_history.is_file():
            raise Inconclusive("native child history was not available for model and cost verification")
        records = [json.loads(line) for line in child_history.read_text().splitlines()]
        models = {record.get("payload", {}).get("model") for record in records if record.get("type") == "turn_context"}
        if models != {self.args.model}:
            raise AssertionError("native child did not use the economical fixture model")
        shutil.copyfile(child_history, self.out / (child_id + ".child-history.jsonl"))
        write(self.out / "child-turns.json", child_turns)
        self.completed(sid, "MESIMON_PARENT_DONE")
        self.mark(assertion="completed native spawn keeps checkout held through live child; only terminal child permits parent completion", passed=True,
                  parent_thread=thread, child_thread=child_id, parent_turns=1, child_turns=1, child_model=self.args.model,
                  parent_terminal_before_child_observed=saw_late_child)

    def brief(self):
        # Preflight fixture fields and actual runtime configuration before the
        # first submission. A startup-only session is intentionally unpaid.
        if (not self.developer.startswith("MESIMON_DEVELOPER_") or self.calls != 0
                or not isinstance(self.manifest.get("checkpoints"), list)):
            raise AssertionError("brief fixture preflight is incomplete")
        ticket = self.request("create_ticket", column="TODO", title="Production brief fixture", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        session, _, _ = self.wait(sid, "brief runtime preflight reaches an unsubmitted native composer", lambda s,t,p:
            s["state"]["state"] == "idle" and not s.get("observation_hold", True)
            and not s.get("pending_prefill", True) and not s.get("pending_submit", True)
            and "Production brief fixture" in p)
        initial_config = json.loads((self.state / "hooks" / (sid + ".codex.json")).read_text())
        if (dict(initial_config["env"]).get("MESIMON_AGENT_BRIEF") != "1"
                or not any("hooks.SessionStart=" in flag for flag in initial_config["config_flags"])
                or initial_config["resume"] is not None):
            raise AssertionError("actual brief runtime preflight configuration is incorrect")
        hooks = self.native_read(sid, "hooks/list", {"cwds": [str(self.repo)]})
        initial_hooks = [hook for group in hooks.get("data", []) for hook in group.get("hooks", []) if hook.get("enabled")]
        expected = "'" + str(self.args.binary) + "' agent-brief"
        if not any(hook.get("eventName") == "sessionStart" and hook.get("command") == expected
                   and hook.get("source") == "sessionFlags" for hook in initial_hooks):
            raise AssertionError("actual Mesimon brief handler is missing before submission")
        Path(session["transcript_path"]).resolve().relative_to(self.home.resolve())
        self.mark(assertion="brief fixture and actual generated runtime/hook fields verified before model submission", passed=True)
        self.tm("send-keys", "-t", sid.replace("-", "")[:16], "C-u")
        self.ask(sid, "Reply with the fixture developer sentinel followed by MESIMON_BRIEF_ON.")
        session, _, _ = self.completed(sid, "MESIMON_BRIEF_ON")
        thread = session["codex_thread_id"]
        preview = json.loads(Path(session["agent_preview_path"]).read_text())
        if self.developer not in (preview.get("text") or ""):
            raise AssertionError("production brief replaced the existing developer instructions")
        first_config = json.loads((self.state / "hooks" / (sid + ".codex.json")).read_text())
        if dict(first_config["env"]).get("MESIMON_AGENT_BRIEF") != "1" or not any("hooks.SessionStart=" in f for f in first_config["config_flags"]):
            raise AssertionError("production launch did not enable its actual brief hook")
        effective = self.native_read(sid, "config/read", {"includeLayers": True, "cwd": str(self.repo)})
        if self.developer not in (effective.get("config", {}).get("developer_instructions") or ""):
            raise AssertionError("effective developer configuration was replaced")
        history = Path(session["transcript_path"])
        history.resolve().relative_to(self.home.resolve())
        before = history.read_bytes()
        brief_marker = b"This session was started by mesimon on a ticket. FIRST,"
        context_records = [json.loads(line) for line in before.decode().splitlines()
                           if brief_marker.decode() in line]
        actual_context = [record for record in context_records if record.get("type") == "response_item"
                          and record.get("payload", {}).get("role") == "developer"
                          and "hooks.additional_context" in record.get("payload", {}).get("internal_chat_message_metadata_passthrough", {}).get("content_item_kinds", [])]
        if not actual_context:
            raise Inconclusive("actual production brief output is not retained in native history; on-injection needs additional observation")
        if not session.get("ticket_read"):
            raise Inconclusive("actual brief context was injected but the first turn did not read its scoped ticket")
        (self.out / "brief-enabled.history.jsonl").write_bytes(before)
        self.mark(assertion="actual Mesimon SessionStart brief is additive and reaches scoped ticket context",
                  passed=True, thread=thread, developer_sentinel=self.developer,
                  native_brief_occurrences=before.count(brief_marker))
        self.request("sleep_session", id=sid)
        self.wait(sid, "brief session sleeping with owned runtime stopped", lambda s,t,p:
                  s["state"]["state"] == "sleeping" and not s.get("codex_stopping", False), consent=False)
        self.request("set_system_prompt", on=False)
        self.request("wake_session", id=sid)
        session, _, _ = self.wait(sid, "brief-disabled wake resumes exact native conversation", lambda s,t,p:
                  s.get("codex_thread_id") == thread and s["state"]["state"] == "idle"
                  and not s.get("observation_hold", True))
        config = json.loads((self.state / "hooks" / (sid + ".codex.json")).read_text())
        hooks = self.native_read(sid, "hooks/list", {"cwds": [str(self.repo)]})
        enabled = [hook for group in hooks.get("data", []) for hook in group.get("hooks", []) if hook.get("enabled")]
        if (dict(config["env"]).get("MESIMON_AGENT_BRIEF") != "0"
                or any("hooks.SessionStart=" in flag for flag in config["config_flags"])
                or any(hook.get("eventName") == "sessionStart" for hook in enabled)):
            raise AssertionError("brief-disabled exact resume retained an enabled SessionStart hook")
        self.mark(assertion="resumed actual runtime and native hook registry omit fresh brief injection",
                  passed=True, thread=thread, agent_brief_env="0",
                  enabled_hooks=[dict(command=h["command"], event=h["eventName"], source=h["source"]) for h in enabled])
        # Capture the same append-only owned rollout immediately before the
        # resumed turn; prior brief context remains valid conversation history.
        after_wake = history.read_bytes()
        self.ask(sid, "Reply exactly MESIMON_BRIEF_OFF. Do not use tools.")
        final, _, _ = self.completed(sid, "MESIMON_BRIEF_OFF")
        after_turn = history.read_bytes()
        if not after_turn.startswith(after_wake) or brief_marker in after_turn[len(after_wake):]:
            raise AssertionError("brief-disabled turn added fresh brief context or rewrote its native history")
        if final.get("codex_thread_id") != thread:
            raise AssertionError("brief-disabled wake changed conversation identity")
        self.mark(assertion="disabled actual brief adds no fresh context on the next exact-resumed turn",
                  passed=True, same_thread=thread, model_turns=self.calls,
                  native_brief_occurrences_before=before.count(brief_marker),
                  native_brief_occurrences_after=after_turn.count(brief_marker))

    def plan(self):
        ticket = self.request("create_ticket", column="TODO", title="Native plan fixture", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        target = sid.replace("-", "")[:16]
        self.wait(sid, "native composer ready with unsubmitted fixture prefill", lambda session, ticket, screen:
                  session["kind"] == "codex" and session["state"]["state"] == "idle"
                  and not session.get("observation_hold", True) and not session.get("pending_prefill", True)
                  and "Native plan fixture" in screen)
        # Native slash command only: clear the intentionally unsubmitted title,
        # then ask the native client to switch collaboration mode.
        self.tm("send-keys", "-t", target, "C-u")
        self.tm("send-keys", "-t", target, "-l", "/plan")
        time.sleep(0.3)
        self.tm("send-keys", "-t", target, "Enter")
        self.wait(sid, "native plan mode enabled without a model turn", lambda s,t,p: "Plan mode" in p)
        self.ask(sid, "Propose a minimal one-step plan to append the line PLAN_ACCEPTED to README.md. "
                      "Do not implement it yet. Do not run tools; this disposable fixture is fully described here.")
        session, ticket_row, screen = self.wait(sid, "completed plan requires native action and holds checkout", lambda s,t,p:
            s["state"] == {"state":"requires_action", "reason":"plan"}
            and s.get("observation_hold") and t["column"] == "IN PROGRESS"
            and s.get("plan_note") and s.get("codex_plan_dialog_seen")
            and "Implement this plan?" in p)
        thread, turn = session["codex_thread_id"], session["codex_turn_id"]
        note = self.request("read_note", ticket=ticket, note=session["plan_note"])
        if "PLAN_ACCEPTED" not in note.get("text", ""):
            raise AssertionError("authoritative native plan did not reach the ticket note")
        self.mark(assertion="plan preview and persisted ticket note are available before decision", passed=True,
                  note=session["plan_note"], plan_text=note["text"], thread=thread, turn=turn)
        # Explicitly queue the person's words while the native local dialog is
        # open. Its lack of an approval RPC must never allow an automatic Enter.
        self.calls += 1
        self.request("prompt_session", ticket=ticket, queued=True,
                     text="Answer this informational request only: reply exactly MESIMON_PLAN_DECLINED. "
                          "Do not use tools, propose a plan, or implement the previous plan.")
        end = time.monotonic() + 2
        while time.monotonic() < end:
            current, current_ticket, pane = self.observe(sid)
            if (current["state"] != {"state":"requires_action", "reason":"plan"}
                    or not current.get("observation_hold") or current.get("codex_turn_id") != turn
                    or current_ticket["column"] != "IN PROGRESS" or "Implement this plan?" not in pane
                    or "MESIMON_PLAN_DECLINED" in pane):
                raise AssertionError("queued prompt reached or dismissed the native plan dialog")
            time.sleep(0.1)
        self.mark(assertion="queued prompt remains withheld throughout native plan dialog", passed=True)
        self.tm("send-keys", "-t", target, "3")
        self.tm("send-keys", "-t", target, "Enter")
        self.mark(action="native_decline_plan_stay_in_plan_mode", session=sid)
        self.wait(sid, "declining plan becomes observed idle without completion or automove", lambda s,t,p:
            s["state"] == {"state":"idle", "stop_reason":"unknown"}
            and (not s.get("observation_hold", True) or s.get("pending_submit", False))
            and t["column"] == "IN PROGRESS" and s.get("codex_plan_dismissed_turn") == turn, timeout=15, consent=False)
        completed, _, _ = self.completed(sid, "MESIMON_PLAN_DECLINED")
        if completed.get("codex_thread_id") != thread or completed.get("codex_turn_id") == turn:
            raise AssertionError("queued prompt did not continue the same native conversation")
        if (self.repo / "README.md").read_text() != "Disposable production-provider acceptance fixture.\n":
            raise AssertionError("declining a plan unexpectedly implemented it")
        self.mark(assertion="queued prompt resumes after explicit decline; proposed file change remains unapplied",
                  passed=True, model_turns=self.calls)

    def guard_edits(self):
        column = next(column for column in self.client.board()["columns"] if column["name"] == "TODO")
        # Column settings are flattened in the board wire snapshot.
        settings = {key: value for key, value in column.items() if key not in ("name", "order")}
        settings.update(codex_sandbox="workspace_write", codex_approval="on_request")
        self.request("set_column_settings", name="TODO", settings=settings)
        sid = self.start("Use apply_patch once to append NEW on its own line to both safe-a.txt and safe-b.txt. "
                         "Do not use any other tool. Then reply exactly MESIMON_SAFE_PATCH_OK.")
        session, _, _ = self.completed(sid, "MESIMON_SAFE_PATCH_OK")
        expected = {"safe-a.txt": "alpha\nNEW\n", "safe-b.txt": "beta\nNEW\n"}
        if any((self.repo / name).read_text() != content for name, content in expected.items()):
            raise AssertionError("ordinary multi-file apply_patch did not make both expected edits")
        self.mark(assertion="production guard permits ordinary multi-file patch", passed=True)
        def title_matches(session, ticket, screen):
            thread = self.native_read(sid, "thread/read", {"threadId": session["codex_thread_id"], "includeTurns": False})["thread"]
            return bool(thread.get("name") and session.get("title") == thread["name"][:80])
        session, _, _ = self.wait(sid, "structured native title reaches board session metadata", title_matches, timeout=10)
        pane_title = self.tm("display-message", "-p", "-t", sid.replace("-", "")[:16], "#{pane_title}").strip()
        thread = self.native_read(sid, "thread/read", {"threadId": session["codex_thread_id"], "includeTurns": False})["thread"]
        self.mark(observation="native and board title sources after real turn", pane_title=pane_title,
                  thread_name=thread.get("name"), board_title=session.get("title"))
        self.ask(sid, "Use apply_patch once to move rename-source.txt to .mesimon/guard-denied.txt, "
                      "changing its one line from source to moved. If a hook denies it, stop without retrying "
                      "or using another tool and reply exactly MESIMON_GUARD_DENIED. Do not modify anything else.")
        session, _, _ = self.completed(sid, "MESIMON_GUARD_DENIED")
        if (self.repo / ".mesimon/guard-denied.txt").exists() or (self.repo / "rename-source.txt").read_text() != "source\n":
            raise AssertionError("protected rename modified the source or board directory")
        feed = self.state / "activity.jsonl"
        def denied():
            if not feed.is_file():
                return False
            return any(row.get("kind") == "hook" and row.get("event") == "GateDenied"
                       and row.get("session") == sid and row.get("reason") == "board_dir"
                       for row in (json.loads(line) for line in feed.read_text().splitlines()))
        lab.wait_for(denied, timeout=5)
        self.mark(assertion="actual production GateDenied board_dir and unchanged protected rename files", passed=True)
        shutil.copyfile(feed, self.out / "activity.jsonl")
        history = Path(session["transcript_path"])
        history.resolve().relative_to(self.home.resolve())
        patches = patch_invocations(json.loads(line) for line in history.read_text().splitlines())
        if len(patches) < 2:
            raise Inconclusive("native history did not retain both actual apply_patch invocations")
        patch_text = [str(patch.get("input") or patch.get("arguments") or "") for patch in patches]
        if not any("safe-a.txt" in text and "safe-b.txt" in text for text in patch_text):
            raise Inconclusive("native history did not retain one ordinary multi-file patch")
        if not any("*** Move to:" in text and ".mesimon/guard-denied.txt" in text for text in patch_text):
            raise Inconclusive("native history did not retain the protected rename patch")
        self.mark(assertion="actual production gate denied protected rename after allowed multi-file edit",
                  passed=True, apply_patch_invocations=len(patches), model_turns=self.calls)
        shutil.copyfile(feed, self.out / "activity.jsonl")

    def switch(self):
        self.request("set_agent_provider", provider="claude_code")
        claude = self.start("Reply exactly MESIMON_CLAUDE_FIRST. Do not use tools.")
        self.completed(claude, "MESIMON_CLAUDE_FIRST")
        self.request("sleep_session", id=claude)
        self.wait(claude, "old Claude sleeping before provider switch", lambda s, t, p: s["state"]["state"] == "sleeping", consent=False)
        self.request("set_agent_provider", provider="codex")
        codex = self.start("Reply exactly MESIMON_CODEX_FIRST. Do not use tools.")
        original, _, _ = self.completed(codex, "MESIMON_CODEX_FIRST")
        thread = original.get("codex_thread_id")
        if not thread:
            raise AssertionError("new Codex conversation has no exact thread identity")
        self.request("wake_session", id=claude)
        self.wait(claude, "sleeping Claude wakes while Codex is selected", lambda s,t,p:
                  s["kind"] == "claude" and s["state"]["state"] == "idle")
        self.ask(claude, "Reply exactly MESIMON_CLAUDE_OLD_ALIVE. Do not use tools.")
        self.completed(claude, "MESIMON_CLAUDE_OLD_ALIVE")
        self.request("sleep_session", id=codex)
        self.wait(codex, "old Codex sleeping before switching back", lambda s,t,p: s["state"]["state"] == "sleeping" and not s.get("codex_stopping", False), consent=False)
        self.request("set_agent_provider", provider="claude_code")
        self.request("wake_session", id=codex)
        self.wait(codex, "sleeping Codex wakes while Claude is selected", lambda s,t,p:
                  s["kind"] == "codex" and s.get("codex_thread_id") == thread and s["state"]["state"] == "idle"
                  and not s.get("observation_hold", True))
        self.ask(codex, "Reply exactly MESIMON_CODEX_OLD_ALIVE. Do not use tools.")
        resumed, _, _ = self.completed(codex, "MESIMON_CODEX_OLD_ALIVE")
        if resumed.get("codex_thread_id") != thread:
            raise AssertionError("wake substituted a new Codex conversation")
        latest = self.start("Reply exactly MESIMON_CLAUDE_LAST. Do not use tools.")
        final, _, _ = self.completed(latest, "MESIMON_CLAUDE_LAST")
        if final["kind"] != "claude":
            raise AssertionError("new session ignored final project provider")
        self.mark(assertion="Claude to Codex to Claude with both sleeping originals operational", passed=True)

    def queue_external(self):
        if self.args.case == "external":
            first = self.start("Reply exactly MESIMON_QUEUE_FIRST_DONE. Do not use tools.")
            original, _, _ = self.completed(first, "MESIMON_QUEUE_FIRST_DONE")
            thread = original["codex_thread_id"]
            owned = (first,)
        else:
            first = self.start("Run sleep 8 once in the terminal, wait for it to finish, then reply exactly MESIMON_QUEUE_FIRST_DONE. Do not modify files.")
            def active_tool(session, ticket, screen):
                path = Path(session.get("agent_preview_path") or "/does-not-exist")
                value = json.loads(path.read_text()) if path.is_file() else {}
                return session["state"]["state"] == "running" and isinstance(value.get("activity"), dict) and "Tool" in value["activity"]
            self.wait(first, "real native tool holds shared checkout", active_tool)
            queued_ticket = self.request("create_ticket", column="TODO", title="Reply exactly MESIMON_QUEUE_SECOND_DONE. Do not use tools.", workspace=None)["id"]
            response = self.request("prompt_session", ticket=queued_ticket, text="", queued=True)
            if response.get("resp") != "queued":
                raise AssertionError("second shared-checkout start did not queue behind real tool work")
            self.calls += 1
            self.request("set_agent_provider", provider="claude_code")
            time.sleep(0.5)
            if any(session["ticket"] == queued_ticket for session in self.client.board()["sessions"]):
                raise AssertionError("queued second agent started while first tool still held checkout")
            self.mark(assertion="queued new session withheld during real tool work", passed=True)
            original, _, _ = self.completed(first, "MESIMON_QUEUE_FIRST_DONE")
            thread = original["codex_thread_id"]
            lab.wait_for(lambda: any(session["ticket"] == queued_ticket for session in self.client.board()["sessions"]), timeout=20)
            second = next(session for session in self.client.board()["sessions"] if session["ticket"] == queued_ticket)
            if second["kind"] != "codex":
                raise AssertionError("accepted queued start changed provider after project switch")
            self.sessions.append(second["id"])
            self.completed(second["id"], "MESIMON_QUEUE_SECOND_DONE")
            self.mark(assertion="captured Codex provider delivered queued start once after checkout quiet", passed=True)
            owned = (first, second["id"])
        for sid in owned:
            self.request("kill_session", id=sid)
            lab.wait_for(lambda: any(session["id"] == sid and session["state"]["state"] == "exited"
                         and not session.get("codex_stopping", False) for session in self.client.board()["sessions"]), timeout=15)
            self.mark(assertion="owned provider cleanup acknowledged", session=sid, passed=True)
        # Reopen only this synthetic fixture conversation outside board ownership.
        # The ordinary production supervisor still owns its native app-server.
        external_id = str(uuid.uuid4())
        configuration = json.loads((self.state / "hooks" / (first + ".codex.json")).read_text())
        external_snapshot = self.runtime / "external-owned.snapshot.json"
        configuration.update(session=external_id, generation=uuid.uuid4().int & ((1 << 64) - 1),
            upstream_socket=str(self.runtime / "external-up.sock"), proxy_socket=str(self.runtime / "external-ui.sock"),
            snapshot_path=str(external_snapshot), preview_path=str(self.state / "hooks" / (external_id + ".preview.json")), resume=thread)
        configuration["env"] = [[key, external_id if key == "MESIMON_SESSION" else value] for key,value in configuration["env"]]
        external_config = self.state / "hooks" / (external_id + ".codex.json")
        write(external_config, configuration)
        self.tm("new-session", "-d", "-s", "external-fixture", "-c", str(self.repo),
                shlex.join([str(self.args.binary), "agent-runtime", "--config", str(external_config)]))
        def external_state():
            try:
                return json.loads(external_snapshot.read_text())
            except (FileNotFoundError, ValueError):
                return {}
        lab.wait_for(lambda: external_state().get("thread_id") == thread
                     and external_state().get("state", {}).get("state") == "idle"
                     and not external_state().get("observation_hold", True), timeout=30)
        self.verify_native_home(external_id)
        native = external_state()
        Path(native["history_path"]).resolve().relative_to(self.home.resolve())
        discovered = self.request("rescan_external")["external"]
        candidate = next((item for item in discovered if item.get("conversation_id") == thread and item.get("provider") == "codex"), None)
        if not candidate or not candidate["running_elsewhere"]:
            raise AssertionError("external drawer did not identify live owned native fixture conversation")
        target_ticket = self.request("create_ticket", column="TODO", title="Take over synthetic native conversation", workspace=None)["id"]
        refused = self.client.request("resume_external", claude_session_id=candidate["claude_session_id"], ticket=target_ticket, confirm=False)
        if refused.get("resp") != "err" or "Codex process" not in refused.get("message", ""):
            raise AssertionError("takeover did not refuse positively identified live native writer")
        adopted = next(session for session in self.client.board()["sessions"] if session["ticket"] == target_ticket)
        self.sessions.append(adopted["id"])
        self.mark(assertion="external native writer positively identified and takeover refused", passed=True)
        pid = int(self.tm("display-message", "-p", "-t", "external-fixture", "#{pane_pid}").strip())
        os.killpg(pid, signal.SIGTERM)
        lab.wait_for(lambda: external_state().get("stopped", False), timeout=15)
        # The last pane exiting may already have stopped the private server.
        remaining = subprocess.run([self.tmux, "-S", str(self.runtime / "tmux.sock"), "has-session", "-t", "external-fixture"],
                                   capture_output=True, text=True, timeout=5)
        if remaining.returncode == 0:
            self.tm("kill-session", "-t", "external-fixture")
        # A person's explicit takeover follows proven cleanup of our fixture.
        # Unrelated personal processes remain untouched and unread.
        self.request("resume_session", id=adopted["id"], confirm=True)
        recovered, _, _ = self.wait(adopted["id"], "exact external conversation resumed under original provider", lambda session,ticket,screen:
            session["kind"] == "codex" and session.get("codex_thread_id") == thread
            and session["state"]["state"] == "idle" and not session.get("observation_hold", True)
            and "MESIMON_QUEUE_FIRST_DONE" in screen)
        self.mark(assertion="external takeover preserved exact fixture history without a new model turn", passed=True,
                  thread_id=recovered["codex_thread_id"])

    def collect(self):
        for child_id, history in self.child_histories.items():
            history.resolve().relative_to(self.home.resolve())
            if history.is_file() and history.stat().st_size <= 16 * 1024 * 1024:
                shutil.copyfile(history, self.out / (child_id + ".child-history.jsonl"))
                usage = []
                for line in history.read_text().splitlines():
                    record = json.loads(line)
                    if record.get("payload", {}).get("type") == "token_count":
                        usage.append(record["payload"].get("info"))
                self.manifest.setdefault("child_token_usage", {})[child_id] = usage
        for pattern in ("*.app-server.log", "*.runtime-error.log", "cdx-*.json"):
            for path in self.runtime.glob(pattern):
                if path.is_file() and path.stat().st_size <= 16 * 1024 * 1024:
                    shutil.copyfile(path, self.out / path.name)
        for sid in self.sessions:
            session = next(s for s in self.client.board()["sessions"] if s["id"] == sid)
            if session["kind"] == "claude":
                history = Path(session.get("transcript_path") or "/does-not-exist")
                try:
                    history.resolve().relative_to((self.guard.root / "claude-home").resolve())
                except ValueError:
                    raise AssertionError("Claude fixture transcript is outside its isolated home")
                if history.is_file() and history.stat().st_size <= 16 * 1024 * 1024:
                    shutil.copyfile(history, self.out / (sid + ".claude-history.jsonl"))
                    usage = []
                    for line in history.read_text().splitlines():
                        record = json.loads(line)
                        if record.get("type") == "assistant":
                            message = record.get("message", {})
                            usage.append(dict(model=message.get("model"), usage=message.get("usage")))
                    self.manifest.setdefault("token_usage", {})[sid] = usage
                continue
            history = Path(session.get("transcript_path") or "/does-not-exist")
            try:
                history.resolve().relative_to(self.home.resolve())
            except ValueError:
                continue
            if not history.is_file():
                continue
            token_usage = []
            for line in history.read_text().splitlines():
                try:
                    record = json.loads(line)
                except ValueError:
                    continue
                payload = record.get("payload", {})
                if payload.get("type") == "token_count":
                    token_usage.append(payload.get("info"))
            self.manifest.setdefault("token_usage", {})[sid] = token_usage
            if history.stat().st_size <= 16 * 1024 * 1024:
                shutil.copyfile(history, self.out / (sid + ".history.jsonl"))
        daemon_log = self.state / "daemon.log"
        if daemon_log.is_file():
            shutil.copyfile(daemon_log, self.out / "daemon.log")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true", help="explicitly authorize real model turns")
    parser.add_argument("--validate-guard-capture", type=Path, help="offline validation of an owned guard capture; append a separate result without model calls")
    parser.add_argument("--validate-plan-capture", type=Path, help="offline validation of owned plan evidence; preserve the original runner outcome")
    parser.add_argument("--case", choices=("complete", "mcp", "sleep-resume", "handover", "interrupt", "switch", "guard", "plan", "queue-external", "brief", "external", "child", "faults"), default="complete")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--claude", default=shutil.which("claude"))
    parser.add_argument("--claude-model", default="haiku")
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "auth.json")
    args = parser.parse_args()
    if args.case == "child":
        if args.model != "gpt-5.6-luna":
            parser.error("child acceptance pins parent and child to gpt-5.6-luna")
        args.timeout = min(args.timeout, 120)
    if args.case == "faults":
        if args.model != "gpt-5.6-luna":
            parser.error("fault acceptance pins its only model turn to gpt-5.6-luna")
        args.timeout = min(args.timeout, 120)
    if args.validate_plan_capture:
        print(json.dumps(validate_plan_capture(args.validate_plan_capture)))
        return 0
    if args.validate_guard_capture:
        print(json.dumps(validate_guard_capture(args.validate_guard_capture)))
        return 0
    if not args.live:
        parser.error("this runner makes real model calls; pass --live explicitly")
    if not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run this verification through ci/test-run.py for bounded ownership")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-state-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="connected_production_codex_mesimon", case=args.case, model=args.model,
                    outcome="inconclusive", cleanup=False, checkpoints=[], cost_usd=None,
                    cost_note="Actual billing unavailable; native token counts collected when supplied",
                    codex_version=subprocess.check_output([args.codex, "--version"], text=True, timeout=10).strip(),
                    mesimon_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    guard = verification = None
    try:
        guard = probe.Guard(args.timeout + 25, args.tmux)
        verification = Verification(args, out, manifest, guard)
        verification.run()
        if any(not checkpoint.get("passed", True) for checkpoint in manifest["checkpoints"]):
            raise AssertionError("a verification checkpoint failed")
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        if verification:
            try:
                verification.collect()
                verification.client.close()
            except Exception as error:
                manifest["capture_error"] = type(error).__name__ + ": " + str(error)
                manifest["outcome"] = "inconclusive"
        if guard:
            try:
                manifest["cleanup"] = guard.close()
            except Exception as error:
                manifest["cleanup_error"] = type(error).__name__ + ": " + str(error)
        write(out / "manifest.json", manifest)
        print(json.dumps(dict(capture=str(out), outcome=manifest["outcome"], cleanup=manifest["cleanup"], error=manifest.get("error"))))
    return 0 if manifest["outcome"] == "passed" and manifest["cleanup"] else 1


if __name__ == "__main__":
    sys.exit(main())
