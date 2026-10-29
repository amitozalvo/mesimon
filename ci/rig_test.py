"""Offline checks of the Claude rig's readiness predicates (A3, T-945),
ST2's acceptance (T-974) and the trace fixture's cut (T-1014)."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import time
import types
import unittest

SPEC = importlib.util.spec_from_file_location("claude_rig", Path(__file__).parent / "rig.py")
rig = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rig)

RULE = "─" * 40


class ComposerTests(unittest.TestCase):
    def test_an_empty_composer_between_its_rules_is_ready(self):
        lines = ["❯ say hello", "⏺ hello", "", RULE, "❯ ", RULE, "  ? for shortcuts"]
        self.assertTrue(rig.composer_empty(lines))

    def test_a_placeholder_is_empty_only_with_the_cursor_at_the_input_start(self):
        lines = ["", RULE, "❯ Try \"fix lint errors\"", RULE]
        self.assertTrue(rig.composer_empty(lines, (2, 2)))
        self.assertFalse(rig.composer_empty(lines, (21, 2)), "text the person typed")

    def test_an_old_prompt_in_the_scrollback_is_no_composer(self):
        lines = ["╭───╮", "❯ the last prompt", "⏺ the answer", "", "  resuming…"]
        self.assertFalse(rig.composer_empty(lines))

    def test_a_dialog_s_selected_row_is_no_composer(self):
        lines = [RULE, " Ready to code?", "", " ❯ 1. Yes, and auto-accept edits", "   2. No", RULE]
        self.assertFalse(rig.composer_empty(lines))

    def test_a_half_painted_box_is_no_composer(self):
        lines = ["", RULE, "❯ "]
        self.assertFalse(rig.composer_empty(lines), "no rule below yet")

    def test_a_box_holding_wrapped_words_is_not_empty(self):
        lines = [RULE, "❯ ", "  words wrapped under the glyph", RULE]
        self.assertFalse(rig.composer_empty(lines))


class SuiteTests(unittest.TestCase):
    def test_the_suite_names_each_test_once(self):
        tests = rig.load_tests(Path(__file__).parent / "rig" / "tests.toml")
        self.assertIn("A3", [t["id"] for t in tests])

    def test_a_second_test_under_one_id_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "tests.toml")
            with open(path, "w") as f:
                f.write('[[test]]\nid = "A2"\n\n[[test]]\nid = "B1"\n\n[[test]]\nid = "A2"\n')
            with self.assertRaisesRegex(ValueError, "two tests are named A2"):
                rig.load_tests(path)

    def test_incoming_board_ask_does_not_teach_the_crown_how_to_reply(self):
        test = next(t for t in rig.load_tests(Path(__file__).parent / "rig" / "tests.toml")
                    if t["id"] == "X2")
        subject = object.__new__(rig.Rig)
        self.assertTrue(test.get("blind"))
        brief = subject.crown_brief([{**test, "key": "T-2"}])
        self.assertNotIn("ask_board", brief)
        self.assertNotIn(test["expect"], subject.test_card(test))
        self.assertNotIn("ask_board", subject.test_card(test))


class BoardExchangeTests(unittest.TestCase):
    def fixture(self, kind="ask"):
        nonce = "fresh-peer-reference"
        sent = "Cut 0.1.0-beta.2 and report what /version serves." if kind == "ask" else f"Version unknown; {nonce}"
        peer_text = f"Release live; {nonce}" if kind == "ask" else f"Which release? Reference {nonce}"
        incoming = (f"The crown of the rig-peer board replied: {peer_text}" if kind == "ask" else
                    f'The crown of the rig-peer board (T-9 "Relay release") asks: {peer_text}')
        delivered = (f'The crown of the local board (T-4 "Crown") asks: {sent} ∙ ask_board board=local replies to it'
                     if kind == "ask" else f"The crown of the local board replied: {sent}")
        receipt = dict(board="rig-peer", crown="T-9", road="queued", reply=kind == "reply")
        use = {"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "ask1", "name": "mcp__mesimon__ask_board",
             "input": {"board": "rig-peer", "text": sent}}]}}
        result = {"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "ask1", "content": json.dumps(receipt)}]}}
        prompt = {"type": "user", "message": {"content": incoming}}
        reply = {"type": "assistant", "message": {"content": f"X1 replied: {peer_text}"}}
        rows = [use, result, prompt, reply] if kind == "ask" else [prompt, use, result, reply]
        evidence = dict(path="/fixture/rig-peer", crown="T-9", local_board="local", local_key="T-4",
                        nonce=nonce, incoming=incoming, delivered=delivered,
                        receipt=dict(board="local", crown="T-4", road="queued", reply=kind == "ask"))
        return rows, evidence, use, result

    def judge(self, rows, evidence, kind="ask"):
        return rig.board_exchange(rows, kind, evidence)[0]

    def test_both_directions_need_the_actual_delivery_and_correlated_receipts(self):
        for kind in ("ask", "reply"):
            with self.subTest(kind=kind):
                rows, evidence, _, _ = self.fixture(kind)
                self.assertTrue(self.judge(rows, evidence, kind))

    def test_a_queued_receipt_without_delivery_fails(self):
        rows, evidence, _, _ = self.fixture()
        evidence["delivered"] = ""
        self.assertFalse(self.judge(rows, evidence))

    def test_the_first_receipt_may_be_followed_by_the_crowns_rules(self):
        rows, evidence, _, result = self.fixture()
        block = result["message"]["content"][0]
        receipt = block["content"]
        block["content"] = "\n" + receipt + "\nThe board's rules for this session's crown:\nRead {state}."
        self.assertTrue(self.judge(rows, evidence))
        # A later valid-looking object cannot rescue a malformed receipt.
        block["content"] = "refused\n" + receipt
        self.assertFalse(self.judge(rows, evidence))

    def test_stale_or_unattributed_or_duplicate_incoming_words_fail(self):
        for bad in ("old reference", "Release live; fresh-peer-reference", "duplicate"):
            with self.subTest(bad=bad):
                rows, evidence, _, _ = self.fixture()
                if bad == "duplicate":
                    rows.insert(3, copy.deepcopy(rows[2]))
                else:
                    rows[2]["message"]["content"] = bad
                self.assertFalse(self.judge(rows, evidence))

    def test_a_reply_must_reach_the_person_with_the_fresh_reference(self):
        rows, evidence, _, _ = self.fixture()
        rows[-1]["message"]["content"] = "X1 replied: live"
        self.assertFalse(self.judge(rows, evidence))

    def test_incoming_request_must_be_answered_through_the_board_with_its_reference(self):
        rows, evidence, use, _ = self.fixture("reply")
        use["message"]["content"][0]["input"]["text"] = "Version unknown"
        evidence["delivered"] = "The crown of the local board replied: Version unknown"
        self.assertFalse(self.judge(rows, evidence, "reply"))
        self.assertFalse(self.judge([rows[0], rows[-1]], evidence, "reply"), "pane-only reply")

    def test_ask_and_reply_templates_are_not_interchangeable(self):
        for kind in ("ask", "reply"):
            rows, evidence, _, _ = self.fixture(kind)
            evidence["delivered"] = evidence["delivered"].replace("replied:", "asks:") if kind == "reply" \
                else evidence["delivered"].replace("local board (T-4", "other board (T-4")
            self.assertFalse(self.judge(rows, evidence, kind))

    def test_refused_missing_malformed_wrong_crown_or_wrong_reply_receipts_fail(self):
        rows, evidence, _, result = self.fixture()
        block = result["message"]["content"][0]
        receipt = json.loads(block["content"])
        for key, value in (("board", "elsewhere"), ("road", "held_for_person"),
                           ("crown", "T-10"), ("reply", True)):
            with self.subTest(key=key):
                block["content"] = json.dumps({**receipt, key: value})
                self.assertFalse(self.judge(rows, evidence))
        block["content"] = "not JSON"
        self.assertFalse(self.judge(rows, evidence))
        block["content"] = json.dumps(receipt)
        block["is_error"] = True
        self.assertFalse(self.judge(rows, evidence))
        self.assertFalse(self.judge([rows[0], *rows[2:]], evidence))

    def test_the_peer_must_recognize_the_reply_too(self):
        for key, value in (("reply", False), ("board", "other"), ("crown", "T-10"),
                           ("road", "held_for_person")):
            with self.subTest(key=key):
                rows, evidence, _, _ = self.fixture()
                evidence["receipt"][key] = value
                self.assertFalse(self.judge(rows, evidence))

    def test_a_retry_or_reply_loop_fails_but_duplicate_transcript_blocks_do_not(self):
        rows, evidence, use, result = self.fixture()
        self.assertTrue(self.judge([*rows, use, result], evidence))
        extra = copy.deepcopy(use)
        extra["message"]["content"][0]["id"] = "ask2"
        self.assertFalse(self.judge([*rows, extra], evidence))

    def test_the_wrong_destination_fails(self):
        rows, evidence, use, _ = self.fixture()
        args = use["message"]["content"][0]["input"]
        args["board"] = evidence["path"]
        self.assertTrue(self.judge(rows, evidence), "an unambiguous path is valid too")
        args["board"] = "other-board"
        self.assertFalse(self.judge(rows, evidence))

    def test_starting_the_peers_work_locally_fails(self):
        rows, evidence, use, _ = self.fixture()
        extra = copy.deepcopy(use)
        extra["message"]["content"][0].update(id="local", name="mcp__mesimon__start_agent")
        self.assertFalse(self.judge([*rows, extra], evidence))

    def test_closed_means_the_actual_not_open_refusal(self):
        rows, evidence, _, result = self.fixture()
        block = result["message"]["content"][0]
        for error, text, ok in ((True, "rig-peer is not open", True),
                                (False, "rig-peer is not open", False),
                                (True, "rig-peer has no crown", False)):
            block.update(is_error=error, content=text)
            self.assertEqual(self.judge(rows, evidence, "closed"), ok)


def wake(key):
    return {"type": "user", "message": {"content": f'{key} "x" has run its turn 31 s, past the 30 s it expected'}}


def assistant(*blocks):
    return {"type": "assistant", "message": {"content": list(blocks)}}


def call(key, cid="s1"):
    return {"type": "tool_use", "id": cid, "name": "mcp__mesimon__sample_agent", "input": {"key": key}}


def result(key, cid="s1", error=False):
    body = "a refusal" if error else json.dumps({"key": key, "state": "working", "captured_at_ms": 1})
    return {"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": cid, "is_error": error, "content": body}]}}


def verdict():
    return {"type": "text", "text": "SA1 worker: looping"}


class SampledBeforeTests(unittest.TestCase):
    """T-973, round 2: the verdict counts only after the sample's own
    successful result, read in order."""

    def judge(self, rows):
        return rig.sampled_before(rows, "T-1", "SA1 worker")[0]

    def test_a_call_its_result_then_the_verdict_passes(self):
        self.assertTrue(self.judge([wake("T-1"), assistant(call("T-1")), result("T-1"), assistant(verdict())]))

    def test_a_verdict_before_the_call_in_one_message_fails(self):
        rows = [wake("T-1"), assistant(verdict(), call("T-1")), result("T-1")]
        self.assertFalse(self.judge(rows))

    def test_a_verdict_before_the_result_fails(self):
        rows = [wake("T-1"), assistant(call("T-1")), assistant(verdict()), result("T-1")]
        self.assertFalse(self.judge(rows))

    def test_a_refused_sample_then_the_verdict_fails(self):
        rows = [wake("T-1"), assistant(call("T-1")), result("T-1", error=True), assistant(verdict())]
        self.assertFalse(self.judge(rows))

    def test_another_tickets_sample_counts_for_nothing(self):
        rows = [wake("T-1"), assistant(call("T-2")), result("T-2"), assistant(verdict())]
        self.assertFalse(self.judge(rows))


REQ = "toolu_plan1"
WHOLE = "1. a step of the design ┌──┐\n" * 40 + "\nThe code word is quince.\n"


def plan_call(cid, name, plan=False, request=None):
    args = {"key": "T-1"}
    if plan:
        args["plan"] = True
    if request:
        args["request"] = request
    return {"type": "tool_use", "id": cid, "name": f"mcp__mesimon__{name}", "input": args}


def plan_result(cid, body, error=False):
    return {"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": cid,
                                                     "is_error": error, "content": body}]}}


def head_body(request=REQ):
    head = WHOLE.encode()[:512].decode("utf-8", "ignore") + "…"
    return json.dumps({"key": "T-1", "needs_you": {
        "reason": "plan", "request": request, "plan": head, "plan_bytes": len(WHOLE.encode())}})


def whole_body(request=REQ):
    return json.dumps({"key": "T-1", "needs_you": {"reason": "plan", "request": request, "plan": WHOLE}})


ACCEPTED = json.dumps({"key": "T-1", "outcome": "accepted"})


class BudgetReceiptTests(unittest.TestCase):
    def sequence(self):
        def use(cid, name, key):
            return assistant({"type": "tool_use", "id": cid,
                              "name": f"mcp__mesimon__{name}", "input": {"key": key}})
        receipt = json.dumps({"status": "waiting_for_worktree", "budget_left": 0})
        return [use("s1", "start_agent", "T-1"), plan_result("s1", receipt),
                use("p1", "sleep_agent", "T-1"), plan_result("p1", "ok"),
                use("s2", "start_agent", "T-2"), plan_result("s2", receipt)]

    def judge(self, rows):
        return rig.budget_reused(rows, "T-1", "T-2")[0]

    def test_zero_receipt_then_park_then_start_passes(self):
        self.assertTrue(self.judge(self.sequence()))

    def test_a_park_before_the_zero_receipt_fails(self):
        rows = self.sequence()
        rows[1], rows[2] = rows[2], rows[1]
        self.assertFalse(self.judge(rows))

    def test_a_start_before_the_park_receipt_fails(self):
        rows = self.sequence()
        rows[3], rows[4] = rows[4], rows[3]
        self.assertFalse(self.judge(rows))

    def test_missing_or_failed_results_fail(self):
        for index in (1, 3, 5):
            with self.subTest(index=index):
                rows = self.sequence()
                self.assertFalse(self.judge(rows[:index] + rows[index + 1:]))
                rows[index]["message"]["content"][0]["is_error"] = True
                self.assertFalse(self.judge(rows))

    def test_a_hand_even_after_success_fails(self):
        rows = self.sequence() + [assistant(plan_call("h1", "raise_hand"))]
        self.assertFalse(self.judge(rows))

    def test_a_refused_start_does_not_replace_the_park_but_is_reported(self):
        rows = self.sequence()
        rows[2:2] = [assistant(plan_call("s0", "start_agent")),
                     plan_result("s0", "budget spent", error=True)]
        ok, seen = rig.budget_reused(rows, "T-1", "T-2")
        self.assertTrue(ok)
        self.assertIn("start_agent T-1 refused", seen)

    def test_room_left_or_wrong_holder_fails(self):
        rows = self.sequence()
        rows[1] = plan_result("s1", json.dumps({"status": "started", "budget_left": 1}))
        self.assertFalse(self.judge(rows))
        rows = self.sequence()
        rows[2]["message"]["content"][0]["input"]["key"] = "T-3"
        self.assertFalse(self.judge(rows))


class PlanReadTests(unittest.TestCase):
    """T-991, rounds 2 and 3: D7's outcome, read in order: this request's
    whole plan, then its accept, each on its own good result; a head read is
    the crown's choice, and checked where it made one."""

    def judge(self, rows):
        return rig.plan_read_order(rows, "T-1", "quince")[0]

    def sequential(self, accept=REQ, accept_body=ACCEPTED, error=False):
        return [assistant(plan_call("g1", "get_ticket")), plan_result("g1", head_body()),
                assistant(plan_call("g2", "get_ticket", plan=True)), plan_result("g2", whole_body()),
                assistant(plan_call("a1", "accept_plan", request=accept)),
                plan_result("a1", accept_body, error)]

    def test_head_then_whole_then_accept_passes(self):
        self.assertTrue(self.judge(self.sequential()))

    def test_an_accept_batched_with_the_whole_read_fails(self):
        rows = [assistant(plan_call("g1", "get_ticket")), plan_result("g1", head_body()),
                assistant(plan_call("g2", "get_ticket", plan=True),
                          plan_call("a1", "accept_plan", request=REQ)),
                plan_result("g2", whole_body()), plan_result("a1", ACCEPTED)]
        self.assertFalse(self.judge(rows))

    def test_an_accept_before_the_whole_result_fails(self):
        rows = [assistant(plan_call("g1", "get_ticket")), plan_result("g1", head_body()),
                assistant(plan_call("g2", "get_ticket", plan=True)),
                assistant(plan_call("a1", "accept_plan", request=REQ)),
                plan_result("g2", whole_body()), plan_result("a1", ACCEPTED)]
        self.assertFalse(self.judge(rows))

    def test_a_refused_accept_fails(self):
        self.assertFalse(self.judge(self.sequential(accept_body="plan changed; read get_ticket again",
                                                    error=True)))

    def test_an_accept_for_another_request_fails(self):
        self.assertFalse(self.judge(self.sequential(accept="toolu_other")))

    def test_a_whole_read_of_another_request_fails(self):
        rows = self.sequential()
        rows[3] = plan_result("g2", whole_body(request="toolu_other"))
        self.assertFalse(self.judge(rows))

    def test_a_plain_read_that_carried_the_whole_fails(self):
        rows = [assistant(plan_call("g1", "get_ticket")), plan_result("g1", whole_body())] \
            + self.sequential()[2:]
        self.assertFalse(self.judge(rows), "a read without plan carried the whole")

    def test_the_whole_read_first_then_the_accept_passes(self):
        # The round-2 run's order: no head read, the whole, the accept.
        self.assertTrue(self.judge(self.sequential()[2:]))

    def test_a_head_whose_size_is_not_the_whole_s_fails(self):
        rows = self.sequential()
        body = json.loads(head_body())
        body["needs_you"]["plan_bytes"] += 1
        rows[1] = plan_result("g1", json.dumps(body))
        self.assertFalse(self.judge(rows))

    def test_the_request_blob_copies_count_for_nothing(self):
        blob = {"type": "api-request-blob", "message": {"content": [
            plan_call("a0", "accept_plan", request=REQ)]}}
        rows = self.sequential()
        rows.insert(3, blob)
        rows.insert(4, {"type": "api-request-blob", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "a0", "content": ACCEPTED}]}})
        self.assertTrue(self.judge(rows))
        only_blob = self.sequential()[:4] + rows[3:5]
        self.assertFalse(self.judge(only_blob), "a blob's accept, after the whole, is no accept")


