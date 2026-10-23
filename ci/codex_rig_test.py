"""Offline selection, model preflight and budget checks for the Codex rig."""
import argparse
import importlib.util
import json
import subprocess
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location("codex_rig", Path(__file__).parent / "rig/codex.py")
rig = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rig)


def arguments(**changes):
    values = dict(suite="smoke", only=None, failed=False, case_timeout=180,
                  max_turns=12, max_tokens=120000)
    return argparse.Namespace(**(values | changes))


class SelectionTests(unittest.TestCase):
    def test_failed_rerun_preserves_model_and_budgets_unless_explicitly_overridden(self):
        parser = argparse.ArgumentParser()
        rig.add_arguments(parser)
        args = parser.parse_args(["--max-tokens", "500"])
        args.failed = True
        previous = dict(model="small", effort="medium", max_turns=3, max_tokens=1000, case_timeout=60)
        rig.resolve_options(args, previous)
        self.assertEqual((args.model, args.effort, args.max_turns, args.max_tokens, args.case_timeout),
                         ("small", "medium", 3, 500, 60))

    def test_smoke_is_bounded_and_excludes_extra_model_activity(self):
        cases = rig.select_cases(arguments(), None)
        self.assertEqual(cases, list(rig.SMOKE))
        self.assertEqual(sum(rig.TURNS[case] for case in cases), 10)
        self.assertTrue(set(cases).isdisjoint(rig.EXTENDED))

    def test_unknown_duplicate_and_over_budget_selections_fail_before_launch(self):
        for args in (arguments(only="complete,typo"), arguments(only="complete,complete"),
                     arguments(max_turns=9), arguments(case_timeout=0)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                rig.select_cases(args, None)

    def test_failed_reruns_include_inconclusive_and_unattempted_cases(self):
        previous = {"cases": [{"case": case, "outcome": outcome} for case, outcome in
                    [("complete", "passed"), ("guard", "failed"), ("mcp", "inconclusive"),
                     ("brief", "not_run")]]}
        self.assertEqual(rig.select_cases(arguments(failed=True), previous), ["guard", "mcp", "brief"])
        self.assertEqual(rig.select_cases(arguments(failed=True, only="complete,mcp"), previous), ["mcp"])
        with self.assertRaises(ValueError):
            rig.select_cases(arguments(failed=True), None)


class PreflightTests(unittest.TestCase):
    def test_plan_and_child_configuration_cannot_inherit_a_costlier_default(self):
        original = 'model = "small"\nmodel_reasoning_effort = "low"\ndeveloper_instructions = "sentinel"\n'
        for case in ("complete", "child"):
            args = argparse.Namespace(case=case, model="small", effort="low")
            source = original + ('[agents]\nmax_threads = 1\n' if case == "child" else "")
            config = tomllib.loads(rig.fixture_configuration(source, args))
            self.assertEqual(config["plan_mode_reasoning_effort"], "low")
            self.assertEqual(config["developer_instructions"], "sentinel")
            self.assertEqual(config["agents"]["enabled"], case == "child")
            self.assertEqual(config["agents"]["default_subagent_model"], "small")
            self.assertEqual(config["agents"]["default_subagent_reasoning_effort"], "low")
            if case == "child":
                self.assertEqual(config["agents"]["max_threads"], 1)

    def test_exact_model_and_supported_effort_are_required_across_pages(self):
        selected = {"model": "small", "supportedReasoningEfforts": [{"reasoningEffort": "low"}]}
        pages = [{"data": [{"model": "expensive", "isDefault": True}]}, {"data": [selected]}]
        self.assertEqual(rig.choose_model(pages, "small", "low"), selected)
        for model, effort in (("missing", "low"), ("small", "high")):
            with self.subTest(model=model, effort=effort), self.assertRaises(ValueError):
                rig.choose_model(pages, model, effort)
        with self.assertRaises(ValueError):
            rig.choose_model([], "small", "low")

    def test_cumulative_usage_is_not_summed_for_every_notification(self):
        rows = [{"payload": {"type": "token_count", "info": {"total_token_usage": {"total_tokens": total}}}}
                for total in (10, 10, 25, 25)]
        rows.append({"payload": {"type": "token_count", "info": None}})
        self.assertEqual(rig.usage_of(rows), 25)

    def test_history_cannot_escape_fixture_even_through_a_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / "home"
            home.mkdir()
            outside = root / "other-history"
            outside.write_text('{}\n')
            link = home / "history"
            link.symlink_to(outside)
            with self.assertRaises(ValueError):
                rig.read_history(link, home)

    def test_turn_limit_prevents_submission(self):
        class Base:
            def start(self, prompt):
                raise RuntimeError("must never submit")
        verification = object.__new__(rig.adapter(Base))
        verification.args = argparse.Namespace(user_turn_budget=1)
        verification.calls = 1
        with self.assertRaisesRegex(AssertionError, "submission budget"):
            verification.start("hello")

    def test_token_limit_prevents_submission(self):
        class Base:
            def start(self, prompt):
                raise RuntimeError("must never submit")
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            (home / "sessions").mkdir()
            (home / "sessions/thread.jsonl").write_text(json.dumps({"payload": {
                "type": "token_count", "info": {"total_token_usage": {"total_tokens": 100}}}}) + "\n")
            verification = object.__new__(rig.adapter(Base))
            verification.args = argparse.Namespace(user_turn_budget=2, remaining_tokens=100)
            verification.calls = 1
            verification.home = home
            verification.manifest = {}
            with self.assertRaisesRegex(AssertionError, "token allowance"):
                verification.start("hello")

    def test_report_preserves_inconclusive_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = dict(version="test", model="small", effort="low", cases=[
                dict(case="complete", outcome="inconclusive", cleanup=False, error="cleanup failed")])
            rig.report(Path(directory), manifest)
            saved = json.loads((Path(directory) / "verdicts.json").read_text())
            self.assertEqual(saved, manifest)
            self.assertIn("inconclusive", (Path(directory) / "verdicts.md").read_text())


def call(name, text=""):
    kind = "custom_tool_call" if name == "exec" else "function_call"
    field = "input" if kind == "custom_tool_call" else "arguments"
    return {"type": "response_item", "payload": {"type": kind, "name": name, field: text}}


def said(role, text, phase=None):
    payload = {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}
    if phase:
        payload["phase"] = phase
    return {"type": "response_item", "payload": payload}


class CrownTests(unittest.TestCase):
    """T-916: the crowned Codex cases' selection, budgets and evidence readers."""

    def test_crown_cases_are_opt_in_with_their_own_deadline(self):
        self.assertTrue(rig.CROWN <= set(rig.EXTENDED) and rig.CROWN.isdisjoint(rig.SMOKE))
        cases = rig.select_cases(arguments(only="delegate,forge", case_timeout=None), None)
        self.assertEqual(cases, ["delegate", "forge"])
        self.assertEqual(sum(rig.TURNS[case] for case in cases), 2, "one person's request each")
        args = arguments(case_timeout=None)
        self.assertEqual([rig.case_timeout(args, case) for case in ("delegate", "complete")], [300, 180])
        self.assertEqual(rig.case_timeout(arguments(case_timeout=120), "delegate"), 120)
        parser = argparse.ArgumentParser()
        rig.add_arguments(parser)
        fresh = parser.parse_args([])
        fresh.failed = False
        rig.resolve_options(fresh, None)
        self.assertIsNone(fresh.case_timeout, "no flag keeps each case's own deadline")
        for only, tokens in (("complete", 600000), ("forge", 2000000), ("delegate,forge", 3000000)):
            args = arguments(only=only, max_tokens=None)
            rig.select_cases(args, None)
            self.assertEqual(args.max_tokens, tokens, only)
        args = arguments(only="forge", max_tokens=5)
        rig.select_cases(args, None)
        self.assertEqual(args.max_tokens, 5, "an explicit allowance stands")

    def test_delegate_offers_codex_s_own_children_bounded_and_forge_does_not(self):
        original = 'model = "small"\nmodel_reasoning_effort = "low"\n'
        for case, enabled in (("delegate", True), ("forge", False)):
            config = tomllib.loads(rig.fixture_configuration(original, argparse.Namespace(
                case=case, model="small", effort="low")))
            self.assertEqual(config["agents"]["enabled"], enabled)
            self.assertEqual(config["features"]["multi_agent"], enabled)
            self.assertEqual(config["agents"].get("max_threads"), 1 if enabled else None)
            self.assertEqual(config["agents"]["default_subagent_model"], "small")

    def test_native_turn_allowance_stops_a_crown_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            (home / "sessions").mkdir()
            rows = [{"type": "turn_context", "payload": {}}] * (rig.NATIVE_TURNS["forge"] + 1)
            (home / "sessions/thread.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
            verification = object.__new__(rig.adapter(object))
            verification.args = argparse.Namespace(case="forge", remaining_tokens=100)
            verification.home = home
            verification.manifest = {}
            verification.last_usage_check = 0
            with self.assertRaisesRegex(AssertionError, "native turn allowance"):
                verification.check_usage(force=True)
            self.assertEqual(verification.manifest["observed_native_turns"], len(rows))

    def test_calls_are_read_through_code_mode(self):
        rows = [said("user", "add greet.py"),
                call("exec", 'text(await tools.mcp__mesimon__create_ticket({"title":"greet"}));'),
                call("mcp__mesimon__start_agent", '{"key":"T-2"}'),
                call("exec", 'text(await tools.exec_command({"cmd":"gh pr checks 42 --watch"}));'),
                call("exec", "await tools.sleep({\"seconds\": 30});"),
                call("exec", "tools.apply_patch(`*** Begin Patch`)"),
                call("spawn_agent", '{"task":"x"}'),
                call("exec", 'text(await tools.mcp__mesimon__sleep_agent({"key":"T-2"}));')]
        calls = rig.tool_calls(rows)
        self.assertEqual(len(calls), 7)
        self.assertEqual(rig.board_calls(calls), ["create_ticket", "start_agent", "sleep_agent"])
        self.assertEqual([c["row"] for c in rig.polling(calls)], [3, 4], "a board sleep_agent is no poll")
        self.assertEqual(len(rig.patched(calls)), 1)
        self.assertEqual(len(rig.spawned(calls)), 1)
        self.assertEqual([c["row"] for c in rig.tool_calls(rows, since=5)], [5, 6, 7])

    def test_user_turns_skip_injected_context_and_a_final_answer_ends_one(self):
        rows = [said("user", "<environment_context>cwd</environment_context>"),
                said("developer", "This session was started by mesimon"),
                said("user", "Add greet.py"), said("assistant", "working", phase="commentary"),
                said("user", "T-2 finished its turn")]
        self.assertEqual(rig.user_turns(rows), [(2, "Add greet.py"), (4, "T-2 finished its turn")])
        self.assertFalse(rig.answered_after(rows, 2))
        rows.append(said("assistant", "T-2 is done", phase="final_answer"))
        self.assertTrue(rig.answered_after(rows, 4))

    def test_the_rig_allows_the_board_s_tools_and_a_worker_s_git_alone(self):
        worktrees = Path("/owned/state/worktrees")
        mcp = {"what": "mcp_tool", "command": "mesimon", "reason": "Allow mesimon to run start_agent?"}
        self.assertEqual(rig.approval_decision(mcp, "crown", worktrees), (True, "start_agent"))
        git = {"what": "command", "command": "git commit -m greet", "cwd": "/owned/state/worktrees/T-2-greet"}
        self.assertEqual(rig.approval_decision(git, "worker", worktrees), (True, "git commit -m greet"))
        for request, role in ((git, "crown"), ({**git, "command": "git add . && curl x"}, "worker"),
                              ({**git, "cwd": "/owned/repo"}, "worker"),
                              ({"what": "command", "command": "gh pr checks 42"}, "crown"),
                              ({"what": "mcp_tool", "command": "other", "reason": "write"}, "crown")):
            with self.subTest(request=request, role=role):
                self.assertFalse(rig.approval_decision(request, role, worktrees)[0])
        self.assertTrue(rig.dialog_allows("Allow?\n› 1. Yes, allow once\n  2. No"))
        self.assertFalse(rig.dialog_allows("› 2. No, and tell Codex what to do"))
        self.assertFalse(rig.dialog_allows("› Ask Codex to do anything"), "the composer is no dialog")

    def test_the_crown_fixture_s_home_hides_the_person_s_tiers_and_claude(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory) / "home"
            shell, claude, path = rig.fixture_home(home)
            self.assertFalse((home / ".local/state/mesimon/tiers.toml").exists())
            self.assertIn("email = fixture@example.invalid", (home / ".gitconfig").read_text())
            refused = subprocess.run([str(claude), "--version"], capture_output=True, text=True, timeout=5)
            self.assertEqual(refused.returncode, 1)
            captured = subprocess.run([str(shell), "-l", "-i", "-c", "echo $PATH"], capture_output=True,
                                      text=True, timeout=5, env={"PATH": "/usr/bin:/bin"})
            self.assertEqual(captured.stdout.strip(), path)

    def test_the_stub_forge_answers_from_its_files_first_on_the_login_shell_path(self):
        claude_rig = rig.module("rig_claude_stub_test", "rig.py")
        with tempfile.TemporaryDirectory() as directory:
            stub = Path(directory) / "forge"
            gh = rig.forge_stub(stub, claude_rig.GH_STUB)
            shell, _, _ = rig.fixture_home(Path(directory) / "home", stub)
            body = rig.pull_requests("msmn/T-2-forge", "abc123", "failed")
            (stub / "prs.json").write_text(body)
            found = subprocess.run([str(shell), "-l", "-i", "-c", "command -v gh"], capture_output=True,
                                   text=True, timeout=5, env={"PATH": "/usr/bin:/bin"})
            self.assertEqual(found.stdout.strip(), str(gh))
            listed = subprocess.run([str(gh), "pr", "list"], capture_output=True, text=True, timeout=5, env={})
            self.assertEqual(json.loads(listed.stdout), json.loads(body))
            pr = json.loads(body)[0]
            self.assertEqual((pr["number"], pr["headRefName"], pr["headRefOid"]), (42, "msmn/T-2-forge", "abc123"))
            self.assertEqual(pr["statusCheckRollup"][0]["conclusion"], "FAILURE")
            self.assertEqual(json.loads(rig.pull_requests("b", "t", "running"))[0]["statusCheckRollup"][0]["status"],
                             "IN_PROGRESS")


class TrustTests(unittest.TestCase):
    def test_child_identity_needs_a_correlated_completed_spawn(self):
        state = rig.module("rig_child_items_test", "codex-state-e2e.py")
        started = {"type": "subAgentActivity", "kind": "started", "id": "activity",
                   "agentThreadId": "child", "agentPath": "/root/child"}
        returned = {"type": "functionCallOutput", "name": "spawn_agent",
                    "output": '{"task_name":"/root/child"}'}
        self.assertEqual(state.native_spawns([started])[0]["status"], "inProgress")
        self.assertEqual(state.native_spawns([started, returned])[0]["status"], "completed")
        other = {**returned, "output": '{"task_name":"/root/other"}'}
        self.assertEqual(state.native_spawns([started, other])[0]["status"], "inProgress")
        legacy = {"type": "collabAgentToolCall", "tool": "spawnAgent", "status": "completed",
                  "receiverThreadIds": ["child"]}
        self.assertEqual(state.native_spawns([legacy, started, returned]), [legacy])
        self.assertEqual(state.native_spawns([{**started, "kind": "completed"}]), [])
        records = [{"type": "response_item", "payload": {"type": "function_call", "name": "spawn_agent",
                    "call_id": "activity"}}, {"type": "response_item", "payload": {
                    "type": "function_call_output", "call_id": "activity", "output": returned["output"]}}]
        self.assertEqual(state.native_spawns([started], records)[0]["status"], "completed")
        self.assertEqual(state.native_spawns([{**started, "id": "other-call"}], records)[0]["status"], "inProgress")

    def test_lifecycle_hooks_use_their_own_allowlist_on_the_current_screen(self):
        lifecycle = rig.module("rig_lifecycle_trust_test", "codex-lifecycle-e2e.py")
        verification = object.__new__(lifecycle.Verification)
        verification.home = Path("/owned/home")
        verification.repo = Path("/owned/repo")
        verification.args = argparse.Namespace(case="stop", binary=Path("/owned/mesimon"))
        verification.client = Mock()
        verification.client.board.return_value = {"sessions": [{"id": "fixture", "kind": "codex",
            "transcript_path": "/owned/home/sessions/thread.jsonl"}]}
        verification.verify_native_home = Mock()
        verification.tm = Mock()
        verification.mark = Mock()
        verification.hook_review = {"fixture"}
        verification.hook_trusted = set()
        hooks = [{"enabled": True, "source": "user", "command": lifecycle.HOOK_COMMAND,
                  "eventName": event} for event in ("userPromptSubmit", "stop")]
        verification.native_read = Mock(return_value={"data": [{"hooks": hooks}]})
        screen = "Lifecycle hooks from config and enabled plugins.\nt trust all · enter review · esc close"
        with tempfile.TemporaryDirectory() as directory:
            verification.out = Path(directory)
            verification.startup_consent("fixture", screen)
            verification.tm.assert_called_once_with("send-keys", "-t", "fixture", "t")
            verification.hook_trusted.clear()
            verification.tm.reset_mock()
            hooks[0]["command"] = "unvetted"
            with self.assertRaises(AssertionError):
                verification.startup_consent("fixture", screen)
            verification.tm.assert_not_called()

    def test_current_hooks_screen_verifies_handlers_before_trusting_and_closes(self):
        state = rig.module("rig_hooks_test", "codex-state-e2e.py")
        verification = object.__new__(state.Verification)
        verification.home = Path("/owned/home")
        verification.repo = Path("/owned/repo")
        verification.client = Mock()
        verification.client.board.return_value = {"sessions": [{"id": "fixture", "kind": "codex",
            "transcript_path": "/owned/home/sessions/thread.jsonl"}]}
        verification.args = argparse.Namespace(case="complete", binary=Path("/owned/mesimon"))
        verification.tm = Mock()
        verification.mark = Mock()
        verification.hook_review = {"fixture"}
        verification.hook_trusted = set()
        verification.native_read = Mock(return_value={"data": [{"hooks": [{"enabled": True,
            "source": "sessionFlags", "command": "'/owned/mesimon' gate --provider codex --from-env",
            "eventName": "preToolUse"}]}]})
        screen = "Lifecycle hooks from config and enabled plugins.\nt trust all · enter review · esc close"
        verification.startup_consent("fixture", screen)
        verification.tm.assert_called_once_with("send-keys", "-t", "fixture", "t")
        verification.startup_consent("fixture", screen.replace("t trust all · enter review", "enter details"))
        self.assertEqual(verification.tm.call_args.args, ("send-keys", "-t", "fixture", "Escape"))
        verification.hook_trusted.clear()
        verification.tm.reset_mock()
        verification.native_read.return_value["data"][0]["hooks"][0]["command"] = "unknown handler"
        with self.assertRaises(state.Inconclusive):
            verification.startup_consent("fixture", screen)
        verification.tm.assert_not_called()

    def test_current_folder_prompt_is_accepted_only_for_the_owned_fixture(self):
        state = rig.module("rig_trust_test", "codex-state-e2e.py")
        verification = object.__new__(state.Verification)
        verification.repo = Path("/owned/repo")
        verification.client = Mock()
        verification.client.board.return_value = {"sessions": [{"id": "fixture", "kind": "codex"}]}
        verification.verify_native_home = Mock()
        verification.tm = Mock()
        verification.mark = Mock()
        verification.args = argparse.Namespace(case="complete")
        screen = "/owned/repo\nTrust this folder?\n› 1. Trust and continue"
        with patch.object(state.time, "sleep"):
            verification.startup_consent("fixture", screen)
        verification.verify_native_home.assert_called_once_with("fixture")
        verification.tm.assert_called_once_with("send-keys", "-t", "fixture", "Enter")
        verification.tm.reset_mock()
        with self.assertRaises(state.Inconclusive):
            verification.startup_consent("fixture", screen.replace("/owned/repo", "/unrelated/repo"))
        verification.tm.assert_not_called()


if __name__ == "__main__":
    unittest.main()
