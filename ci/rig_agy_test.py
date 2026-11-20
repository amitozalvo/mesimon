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


def rows(*calls, said=NONCE, brief=True, prompt=agy.TITLE):
    out = [{"type": "USER_INPUT", "content": f"<USER_REQUEST>\n{prompt}\n</USER_REQUEST>"}]
    if brief:
        out.append({"type": "SYSTEM_MESSAGE", "content": "<SYSTEM_MESSAGE>" + agy.BRIEF_MARK})
    for name, args in calls:
        out.append({"type": "PLANNER_RESPONSE", "tool_calls": [{"name": name, "args": args}]})
        out.append({"type": "GENERIC", "content": "Created At: …"})
    out.append({"type": "PLANNER_RESPONSE", "content": said})
    return out


SCHEMA = ("read_file", {"Path": str(HOME / "mcp" / "mesimon" / "get_ticket.json")})
GET_TICKET = ("mcp_mesimon_get_ticket", {})


def passed(points):
    return {name: ok for name, ok, _ in points}


class BriefVerdictTests(unittest.TestCase):
    def test_the_eager_call_passes(self):
        self.assertTrue(all(passed(agy.verdict(rows(GET_TICKET), NONCE)).values()))

    def test_the_old_lazy_call_does_not_prove_eager_loading(self):
        lazy = ("call_mcp_tool", {"ServerName": "mesimon", "ToolName": "get_ticket", "Arguments": {}})
        self.assertFalse(passed(agy.verdict(rows(SCHEMA, lazy), NONCE))["get_ticket first"])

    def test_a_prompt_containing_the_ticket_note_fails(self):
        fixture = rows(GET_TICKET, prompt=agy.TITLE + "\n\n" + NONCE)
        self.assertFalse(passed(agy.verdict(fixture, NONCE))["only the title was submitted"])

    def test_work_before_the_ticket_fails(self):
        ls = ("run_command", {"CommandLine": "ls"})
        self.assertFalse(passed(agy.verdict(rows(ls, GET_TICKET), NONCE))["get_ticket first"])

    def test_another_board_tool_first_fails(self):
        board = ("call_mcp_tool", {"ServerName": "mesimon", "ToolName": "list_board", "Arguments": {}})
        self.assertFalse(passed(agy.verdict(rows(board, GET_TICKET), NONCE))["get_ticket first"])

    def test_a_reply_without_the_nonce_or_a_missing_brief_fails(self):
        points = passed(agy.verdict(rows(GET_TICKET, said="done", brief=False), NONCE))
        self.assertFalse(points["reply carries the ticket's nonce"])
        self.assertFalse(points["brief in the conversation"])
        self.assertTrue(points["get_ticket first"])

    def test_no_tool_call_fails(self):
        self.assertFalse(passed(agy.verdict(rows(), NONCE))["get_ticket first"])


if __name__ == "__main__":
    unittest.main()
