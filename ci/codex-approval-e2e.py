#!/usr/bin/env python3
"""One-turn production native command-approval refusal, Luna with low reasoning.

Codex 0.153.4's measured command dialog exposes Yes and No (cancel), not a
separate Deny RPC decision. Select its visible No option with native keys;
record whether refusal interrupts the turn or permits a final model response.
Never manufacture a decline response or substitute an observer for the user.
"""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("approval_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)


def refusal_choice(screen):
    """Parse only the active numbered native approval menu, never history text."""
    if "Would you like to run the following command?" not in screen:
        return None
    dialog = screen.rsplit("Would you like to run the following command?", 1)[1]
    options = list(re.finditer(r"(?m)^\s*([›❯]?)\s*(\d+)\.\s+(.*)$", dialog))
    refusal = [match for match in options if re.match(r"(?:No,|Deny\b|Decline\b)", match[3])]
    selected = [match for match in options if match[1]]
    if len(refusal) != 1 or len(selected) != 1:
        return None
    return dict(index=int(refusal[0][2]), selected=int(selected[0][2]), label=refusal[0][3])


def wrapper_provenance(records, command, turn):
    """Accept the measured native exec wrapper, never arbitrary echoed code."""
    calls = [record.get("payload", {}) for record in records
             if record.get("type") == "response_item"
             and record.get("payload", {}).get("type") in ("custom_tool_call", "function_call")]
    if len(calls) != 1:
        raise AssertionError("native parent must issue exactly one tool wrapper")
    call = calls[0]
    if (call.get("type") != "custom_tool_call" or call.get("name") != "exec"
            or call.get("internal_chat_message_metadata_passthrough", {}).get("turn_id") != turn):
        raise AssertionError("native wrapper identity or turn provenance is absent")
    code = call.get("input", "")
    match = re.fullmatch(r"\s*text\s*\(\s*await\s+tools\.exec_command\s*\((.*)\)\s*\)\s*;\s*", code, re.DOTALL)
    if not match or code.count("tools.exec_command") != 1:
        raise AssertionError("native exec wrapper is not the single requested command call")
    try:
        args = json.loads(match[1])
    except ValueError as error:
        raise AssertionError("native wrapper arguments are not the requested JSON object") from error
    if (not isinstance(args, dict) or args.get("cmd") != command
            or args.get("sandbox_permissions") != "require_escalated"
            or set(args) != {"cmd", "sandbox_permissions", "justification"}
            or args.get("justification") != "May I write this disposable fixture file?"):
        raise AssertionError("native wrapper changed the command or escalation request")
    if not call.get("call_id"):
        raise AssertionError("native exec wrapper has no call identity")
    return dict(call_id=call["call_id"], turn_id=turn, input=code)


class Verification(state.Verification):
    def ask(self, sid, prompt):
        if self.calls != 0:
            raise AssertionError("approval fixture permits only one native prompt")
        super().ask(sid, prompt)

    def verify_native_home(self, sid):
        super().verify_native_home(sid)
        if getattr(self, "policy_checked", False):
            return
        value = self.native_read(sid, "config/read", {"includeLayers": True, "cwd": str(self.repo)})
        policy = value.get("config", {})
        if policy.get("sandbox_mode") != "read-only" or policy.get("approval_policy") != "on-request":
            raise AssertionError("production read-only/on-request column policy did not reach native configuration")
        state.write(self.out / "native-approval-policy.json", dict(model=policy.get("model"),
                    effort=policy.get("model_reasoning_effort"), sandbox=policy.get("sandbox_mode"),
                    approval=policy.get("approval_policy"), model_origin=value.get("origins", {}).get("model")))
        self.policy_checked = True

    def turns(self, sid, thread):
        value = self.native_read(sid, "thread/turns/list", {"threadId": thread, "limit": 2, "itemsView": "full"})
        if value.get("nextCursor") or len(value.get("data", [])) > 1:
            raise AssertionError("approval fixture exceeded its one-turn budget")
        return value

    def history(self, session, artifact):
        path = Path(session.get("transcript_path") or "/missing-native-history")
        path.resolve().relative_to(self.home.resolve())
        records = [json.loads(line) for line in path.read_text().splitlines()]
        shutil.copyfile(path, self.out / artifact)
        return records

    def run(self):
        column = next(row for row in self.client.board()["columns"] if row["name"] == "TODO")
        settings = {key:value for key,value in column.items() if key not in ("name", "order")}
        settings.update(codex_sandbox="read_only", codex_approval="on_request")
        self.request("set_column_settings", name="TODO", settings=settings)
        outside = self.guard.root / "approval-refused.txt"
        outside.resolve().relative_to(self.guard.root.resolve())
        if outside.exists():
            raise AssertionError("fresh owned refusal path unexpectedly exists")
        command = "printf MESIMON_REFUSED_WRITE > " + shlex.quote(str(outside))
        ticket = self.request("create_ticket", column="TODO", title="Native approval refusal fixture", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        self.wait(sid, "unsubmitted native composer ready", lambda s,t,p:
                  s["state"]["state"] == "idle" and not s.get("pending_prefill", True)
                  and not s.get("pending_submit") and not s.get("observation_hold", True))
        self.verify_native_home(sid)
        call = "text(await tools.exec_command(" + json.dumps(dict(cmd=command,
            sandbox_permissions="require_escalated", justification="May I write this disposable fixture file?")) + "));"
        self.ask(sid, "Execute this exact JavaScript in exec once: " + call
                 + " If refused, reply MESIMON_APPROVAL_REFUSED. No alternate writes, other tools, agents, or retries.")
        approved, _, screen = self.wait(sid, "exact native command approval maps to Permission and checkout hold",
            lambda s,t,p: s["state"] == {"state":"requires_action", "reason":"permission"}
            and s.get("observation_hold") and t["column"] == "IN PROGRESS"
            and refusal_choice(p) is not None and "".join(command.split()) in "".join(p.split()))
        thread, turn = approved["codex_thread_id"], approved["codex_turn_id"]
        before = self.turns(sid, thread)
        state.write(self.out / "before-native-refusal.json", dict(turns=before, screen=screen, board=approved))
        history_before = self.history(approved, "before-native-refusal.history.jsonl")
        commands = [item for row in before.get("data", []) for item in row.get("items", [])
                    if item.get("type") == "commandExecution"]
        wrapper = None
        if commands:
            if len(commands) != 1 or command not in commands[0].get("command", "") or commands[0].get("status") != "inProgress":
                raise AssertionError("visible approval lacks exactly one matching pending native command")
            command_id = commands[0]["id"]
        else:
            # Codex code mode can persist the outer custom_tool_call before its
            # nested commandExecution appears in the reconstructed turn items.
            wrapper = wrapper_provenance(history_before, command, turn)
            command_id = None
        state.write(self.out / "pending-command-provenance.json", dict(command_id=command_id, wrapper=wrapper))
        # Preserve a measured unanswered interval. Neither native read connection
        # sends a request response; only the following user-key gesture refuses.
        end = time.monotonic() + 0.6
        while time.monotonic() < end:
            s, t, screen = self.observe(sid)
            if s["state"] != approved["state"] or not s.get("observation_hold") or t["column"] != "IN PROGRESS" or outside.exists():
                raise AssertionError("unanswered command approval released checkout or executed the write")
            time.sleep(0.1)
        choice = refusal_choice(screen)
        if choice is None:
            raise state.Inconclusive("native refusal menu disappeared before the user gesture")
        target = sid.replace("-", "")[:16]
        steps = choice["index"] - choice["selected"]
        if abs(steps) > 5:
            raise state.Inconclusive("native refusal menu exceeds the bounded known layout")
        for _ in range(abs(steps)):
            self.tm("send-keys", "-t", target, "Down" if steps > 0 else "Up")
        time.sleep(0.15)
        selected_screen = self.tm("capture-pane", "-p", "-t", target)
        selected = refusal_choice(selected_screen)
        if not selected or selected["selected"] != selected["index"] or selected["label"] != choice["label"]:
            raise state.Inconclusive("native refusal selection not independently visible; Enter not pressed")
        self.mark(action="native_select_visible_refusal", label=choice["label"], session=sid)
        self.tm("send-keys", "-t", target, "Enter")
        final = None
        saw_native_cancel = False
        while time.monotonic() - self.started < self.args.timeout:
            s, t, screen = self.observe(sid)
            if outside.exists():
                raise AssertionError("refused command created its outside fixture file")
            saw_native_cancel |= bool(re.search(r"You canceled the request to run\s+printf MESIMON_REFUSED_WRITE", screen))
            final = self.turns(sid, thread)
            rows = final.get("data", [])
            if rows and rows[0].get("status") in ("completed", "interrupted", "failed"):
                break
            time.sleep(0.15)
        else:
            raise state.Inconclusive("native refusal exceeded its 120-second deadline; no retry")
        rows = final["data"]
        state.write(self.out / "native-refusal-terminal.json", dict(turns=final, board=s, screen=screen,
                    saw_native_cancel=saw_native_cancel, outside_file_exists=outside.exists()))
        history_after = self.history(s, "after-native-refusal.history.jsonl")
        if wrapper is not None and wrapper_provenance(history_after, command, turn)["call_id"] != wrapper["call_id"]:
            raise AssertionError("native model substituted a different command wrapper")
        commands = [item for row in rows for item in row.get("items", []) if item.get("type") == "commandExecution"]
        command_status = None
        if commands:
            if (len(commands) != 1 or command not in commands[0].get("command", "")
                    or (command_id is not None and commands[0].get("id") != command_id)):
                raise AssertionError("native model attempted more than the one approved command")
            command_result = commands[0]
            command_status = command_result.get("status")
            denial = command_status == "declined" or (
                command_status == "failed" and any(word in (command_result.get("aggregatedOutput") or "").lower()
                                                  for word in ("denied", "rejected", "not approved")))
            if not denial:
                raise AssertionError("native command history did not record a declined/error refusal")
        elif not (wrapper is not None and saw_native_cancel and rows[0].get("status") == "interrupted"):
            raise state.Inconclusive("native nested command outcome absent and no visible cancellation / interrupted-turn proof")
        outcome = rows[0].get("status")
        if outcome == "completed":
            settled, _, _ = self.completed(sid, "MESIMON_APPROVAL_REFUSED")
        elif outcome == "interrupted":
            settled, _, _ = self.wait(sid, "native refusal interruption has no successful automatic movement",
                lambda s,t,p: s["state"] == {"state":"idle", "stop_reason":"interrupted"}
                and not s.get("observation_hold", True) and t["column"] == "IN PROGRESS", consent=False)
        else:
            raise state.Inconclusive("native refusal ended with failure instead of denial continuation or interruption")
        if self.calls != 1 or rows[0]["id"] != turn or settled.get("codex_thread_id") != thread or outside.exists():
            raise AssertionError("refusal changed native identity, exceeded budget, or wrote the file")
        state.write(self.out / "after-native-refusal.json", dict(turns=final, board=settled, outside_file_exists=False))
        self.mark(assertion="native visible refusal declines exact command without writing or false completion",
                  passed=True, native_option=choice["label"], native_turn_outcome=outcome,
                  command_status=command_status, nested_item_available=bool(commands),
                  native_cancellation_visible=saw_native_cancel, model_turns=1, separate_deny_rpc_claimed=False)


def self_test():
    screen = "Would you like to run the following command?\n› 1. Yes, proceed (y)\n  2. Yes, always (p)\n  3. No, and tell Codex what to do differently (esc)\n"
    choice = refusal_choice(screen)
    assert choice["selected"] == 1 and choice["index"] == 3
    assert refusal_choice(screen.replace("› 1.", "  1.").replace("  3.", "› 3."))["selected"] == 3
    assert refusal_choice("old No, text but no current approval") is None
    command, turn = "printf MESIMON_REFUSED_WRITE > /owned/approval-refused.txt", "fixture-turn"
    args = dict(cmd=command, sandbox_permissions="require_escalated",
                justification="May I write this disposable fixture file?")
    call = dict(type="response_item", payload=dict(type="custom_tool_call", name="exec", call_id="fixture-call",
                input="text(await tools.exec_command(" + json.dumps(args) + "));\n",
                internal_chat_message_metadata_passthrough=dict(turn_id=turn)))
    assert wrapper_provenance([call], command, turn)["call_id"] == "fixture-call"
    for records, expected, expected_turn in [([call, call], command, turn), ([call], command + " changed", turn),
                                           ([call], command, "different-turn")]:
        try:
            wrapper_provenance(records, expected, expected_turn)
        except AssertionError:
            continue
        raise AssertionError("invalid native command provenance accepted")
    print("Offline native refusal selection checks passed; no process or model launched")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--self-test", action="store_true", help="check native refusal selection without any process or model")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home()/".codex"))) / "auth.json")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run through ci/test-run.py with --live; exactly one Luna turn is permitted")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    args.case, args.model, args.timeout = "approval", "gpt-5.6-luna", 120
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-approval-no-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="production_native_command_refusal", outcome="inconclusive", cleanup=False,
                    model=args.model, reasoning_effort="low", answer="native_no",
                    model_turn_budget=1, checkpoints=[], cost_usd=None)
    guard = verification = None
    try:
        guard = state.probe.Guard(145, args.tmux)
        verification = Verification(args, out, manifest, guard)
        verification.run()
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
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