class DoubledRowTests(unittest.TestCase):
    """T-1034: Claude Code 2.1.310 writes an MCP tool_use row twice under one
    id, and its tool_result twice; every reader counts each id once."""

    @staticmethod
    def rows():
        def use(i, key):
            return {"type": "assistant", "message": {"id": "m" + i, "content": [
                {"type": "tool_use", "id": i, "name": "mcp__mesimon__ask_agent",
                 "input": {"key": key}}]}}

        def res(i, text, err=False):
            return {"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": i, "is_error": err, "content": text}]}}
        bash = {"type": "assistant", "message": {"id": "mb", "content": [
            {"type": "tool_use", "id": "b1", "name": "Bash", "input": {}}]}}
        ask1 = json.dumps({"asked": {"ask": "A1"}})
        refusal = rig.OUTSTANDING_ASK_MARK + ": A1"
        return [use("u1", "T-1"), use("u1", "T-1"), res("u1", ask1), res("u1", ask1),
                bash, use("u2", "T-1"), use("u2", "T-1"), res("u2", refusal, True), res("u2", refusal, True)]

    def test_calls_and_inputs_count_each_id_once(self):
        rows = self.rows()
        self.assertEqual([n.rsplit("__", 1)[-1] for n, _ in rig.tool_calls(rows)],
                         ["ask_agent", "Bash", "ask_agent"])
        self.assertEqual(len(rig.tool_inputs(rows)), 3)
        self.assertEqual(len(rig.tool_uses(rows)), 3)

    def test_measures_count_each_id_once(self):
        m = rig.crown_measures(rows=self.rows(), since_ms=0)
        self.assertEqual((m["calls"], m["failed"], len(m["receipt_bytes"])), (3, 1, 1))

    def test_the_first_two_asks_are_two_asks(self):
        self.assertTrue(rig.second_ask_refusal(self.rows(), "T-1")[0])


