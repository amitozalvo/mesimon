"""Offline checks of the Claude rig's readiness predicates (A3, T-945),
ST2's acceptance (T-974) and the trace fixture's cut (T-1014)."""
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


if __name__ == "__main__":
    unittest.main()
