#!/usr/bin/env python3
"""Bounded opt-in native Codex lifecycle checks against the production daemon.

Use ci/test-run.py. Cases never exceed two submitted user turns. Stop also
permits one automatic continuation; compact requests one native summarization.
The CLI provides no enforceable dollar cap: record native usage, including any
system/title activity, and report unavailable billing honestly.

Hook semantics: https://learn.chatgpt.com/docs/hooks (checked 2026-09-10).
All hooks, histories and trust decisions belong to the disposable fixture.
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
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("codex_lifecycle_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)
Inconclusive = state.Inconclusive
HOOK_COMMAND = '"$MESIMON_LIFECYCLE_PYTHON" -B "$MESIMON_LIFECYCLE_SCRIPT" hook'
BUDGETS = {"stop": 1, "compact": 2, "identity": 2, "question": 1}
MAX_FILE = 16 * 1024 * 1024


def fixture_hook():
    """Vetted, bounded fixture handler. Never executes model-provided paths."""
    payload = json.loads(sys.stdin.read(65537))
    root = Path(os.environ["MESIMON_LIFECYCLE_ROOT"]).resolve(strict=True)
    config = json.loads((root / "lifecycle.json").read_text())
    event = payload.get("hook_event_name")
    expected = (root / "expected-thread").read_text().strip() if (root / "expected-thread").exists() else None
    scoped = payload.get("session_id") == expected and expected is not None
    output = {}
    wait_for = None
    count = None
    if scoped and event == "UserPromptSubmit":
        # Includes automatic Stop continuations if the installed CLI delivers
        # their UserPromptSubmit event. CLI/history evidence checks both forms.
        counter = root / "submit-count"
        count = int(counter.read_text()) + 1 if counter.exists() else 1
        counter.write_text(str(count))
        if count > 2:
            output = {"decision": "block", "reason": "Disposable fixture turn budget exhausted."}
    if scoped and event == "Stop" and config["case"] == "stop":
        counter = root / "stop-count"
        count = int(counter.read_text()) + 1 if counter.exists() else 1
        counter.write_text(str(count))
        if count == 1:
            output = {"decision": "block", "reason": "Reply exactly MESIMON_STOP_CONTINUED. Do not use tools or produce further work."}
            wait_for = root / "release-stop-first"
        elif count == 2:
            wait_for = root / "release-stop-second"
        else:
            output = {"continue": False, "stopReason": "Disposable fixture continuation budget exhausted."}
    if scoped and event == "PreCompact":
        wait_for = root / "release-compact"
    receipt = dict(at_ms=int(time.time() * 1000), input=payload, output=output,
                   scoped=scoped, count=count, barrier=wait_for.name if wait_for else None,
                   environment={key: os.environ.get(key) for key in
                                ("MESIMON_SESSION", "MESIMON_GATE_BOARD", "MESIMON_HOOK_SOCK")})
    with (root / "lifecycle-hooks.jsonl").open("a") as log:
        log.write(json.dumps(receipt) + "\n")
    if wait_for:
        deadline = time.monotonic() + 12
        while not wait_for.exists() and time.monotonic() < deadline:
            time.sleep(.05)
        if not wait_for.exists():
            output = {"continue": False, "stopReason": "Disposable fixture barrier deadline expired."}
            receipt = dict(at_ms=int(time.time() * 1000), barrier_timeout=wait_for.name)
            with (root / "lifecycle-hooks.jsonl").open("a") as log:
                log.write(json.dumps(receipt) + "\n")
    print(json.dumps(output))


class Verification(state.Verification):
    def __init__(self, args, out, manifest, guard):
        super().__init__(args, out, manifest, guard)
        self.histories = {}
        self.read_serial = 0
        state.write(guard.root / "lifecycle.json", {"case": args.case})
        events = ["UserPromptSubmit"]
        if args.case == "stop":
            events.append("Stop")
        elif args.case == "compact":
            events.extend(["PreCompact", "PostCompact"])
        state.write(self.home / "hooks.json", {"hooks": {
            event: [{"hooks": [{"type": "command", "command": HOOK_COMMAND, "timeout": 15}]}]
            for event in events
        }})
        wrapper = guard.root / "codex-isolated"
        wrapper.write_text("#!/bin/sh\n" + "\n".join(
            "export " + key + "=" + shlex.quote(value) for key, value in {
                "CODEX_HOME": str(self.home),
                "MESIMON_LIFECYCLE_ROOT": str(guard.root),
                "MESIMON_LIFECYCLE_PYTHON": sys.executable,
                "MESIMON_LIFECYCLE_SCRIPT": str(Path(__file__).resolve()),
            }.items()) + "\nexec " + shlex.quote(args.codex) + ' "$@"\n')
        # Existing developer instructions remain separate from integration hooks.
        self.developer = "MESIMON_LIFECYCLE_DEVELOPER"
        with (self.home / "config.toml").open("a") as config:
            config.write("developer_instructions = " + json.dumps(
                "Disposable fixture sentinel: " + self.developer + ".") + "\n")
        manifest.update(user_turn_budget=BUDGETS[args.case], automatic_continuation_budget=int(args.case == "stop"),
                        manual_compaction_budget=int(args.case == "compact"), hooks_definition=HOOK_COMMAND)

    def hooks(self, event=None):
        path = self.guard.root / "lifecycle-hooks.jsonl"
        if not path.exists():
            return []
        if path.stat().st_size > MAX_FILE:
            raise Inconclusive("fixture hook evidence exceeded bound")
        rows = []
        for line in path.read_text().splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("barrier_timeout"):
                raise Inconclusive("native fixture hook barrier expired: " + row["barrier_timeout"])
            if row.get("scoped") and (event is None or row.get("input", {}).get("hook_event_name") == event):
                rows.append(row)
        return rows

    def startup_consent(self, sid, screen):
        if "Press t to trust all;" not in screen:
            return super().startup_consent(sid, screen)
        if sid not in self.hook_review or sid in self.hook_trusted:
            return
        self.verify_native_home(sid)
        session = next(s for s in self.client.board()["sessions"] if s["id"] == sid)
        history = session.get("transcript_path")
        if not history:
            return
        Path(history).resolve().relative_to(self.home.resolve())
        result = self.native_read(sid, "hooks/list", {"cwds": [str(self.repo)]})
        enabled = [hook for group in result.get("data", []) for hook in group.get("hooks", []) if hook.get("enabled")]
        gate = "'" + str(self.args.binary) + "' gate --provider codex --from-env"
        expected_events = {"userPromptSubmit"}
        if self.args.case == "stop": expected_events.add("stop")
        if self.args.case == "compact": expected_events.update(["preCompact", "postCompact"])
        own = [hook for hook in enabled if hook.get("command") == HOOK_COMMAND]
        if {hook.get("eventName") for hook in own} != expected_events:
            raise AssertionError("native lifecycle hooks differ from exact fixture definitions")
        for hook in enabled:
            valid = ((hook.get("source") == "user" and hook.get("command") == HOOK_COMMAND
                      and hook.get("eventName") in expected_events)
                     or (hook.get("source") == "sessionFlags" and hook.get("command") == gate))
            if not valid:
                raise AssertionError("hook review includes an unvetted definition")
        state.write(self.out / "reviewed-hooks.json", enabled)
        self.tm("send-keys", "-t", sid.replace("-", "")[:16], "t")
        self.hook_trusted.add(sid)
        self.mark(action="native_trust_exact_generated_lifecycle_hooks_and_production_gate", session=sid)

    def verify_native_home(self, sid):
        super().verify_native_home(sid)
        config = self.native_read(sid, "config/read", {"includeLayers": True, "cwd": str(self.repo)})
        if self.developer not in (config.get("config", {}).get("developer_instructions") or ""):
            raise AssertionError("existing developer instructions were replaced")

    def native_read(self, sid, method, params):
        if method not in ("config/read", "hooks/list", "thread/read", "thread/turns/list", "thread/loaded/list", "model/list"):
            raise AssertionError("lifecycle observer may only use read-only native methods")
        result = super().native_read(sid, method, params)
        self.read_serial += 1
        state.write(self.out / f"native-read-{self.read_serial:04}.json", dict(method=method, params=params, result=result))
        return result

    def select_expected(self, sid):
        session, _, _ = self.observe(sid)
        thread = session.get("codex_thread_id")
        if not thread:
            raise Inconclusive("native conversation identity is missing")
        (self.guard.root / "expected-thread").write_text(thread)
        native = self.native_read(sid, "thread/read", {"threadId": thread, "includeTurns": False})["thread"]
        if native.get("cwd") != str(self.repo) or native.get("id") != thread:
            raise AssertionError("native selection changed the fixture working directory or identity")
        path = native.get("path")
        if path:
            Path(path).resolve().relative_to(self.home.resolve())
            self.histories[thread] = Path(path)
        self.mark(assertion="selected native thread retains exact owned cwd and scoped session", session=sid, thread=thread, passed=True)
        return thread

    def spawn_unsubmitted(self):
        ticket = self.request("create_ticket", column="TODO", title="Native lifecycle fixture", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        self.wait(sid, "native input and editable prefill ready", lambda session, ticket, screen:
                  session["state"]["state"] == "idle" and not session.get("observation_hold", True)
                  and not session.get("pending_prefill", True) and "Native lifecycle fixture" in screen)
        self.tm("send-keys", "-t", sid.replace("-", "")[:16], "C-u")
        self.select_expected(sid)
        return sid

    def ask(self, sid, prompt):
        if self.calls >= BUDGETS[self.args.case]:
            raise Inconclusive("strict lifecycle user-turn budget reached")
        super().ask(sid, prompt)

    def slash(self, sid, command):
        session, _, screen = self.observe(sid)
        if session["state"]["state"] != "idle" or session.get("pending_submit"):
            raise Inconclusive("native slash command requires verified idle input")
        if "Implement this plan?" in screen:
            raise Inconclusive("native plan dialog is still open")
        target = sid.replace("-", "")[:16]
        self.tm("send-keys", "-t", target, "C-u")
        self.tm("send-keys", "-t", target, "-l", command)
        time.sleep(.4)
        self.tm("send-keys", "-t", target, "Enter")
        self.mark(action="native_slash_command", session=sid, command=command)

    def page(self, sid):
        session, _, _ = self.observe(sid)
        return self.native_read(sid, "thread/turns/list", {"threadId": session["codex_thread_id"], "limit": 5, "itemsView": "full"})

    def held_barrier(self, sid, name, seconds=1.8):
        receipt = next(row for row in reversed(self.hooks()) if row.get("barrier") == name)
        expected_turn = receipt["input"]["turn_id"]
        propagation = time.monotonic() + 1.5
        while True:
            session, ticket, _ = self.observe(sid)
            if ticket["column"] == "REVIEW":
                raise AssertionError("native lifecycle work moved the ticket before its barrier released")
            if session.get("codex_turn_id") == expected_turn:
                break
            if time.monotonic() >= propagation:
                raise AssertionError("native lifecycle turn was not observed within the propagation bound")
            time.sleep(.05)
        # The external hook receipt can precede the next board observation.
        # Compare completion only to this operation's exact native turn, not
        # the previous task's already-published successful result.
        deadline = time.monotonic() + seconds
        explicit_hold_seen = False
        while time.monotonic() < deadline:
            session, ticket, _ = self.observe(sid)
            if session["state"] == {"state": "idle", "stop_reason": "end_turn"} or ticket["column"] == "REVIEW":
                raise AssertionError("native pending lifecycle work falsely completed the ticket")
            explicit_hold_seen |= bool(session.get("observation_hold"))
            # Shared checkout safety includes working/attention/unknown state;
            # observation_hold is the additional hold, not the whole policy.
            if not session.get("observation_hold") and session["state"]["state"] not in ("running", "requires_action", "unknown", "spawning"):
                raise AssertionError("native lifecycle hook did not hold checkout operations")
            time.sleep(.1)
        (self.guard.root / name).write_text("release")
        self.mark(assertion="native lifecycle barrier withholds successful completion and checkout", barrier=name,
                  explicit_observation_hold_seen=explicit_hold_seen, passed=True)

    def stop(self):
        sid = self.spawn_unsubmitted()
        self.ask(sid, "Reply exactly MESIMON_STOP_FIRST. Do not use tools.")
        self.wait(sid, "first real Stop hook reached controlled barrier", lambda s,t,p: len(self.hooks("Stop")) >= 1, timeout=45)
        self.held_barrier(sid, "release-stop-first")
        self.wait(sid, "automatic continuation reaches second Stop hook", lambda s,t,p: len(self.hooks("Stop")) >= 2, timeout=45)
        stops = self.hooks("Stop")
        if len(stops) != 2 or "MESIMON_STOP_CONTINUED" not in (stops[1]["input"].get("last_assistant_message") or ""):
            raise AssertionError("Stop did not produce exactly one expected automatic continuation")
        self.held_barrier(sid, "release-stop-second")
        self.completed(sid, "MESIMON_STOP_CONTINUED")
        page = self.page(sid)
        if page.get("nextCursor") or not 1 <= len(page.get("data", [])) <= 2:
            raise AssertionError("Stop continuation exceeded its bounded native turns")
        if len(self.hooks("Stop")) != 2:
            raise AssertionError("Stop hook ran beyond its block-once contract")
        self.mark(assertion="native Stop continues once without premature board completion", passed=True, user_turns=self.calls, hook_stops=2)

    def compact(self):
        sid = self.spawn_unsubmitted()
        self.ask(sid, "Remember fixture marker MESIMON_COMPACT_MEMORY. Reply exactly MESIMON_COMPACT_READY. Do not use tools.")
        self.completed(sid, "MESIMON_COMPACT_READY")
        session, ticket, _ = self.observe(sid)
        thread = session["codex_thread_id"]
        # Preserve an independent no-completion column oracle through compaction.
        self.request("move_ticket", id=ticket["id"], column="IN PROGRESS", before=None)
        self.slash(sid, "/compact")
        self.wait(sid, "manual PreCompact hook reached controlled barrier", lambda s,t,p: bool(self.hooks("PreCompact")), timeout=20)
        if self.hooks("PreCompact")[-1]["input"].get("trigger") != "manual":
            raise AssertionError("compaction was not explicitly manual")
        self.held_barrier(sid, "release-compact")
        self.wait(sid, "manual PostCompact observed on same native thread", lambda s,t,p:
                  bool(self.hooks("PostCompact")) and s.get("codex_thread_id") == thread, timeout=55)
        self.wait(sid, "manual compaction returns input without successful automove", lambda s,t,p:
                  s["state"]["state"] == "idle" and not s.get("observation_hold", True)
                  and t["column"] == "IN PROGRESS", timeout=15)
        page = self.page(sid)
        if not any(item.get("type") == "contextCompaction" for turn in page.get("data", []) for item in turn.get("items", [])):
            raise Inconclusive("native compact hooks ran but no authoritative ContextCompaction item was retained")
        self.ask(sid, "Reply with the remembered MESIMON_COMPACT_MEMORY marker and MESIMON_COMPACT_ALIVE only. Do not use tools.")
        self.completed(sid, "MESIMON_COMPACT_ALIVE")
        if self.select_expected(sid) != thread:
            raise AssertionError("compaction changed native conversation identity")
        self.mark(assertion="manual compaction preserves native identity and subsequent prompt workflow", passed=True, user_turns=self.calls)

    def identity(self):
        sid = self.spawn_unsubmitted()
        self.ask(sid, "Reply exactly MESIMON_IDENTITY_ORIGINAL. Do not use tools.")
        self.completed(sid, "MESIMON_IDENTITY_ORIGINAL")
        original = self.select_expected(sid)
        self.slash(sid, "/new")
        self.wait(sid, "native new selects a distinct empty conversation", lambda s,t,p: s.get("codex_thread_id") not in (None, original) and s["state"]["state"] == "idle")
        fresh = self.select_expected(sid)
        self.slash(sid, "/resume " + original)
        self.wait(sid, "native resume selects exact original conversation", lambda s,t,p: s.get("codex_thread_id") == original and s["state"]["state"] == "idle")
        self.select_expected(sid)
        self.slash(sid, "/fork")
        self.wait(sid, "native fork selects a distinct conversation", lambda s,t,p: s.get("codex_thread_id") not in (None, original, fresh) and s["state"]["state"] == "idle")
        fork = self.select_expected(sid)
        native = self.native_read(sid, "thread/read", {"threadId": fork, "includeTurns": False})["thread"]
        if native.get("forkedFromId") != original:
            raise AssertionError("native fork lacks exact original ancestry")
        self.ask(sid, "Reply exactly MESIMON_IDENTITY_FORK_ALIVE. Do not use tools.")
        self.completed(sid, "MESIMON_IDENTITY_FORK_ALIVE")
        submits = self.hooks("UserPromptSubmit")
        if {row["input"].get("session_id") for row in submits} != {original, fork}:
            raise AssertionError("native original/fork prompts lack independently scoped hook receipts")
        if any(row["environment"].get("MESIMON_SESSION") != sid
               or row["input"].get("cwd") != str(self.repo) for row in submits):
            raise AssertionError("native identity transition changed real hook ticket environment")
        board = self.client.board()
        selected = next(session for session in board["sessions"] if session["id"] == sid)
        seats = [session for session in board["sessions"] if session["ticket"] == selected["ticket"]
                 and session["kind"] in ("claude", "codex")]
        if len(seats) != 1:
            raise AssertionError("native identity selection created a competing Mesimon agent seat")
        self.mark(assertion="native new resume and fork retain one Mesimon seat and owned environment", passed=True, original=original, fresh=fresh, fork=fork)

    def question(self):
        sid = self.spawn_unsubmitted()
        self.slash(sid, "/plan")
        self.wait(sid, "native Plan mode enabled", lambda s,t,p: "Plan mode" in p)
        self.ask(sid, "Use request_user_input once to ask 'Choose fixture color?' with exactly two options Blue and Green. After my answer, reply exactly MESIMON_QUESTION_BLUE. Do not propose a plan or use any other tools.")
        self.wait(sid, "native question and board attention agree", lambda s,t,p:
                  s["state"] == {"state":"requires_action", "reason":"question"}
                  and s.get("observation_hold") and "Choose fixture color?" in p and "Blue" in p)
        self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Enter")
        self.mark(action="native_select_Blue_question_answer", session=sid)
        self.completed(sid, "MESIMON_QUESTION_BLUE")
        self.page(sid)
        self.mark(assertion="native question resolves through original TUI and releases board attention", passed=True)

    def run(self):
        getattr(self, self.args.case)()

    def collect(self):
        hook_log = self.guard.root / "lifecycle-hooks.jsonl"
        if hook_log.is_file() and hook_log.stat().st_size <= MAX_FILE:
            shutil.copyfile(hook_log, self.out / hook_log.name)
        for thread, path in self.histories.items():
            path.resolve().relative_to(self.home.resolve())
            if path.is_file() and path.stat().st_size <= MAX_FILE:
                shutil.copyfile(path, self.out / (thread + ".selected-history.jsonl"))
        self.manifest["submitted_user_turns"] = self.calls
        super().collect()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--case", choices=tuple(BUDGETS), default="stop")
    parser.add_argument("--timeout", type=int, default=150)
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "auth.json")
    args = parser.parse_args()
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("explicit --live and ci/test-run.py supervision are required")
    if args.model != "gpt-5.6-luna":
        parser.error("lifecycle acceptance pins the economical Luna model")
    if not 1 <= args.timeout <= 180 or not args.codex or not args.tmux:
        parser.error("installed Codex/tmux and a deadline of 1..180 seconds are required")
    args.binary = args.binary.resolve(strict=True)
    args.claude = None
    args.claude_model = "haiku"
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-lifecycle-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="connected_codex_lifecycle", case=args.case, model=args.model,
                    outcome="inconclusive", cleanup=False, checkpoints=[], cost_usd=None,
                    cost_note="Native token usage retained; internal title/compaction calls may add usage. No enforceable dollar cap.",
                    codex_version=subprocess.check_output([args.codex, "--version"], text=True, timeout=10).strip(),
                    codex_binaries=state.probe.identities(args.codex),
                    mesimon_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    guard = verification = None
    try:
        guard = state.probe.Guard(args.timeout + 25, args.tmux)
        verification = Verification(args, out, manifest, guard)
        verification.run()
        if any(not point.get("passed", True) for point in manifest["checkpoints"]):
            raise AssertionError("a lifecycle checkpoint failed")
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        if verification:
            try:
                verification.collect()
            except Exception as error:
                manifest.update(capture_error=str(error), outcome="inconclusive")
            finally:
                verification.client.close()
        if guard:
            try:
                manifest["cleanup"] = guard.close()
            except Exception as error:
                manifest["cleanup_error"] = str(error)
        state.write(out / "manifest.json", manifest)
        print(json.dumps(dict(capture=str(out), outcome=manifest["outcome"], cleanup=manifest["cleanup"], error=manifest.get("error"))))
    return 0 if manifest["outcome"] == "passed" and manifest["cleanup"] else 1


if __name__ == "__main__":
    if sys.argv[1:] == ["hook"]:
        fixture_hook()
    else:
        sys.exit(main())