class RestartEnvironmentTests(unittest.TestCase):
    def test_a_losing_attempt_s_capture_is_not_the_winner_s(self):
        with tempfile.TemporaryDirectory() as d:
            env_file = os.path.join(d, "shellenv.env")
            board = object.__new__(rig.Rig)
            board.paths = types.SimpleNamespace(rt_dir=d)
            board.seams = {}
            attempts = []

            def start_daemon(extra_env=None):
                attempts.append(1)
                if len(attempts) == 1:
                    # The first attempt's daemon (another client's respawn,
                    # without the seam) captures the environment, then loses.
                    with open(env_file, "w") as f:
                        f.write("PATH=/first\n")
                return {"daemon_pid": len(attempts)}

            board.stop_daemon = lambda: None
            board.start_daemon = start_daemon
            board.env_of = lambda pid: "MESIMON_X=1" if pid == 2 else ""
            self.assertTrue(board.restart({"MESIMON_X": "1"}))
            self.assertEqual(len(attempts), 2, "the second attempt won")
            landed = lambda: rig.file_generation(env_file) not in (None, board.env_before)  # noqa: E731
            self.assertFalse(landed(), "the loser's file, written after the restart began")
            time.sleep(0.01)
            tmp = env_file + ".new"
            with open(tmp, "w") as f:
                f.write("PATH=/winner\n")
            os.replace(tmp, env_file)
            self.assertTrue(landed(), "the winner's own capture")


