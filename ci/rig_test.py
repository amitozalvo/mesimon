"""Offline checks of the Claude rig's readiness predicates (A3, T-945)."""
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
