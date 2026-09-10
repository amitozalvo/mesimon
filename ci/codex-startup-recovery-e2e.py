#!/usr/bin/env python3
"""Zero-turn production startup recovery against the real native Codex TUI.

Run through ci/test-run.py with --live. The owned, home-pinned CLI wrapper fails
only its first app-server invocation before selection. Explicit human recovery
must retry that proven startup on the original Codex seat despite a project
provider switch. No prompt, prefill, task or observer response is submitted.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("startup_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)


def reject_user_turns(records):
    for record in records:
        payload = record.get("payload", {})
        if (record.get("type") == "event_msg" and payload.get("type") in ("task_started", "user_message")):
            raise AssertionError("zero-turn startup fixture unexpectedly started a native task")
        if record.get("type") == "response_item" and payload.get("role") == "user":
            raise AssertionError("zero-turn startup fixture unexpectedly wrote a user conversation item")


def injected_wrapper(original, marker, attempts):
    if not original.startswith("#!/bin/sh\n") or "export CODEX_HOME=" not in original:
        raise AssertionError("existing CLI wrapper does not pin fixture isolation")
    return ("#!/bin/sh\n"
            "for fixture_arg in \"$@\"; do\n"
            "if [ \"$fixture_arg\" = app-server ]; then\n"
            "  printf '%s\\n' app-server >> " + shlex.quote(str(attempts)) + "\n"
            "  if mkdir " + shlex.quote(str(marker)) + " 2>/dev/null; then\n"
            "    printf '%s\\n' 'MESIMON_FIXTURE_FIRST_APP_SERVER_EXIT' >&2\n"
            "    exit 1\n"
            "  fi\n"
            "  break\n"
            "fi\n"
            "done\n" + original.split("\n", 1)[1])


def self_test():
    reject_user_turns([{"type":"session_meta","payload":{"id":"fixture"}}])
    for record in [
        {"type":"event_msg","payload":{"type":"task_started","turn_id":"fixture"}},
        {"type":"event_msg","payload":{"type":"user_message"}},
        {"type":"response_item","payload":{"role":"user","content":[]}},
    ]:
        try:
            reject_user_turns([record])
        except AssertionError:
            pass
        else:
            raise AssertionError("unexpected user turn passed the zero-turn audit")
    original = '#!/bin/sh\nexport CODEX_HOME="/owned/home"\nexec /owned/codex "$@"\n'
    wrapper = injected_wrapper(original, Path("/owned/first failure"), Path("/owned/attempts"))
    assert wrapper.endswith(original.split("\n", 1)[1])
    assert wrapper.count("exit 1") == 1 and '"$fixture_arg" = app-server' in wrapper
    assert 'for fixture_arg in "$@"' in wrapper  # Native config flags precede the subcommand.
    assert "'/owned/first failure'" in wrapper
    print("Startup recovery fixture self-test passed; no process or model launched")


class Verification(state.Verification):
    def __init__(self, args, out, manifest, guard):
        super().__init__(args, out, manifest, guard)
        self.failure_marker = guard.root / "first-app-server-failed"
        self.attempts = guard.root / "app-server-attempts.txt"
        wrapper = guard.root / "codex-isolated"
        wrapper.write_text(injected_wrapper(wrapper.read_text(), self.failure_marker, self.attempts))
        wrapper.chmod(0o700)

    def ask(self, *_args, **_kwargs):
        raise AssertionError("startup recovery verification must never submit a prompt")

    def start(self, *_args, **_kwargs):
        raise AssertionError("startup recovery verification must never create a submitted prompt")

    def run(self):
        ticket = self.request("create_ticket", column="TODO", title="Startup recovery fixture", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        config_path = self.state / "hooks" / (sid + ".codex.json")
        failed = raw = config = None
        deadline = min(self.started + self.args.timeout, time.monotonic() + 30)
        while time.monotonic() < deadline:
            board = self.client.board()
            session = next(row for row in board["sessions"] if row["id"] == sid)
            if session.get("pending_submit") or session.get("codex_turn_id"):
                raise AssertionError("failed startup unexpectedly owed or started a submitted turn")
            if config_path.is_file():
                config = json.loads(config_path.read_text())
                snapshot_path = Path(config["snapshot_path"])
                if snapshot_path.is_file():
                    raw = json.loads(snapshot_path.read_text())
            if session["state"]["state"] == "exited" and session.get("codex_stopping") and raw:
                failed = session
                break
            time.sleep(0.1)
        if not failed:
            raise state.Inconclusive("first injected app-server exit did not reach unverified-cleanup Exited before deadline")
        if (not self.failure_marker.is_dir() or self.attempts.read_text().splitlines() != ["app-server"]
                or failed["kind"] != "codex" or failed.get("codex_thread_id")
                or raw.get("launch_phase") != "before_selection" or raw.get("thread_id")
                or raw.get("stopped") or not raw.get("observation_hold")
                or config.get("resume") is not None or raw["generation"] != failed.get("codex_generation")):
            raise AssertionError("startup retry lacks exact before-selection/no-conversation failure evidence")
        original_generation = failed["codex_generation"]
        state.write(self.out / "failed-startup-snapshot.json", raw)
        state.write(self.out / "failed-startup-session.json", failed)
        self.mark(assertion="explicitly injected first app-server exit leaves held before-selection evidence", passed=True,
                  session=sid, generation=original_generation, model_turns=0)
        self.request("set_agent_provider", provider="claude_code")
        warning = self.client.request("resume_session", id=sid, confirm=False)
        state.write(self.out / "startup-retry-warning.json", warning)
        if (warning.get("resp") != "err" or "unknown child processes may remain" not in warning.get("message", "")
                or "retry startup" not in warning.get("message", "")):
            raise AssertionError("startup retry did not require explicit unknown-child acknowledgment")
        unchanged = next(row for row in self.client.board()["sessions"] if row["id"] == sid)
        if unchanged.get("codex_generation") != original_generation or not unchanged.get("codex_stopping"):
            raise AssertionError("first acknowledgment gesture silently retried startup")
        resumed = self.request("resume_session", id=sid, confirm=True)
        if resumed.get("resp") != "spawned" or resumed.get("id") != sid or not resumed.get("fresh"):
            raise AssertionError("confirmed before-selection retry did not preserve the original seat")
        session, _, screen = self.wait(sid, "confirmed retry reaches real native Codex composer without a user turn", lambda s,t,p:
            s["kind"] == "codex" and s.get("codex_generation") != original_generation
            and bool(s.get("codex_thread_id")) and s["state"]["state"] == "idle"
            and not s.get("observation_hold", True) and not s.get("pending_prefill")
            and not s.get("pending_submit") and "›" in p, timeout=50)
        self.verify_native_home(sid)
        config = json.loads(config_path.read_text())
        snapshot = json.loads(Path(config["snapshot_path"]).read_text())
        if (snapshot.get("launch_phase") != "selected" or snapshot.get("thread_id") != session["codex_thread_id"]
                or snapshot.get("generation") != session["codex_generation"]
                or config.get("resume") is not None or session.get("codex_turn_id") or self.calls):
            raise AssertionError("retry did not reach an actual selected, zero-turn native session")
        native = self.native_read(sid, "thread/read", {"threadId":session["codex_thread_id"], "includeTurns":False})
        thread = native.get("thread", {})
        if thread.get("id") != session["codex_thread_id"] or thread.get("status", {}).get("type") != "idle":
            raise AssertionError("read-only native metadata does not confirm the selected idle thread")
        state.write(self.out / "selected-native-metadata.json", native)
        board = self.client.board()
        seats = [row for row in board["sessions"] if row["ticket"] == ticket]
        if len(seats) != 1 or seats[0]["id"] != sid or seats[0]["kind"] != "codex":
            raise AssertionError("provider switch or retry duplicated/replaced the original Codex seat")
        if self.attempts.read_text().splitlines() != ["app-server", "app-server"]:
            raise AssertionError("fixture exceeded its one failed startup plus one real retry")
        archives = list((self.state / "hooks").glob(sid + ".cleanup-*.json"))
        if len(archives) != 1:
            raise AssertionError("old unverified-cleanup evidence was not retained exactly once")
        archive = json.loads(archives[0].read_text())
        if not archive.get("unknown_descendants_may_remain") or archive["snapshot"] != raw:
            raise AssertionError("startup recovery did not retain original uncertain snapshot")
        shutil.copyfile(archives[0], self.out / "acknowledged-startup-evidence.json")
        history_files = list(self.home.glob("sessions/**/*.jsonl"))
        for history in history_files:
            history.resolve().relative_to(self.home.resolve())
            if history.stat().st_size > 4 * 1024 * 1024:
                raise AssertionError("zero-turn native history exceeds fixture audit bound")
            reject_user_turns([json.loads(line) for line in history.read_text().splitlines()])
        if self.calls or session.get("pending_submit"):
            raise AssertionError("zero-model-call fixture submitted input")
        self.mark(assertion="real native retry retains Codex across project switch with zero task/history user turns", passed=True,
                  session=sid, thread_id=session["codex_thread_id"], generation=session["codex_generation"],
                  native_app_server_attempts=2, history_files=len(history_files), model_turns=0)

    def collect(self):
        if self.attempts.exists():
            shutil.copyfile(self.attempts, self.out / "app-server-attempts.txt")
        super().collect()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home()/".codex"))) / "auth.json")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run through ci/test-run.py with --live; no model turn is permitted")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    args.case, args.model, args.timeout = "startup-recovery", "gpt-5.6-luna", 90
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-startup-recovery-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="real_native_startup_recovery_with_first_app_server_exit_injection", outcome="inconclusive", cleanup=False,
                    model_turn_budget=0, model_turns_submitted=0, billing_calls=0, timeout_seconds=90,
                    model=args.model, reasoning_effort="low", checkpoints=[],
                    mesimon_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    guard = verification = None
    def deadline(_signum, _frame):
        raise state.Inconclusive("90-second whole startup recovery deadline exceeded")
    signal.signal(signal.SIGALRM, deadline)
    signal.alarm(90)
    try:
        guard = state.probe.Guard(115, args.tmux)
        verification = Verification(args, out, manifest, guard)
        verification.run()
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        signal.alarm(0)  # Cleanup has the guard's remaining bounded allowance.
        if verification:
            manifest["model_turns_submitted"] = verification.calls
            try:
                verification.collect()
                verification.client.close()
            except Exception as error:
                manifest.update(outcome="inconclusive", capture_error=str(error))
        if guard:
            try:
                manifest["cleanup"] = guard.close()
            except Exception as error:
                manifest["cleanup_error"] = str(error)
        state.write(out / "manifest.json", manifest)
        print(json.dumps(dict(capture=str(out), **manifest)))
    return 0 if manifest["outcome"] == "passed" and manifest["cleanup"] else 1


if __name__ == "__main__":
    sys.exit(main())