class SteerVerdictTests(unittest.TestCase):
    """T-974 round 2: ST2's acceptance, `held_steer`, off a recorded feed."""

    PAYLOAD = "Reply with the single word mango and end your turn. Read no file and run no command."
    TID = "01TICKET"

    def lines(self, second="immediately", second_ask="B", wake="A taken"):
        sent = lambda at, outcome, ask, beside=None: dict(
            kind="board", cmd="ask_agent_sent", ticket=self.TID, at_ms=at, outcome=outcome,
            ask=ask, **({"beside": beside} if beside else {}))
        out = [sent(1_000, "now", "A"), sent(2_000, second, second_ask, ["A sent"])]
        if wake:
            out.append(dict(kind="crown_wake", worker=self.TID, cause="answered", at_ms=9_000,
                            ask=wake))
        return out

    def verdict(self, lines, prompts):
        return rig.steer_verdict("held_steer", self.PAYLOAD, lines, self.TID, prompts)

    def prompts(self, n=2):
        return ["ST2 · the title", *[self.PAYLOAD] * n, "first"]

    def test_the_measured_run_passes_with_no_wake_naming_the_steer(self):
        ok, seen = self.verdict(self.lines(), self.prompts())
        self.assertTrue(ok, seen)
        self.assertIn("2 of 4", seen)

    def test_a_missing_second_prompt_fails(self):
        ok, seen = self.verdict(self.lines(), self.prompts(1))
        self.assertFalse(ok, seen)

    def test_a_second_now_fails(self):
        ok, seen = self.verdict(self.lines(second="now"), self.prompts())
        self.assertFalse(ok, seen)

    def test_the_same_ask_twice_fails(self):
        ok, seen = self.verdict(self.lines(second_ask="A"), self.prompts())
        self.assertFalse(ok, seen)

    def test_an_incidental_word_is_not_the_payload(self):
        prompts = ["ST2 · the title", "mango", "a mango, please", "first"]
        ok, seen = self.verdict(self.lines(), prompts)
        self.assertFalse(ok, seen)

    def test_no_wake_or_a_wake_short_of_taken_fails(self):
        self.assertFalse(self.verdict(self.lines(wake=None), self.prompts())[0])
        self.assertFalse(self.verdict(self.lines(wake="A handed"), self.prompts())[0])


