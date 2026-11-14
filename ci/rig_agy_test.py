"""Offline checks of the agy rig's `brief` verdict (T-1232), on transcript
rows in the shape agy 1.5.9 writes (`transcript_full.jsonl`)."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("agy_rig", Path(__file__).parent / "rig" / "agy.py")
agy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(agy)

HOME = Path("/home/p/.gemini/antigravity-cli")
NONCE = "PERIWINKLE-0000"


def rows(*calls, said=NONCE, brief=True):
    out = [{"type": "USER_INPUT", "content": "<USER_REQUEST>\nAnswer the question\n</USER_REQUEST>"}]
    if brief:
        out.append({"type": "SYSTEM_MESSAGE", "content": "<SYSTEM_MESSAGE>" + agy.BRIEF_MARK})
    for name, args in calls:
        out.append({"type": "PLANNER_RESPONSE", "tool_calls": [{"name": name, "args": args}]})
        out.append({"type": "GENERIC", "content": "Created At: …"})
    out.append({"type": "PLANNER_RESPONSE", "content": said})
    return out


SCHEMA = ("read_file", {"Path": str(HOME / "mcp" / "mesimon" / "get_ticket.json")})
GET_TICKET = ("call_mcp_tool", {"ServerName": "mesimon", "ToolName": "get_ticket", "Arguments": {}})


def passed(points):
    return {name: ok for name, ok, _ in points}


class BriefVerdictTests(unittest.TestCase):
    def test_the_measured_road_passes(self):
        self.assertTrue(all(passed(agy.verdict(rows(SCHEMA, GET_TICKET), NONCE, HOME)).values()))

    def test_work_before_the_ticket_fails(self):
        ls = ("run_command", {"CommandLine": "ls"})
        self.assertFalse(passed(agy.verdict(rows(ls, GET_TICKET), NONCE, HOME))["get_ticket first"])

    def test_another_board_tool_first_fails(self):
        board = ("call_mcp_tool", {"ServerName": "mesimon", "ToolName": "list_board", "Arguments": {}})
        self.assertFalse(passed(agy.verdict(rows(board, GET_TICKET), NONCE, HOME))["get_ticket first"])

    def test_a_reply_without_the_nonce_or_a_missing_brief_fails(self):
        points = passed(agy.verdict(rows(GET_TICKET, said="done", brief=False), NONCE, HOME))
        self.assertFalse(points["reply carries the ticket's nonce"])
        self.assertFalse(points["brief in the conversation"])
        self.assertTrue(points["get_ticket first"])

    def test_no_tool_call_fails(self):
        self.assertFalse(passed(agy.verdict(rows(), NONCE, HOME))["get_ticket first"])


if __name__ == "__main__":
    unittest.main()
