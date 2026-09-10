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
            'cli_auth_credentials_store = "file"\ncheck_for_update_on_startup = false\n'
            + "model = " + json.dumps(args.model) + "\n")
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
        self.client.request("set_mcp_tools", on=args.case == "mcp")
        self.sessions = []
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
                    raise Inconclusive("read-only fixture configuration audit was refused")
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
        if self.args.case == "mcp" and "get_ticket" in screen and "mesimon" in screen.lower() and "Allow" in screen:
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
            if session["kind"] == "codex" and session.get("observation_hold", True):
                return False
            preview = Path(session.get("agent_preview_path") or "/does-not-exist")
            preview_text = preview.read_text() if preview.is_file() else ""
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

    def collect(self):
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
    parser.add_argument("--case", choices=("complete", "mcp", "sleep-resume", "handover", "interrupt", "switch"), default="complete")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--claude", default=shutil.which("claude"))
    parser.add_argument("--claude-model", default="haiku")
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "auth.json")
    args = parser.parse_args()
    if not args.live:
        parser.error("this runner makes real model calls; pass --live explicitly")
    if not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run this verification through ci/test-run.py for bounded ownership")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-state-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True)
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