class FixtureTests(unittest.TestCase):
    """A recorded trace becomes a fixture in a public repository (T-1014)."""

    SID = "0123abcd-0000-4000-8000-00000000beef"

    def raw(self):
        home = os.environ["HOME"]
        cwd = f"{home}/state/worktrees/T-1-x"
        transcript = f"{home}/.claude/projects/{cwd.replace('/', '-')}/{self.SID}.jsonl"
        frame = {"at_ms": 1000, "kind": "hook", "session": self.SID, "event": "SessionStart",
                 "reason": "startup", "payload": {"cwd": cwd, "transcript_path": transcript,
                                                  "scratchpad_dir": f"/tmp/c{home.replace('/', '-')}"}}
        blob = {"at_ms": 1200, "kind": "transcript", "session": self.SID,
                "line": {"type": "api-request-blob", "hash": "h",
                         "message": {"content": "the person's private memory"}}}
        note = {"at_ms": 1200, "kind": "transcript", "session": self.SID,
                "line": {"type": "attachment", "uuid": "u", "timestamp": "2026-10-26T00:00:00.000Z",
                         "attachment": {"type": "instructions", "content": "private rules"}}}
        said = {"at_ms": 1500, "kind": "transcript", "session": self.SID,
                "line": {"type": "assistant", "cwd": cwd, "message": {"id": "m", "content": [
                    {"type": "thinking", "thinking": "private", "signature": "EqRkZXNpZw"},
                    {"type": "tool_use", "id": "toolu_1", "name": "Read",
                     "input": {"file_path": f"{cwd}/a.txt"}}]}}}
        return [{"trace": "X1", "claude": "2.1.310"}, frame, blob, note, said], home

    def test_paths_sessions_and_private_records_leave_the_fixture(self):
        raw, home = self.raw()
        out = rig.fixture_of(raw, "/repo", "/state")
        text = json.dumps(out)
        self.assertEqual(out[0]["trace"], "X1", "the header first")
        for gone in (home, home.replace("/", "-"), self.SID, "private", "EqRkZXNpZw"):
            self.assertNotIn(gone, text)
        frame, blob, note, said = out[1:]
        self.assertEqual((frame["t"], said["t"]), (0, 500), "times from the first line")
        self.assertEqual(frame["session"], "{session}")
        self.assertEqual(frame["payload"]["transcript_path"], "{transcript}")
        self.assertEqual(frame["payload"]["cwd"], "{cwd}")
        self.assertEqual(blob["line"], {"type": "api-request-blob"}, "a blob keeps its kind alone")
        self.assertEqual(note["line"]["attachment"], {"type": "instructions"})
        self.assertEqual(note["line"]["timestamp"], "2026-10-26T00:00:00.000Z")
        blocks = said["line"]["message"]["content"]
        self.assertEqual(blocks[0], {"type": "thinking", "thinking": "", "signature": ""})
        self.assertEqual(blocks[1]["input"]["file_path"], "{cwd}/a.txt")
        self.assertEqual(said["line"]["message"]["id"], "m", "a message id stays: siblings")



class CompactionWindowTests(unittest.TestCase):
    """T-1069's verdicts: the compaction frames, words held for an open
    window, and the crown's rules owed again after its own compaction."""

    @staticmethod
    def frame(event, at, reason=None):
        line = {"kind": "hook", "event": event, "at_ms": at, "road": "mod"}
        if reason:
            line["reason"] = reason
        return line

    @staticmethod
    def iso(ms):
        import datetime
        return datetime.datetime.fromtimestamp(ms / 1000, datetime.timezone.utc).isoformat().replace("+00:00", "Z")

    def window(self, start=False):
        lines = [self.frame("SessionStart", 500, "startup"), self.frame("PreCompact", 1_000)]
        if start:
            lines.append(self.frame("SessionStart", 9_000, "compact"))
        return lines + [self.frame("PostCompact", 10_000), self.frame("Stop", 14_000)]

    def test_the_frames_are_the_compactions_own_in_order(self):
        self.assertEqual(rig.compaction_frames(self.window()), ["PreCompact", "PostCompact"])
        self.assertEqual(rig.compaction_frames(self.window(start=True)),
                         ["PreCompact", "SessionStart", "PostCompact"])

    def test_words_queued_inside_the_window_and_entered_after_it_are_held(self):
        ok, seen = rig.held_by_compaction(self.window(), 4_000, [("after", self.iso(10_200))], "after")
        self.assertTrue(ok, seen)

    def test_words_that_entered_inside_the_window_were_not_held(self):
        ok, seen = rig.held_by_compaction(self.window(), 4_000, [("after", self.iso(5_000))], "after")
        self.assertFalse(ok, seen)

    def test_words_queued_outside_the_window_prove_nothing(self):
        for queued in (900, 10_000, 12_000, None):
            ok, seen = rig.held_by_compaction(self.window(), queued, [("after", self.iso(12_500))], "after")
            self.assertFalse(ok, f"{queued}: {seen}")

    def test_words_taken_twice_or_a_window_never_closed_fail(self):
        twice = [("after", self.iso(10_200)), ("after", self.iso(11_000))]
        self.assertFalse(rig.held_by_compaction(self.window(), 4_000, twice, "after")[0])
        open_only = [self.frame("PreCompact", 1_000)]
        self.assertFalse(rig.held_by_compaction(open_only, 4_000, [("after", self.iso(10_200))], "after")[0])

    def crown_rows(self, before_rules, after_rules, boundary=True):
        def call(i, ms, rules):
            text = (rig.CROWN_RULES_MARK + " …") if rules else "receipt"
            return [
                {"type": "assistant", "uuid": f"a{i}", "timestamp": self.iso(ms), "message": {"content": [
                    {"type": "tool_use", "id": f"t{i}", "name": "mcp__mesimon__get_ticket", "input": {}}]}},
                {"type": "user", "uuid": f"u{i}", "timestamp": self.iso(ms + 10), "message": {"content": [
                    {"type": "tool_result", "tool_use_id": f"t{i}", "content": text}]}},
            ]
        rows = call(1, 1_000, True) + call(2, 2_000, before_rules)
        if boundary:
            rows.append({"type": "system", "subtype": "compact_boundary", "uuid": "b",
                         "timestamp": self.iso(3_000), "compactMetadata": {"trigger": "manual"}})
        return rows + call(3, 4_000, after_rules)

    def test_the_rules_ride_again_after_the_crowns_compaction(self):
        ok, seen = rig.rules_after_compaction(self.crown_rows(False, True), 0)
        self.assertTrue(ok, seen)

    def test_no_rules_after_the_compaction_fails(self):
        self.assertFalse(rig.rules_after_compaction(self.crown_rows(False, False), 0)[0])

    def test_rules_on_every_result_show_no_refresh(self):
        self.assertFalse(rig.rules_after_compaction(self.crown_rows(True, True), 0)[0])

    def test_an_untimestamped_request_blob_does_not_hide_the_call_it_copies(self):
        # Claude Code 2.1.312 writes each call first into an `api-request-blob`
        # row with no timestamp, the same tool_use and tool_result ids.
        rows = self.crown_rows(False, True)
        blob = [{"type": "api-request-blob", "message": copy.deepcopy(r["message"])} for r in rows[-2:]]
        ok, seen = rig.rules_after_compaction(rows[:-2] + blob + rows[-2:], 0)
        self.assertTrue(ok, seen)

    def test_no_board_result_before_the_boundary_shows_no_baseline_and_fails(self):
        rows = self.crown_rows(False, True)
        at = next(i for i, r in enumerate(rows) if r.get("subtype") == "compact_boundary")
        ok, seen = rig.rules_after_compaction(rows[at:], 0)
        self.assertFalse(ok, seen)
        self.assertIn("before the boundary: none", seen)

    def test_no_boundary_fails(self):
        self.assertFalse(rig.rules_after_compaction(self.crown_rows(False, True, boundary=False), 0)[0])

class CreateStartTests(unittest.TestCase):
    """T-1072: SF1's acceptance, off the crown's own calls and results."""

    @staticmethod
    def use(i, name, args):
        return {"message": {"content": [
            {"type": "tool_use", "id": i, "name": "mcp__mesimon__" + name, "input": args}]}}

    @staticmethod
    def said(i, body):
        return {"message": {"content": [
            {"type": "tool_result", "tool_use_id": i, "content": [{"type": "text", "text": body}]}]}}

    def filed(self):
        # The first call after a crowning carries the rules after its receipt.
        return [self.use("c", "create_ticket", {"title": "SF1 child"}),
                self.said("c", json.dumps({"key": "T-9", "seen": "receipt:a"}, indent=2)
                          + "\n\nThe board's rules for this session's crown: ...")]

    def start(self, seen="receipt:a", body=None):
        return [self.use("s", "start_agent", {"key": "T-9", "seen": seen}),
                self.said("s", body or json.dumps({"key": "T-9", "status": "started"}))]

    def test_a_start_on_the_create_s_stamp_with_no_read_passes(self):
        ok, observed = rig.create_start_outcome(self.filed() + self.start())
        self.assertTrue(ok, observed)

    def test_a_read_a_stale_refusal_another_stamp_or_no_start_fails(self):
        read = [self.use("g", "get_ticket", {"key": "T-9"}), self.said("g", "{}")]
        for rows in (
            self.filed() + read + self.start(),
            self.filed() + self.start(body="T-9 changed since it was read (now in TODO)"),
            self.filed() + self.start(seen="0123abcd"),
            self.filed(),
            [self.use("c", "create_ticket", {}), self.said("c", json.dumps({"key": "T-9"}))]
            + self.start(),
        ):
            ok, observed = rig.create_start_outcome(rows)
            self.assertFalse(ok, observed)


if __name__ == "__main__":
    unittest.main()
