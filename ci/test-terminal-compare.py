#!/usr/bin/env python3
"""PTY regression for the standalone replay tool; no daemon or tmux.

Run with python3 -B ci/test-run.py -- python3 -B ci/test-terminal-compare.py.
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest


class Replay(unittest.TestCase):
    def test_controls_resize_idle_and_terminal_restoration(self):
        with tempfile.TemporaryDirectory(prefix="mesimon-replay-") as folder:
            data = Path(folder) / "frames.json"
            data.write_text(json.dumps({"version": 1, "sets": [{
                "theme": "graphite", "endpoint": "apple", "width": 120, "height": 30,
                "frames": [
                    {"name": "first", "ms": 20, "full": "FIRST", "diff": "FIRST"},
                    {"name": "second", "ms": 20, "full": "SECOND_FULL", "diff": "SECOND_DIFF"},
                ],
            }]}))
            master, slave = pty.openpty()
            def resize(rows, columns):
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
            resize(32, 120)
            saved = termios.tcgetattr(slave)
            proc = subprocess.Popen(
                [sys.executable, "-B", str(Path(__file__).with_name("terminal-compare.py")),
                 str(data), "--endpoint", "apple"],
                stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
            )
            def until(needle):
                output = b""
                end = time.monotonic() + 5
                while time.monotonic() < end:
                    if select.select([master], [], [], 0.1)[0]:
                        output += os.read(master, 65536)
                        if needle in output:
                            return output
                    self.assertIsNone(proc.poll(), output)
                self.fail(f"timeout waiting for {needle!r}: {output!r}")
            try:
                until(b"q quit")
                self.assertFalse(select.select([master], [], [], 0.5)[0], "idle replay must be silent")
                os.write(master, b"n\r") # Terminal.app do script adds a terminator
                self.assertIn(b"SECOND_DIFF", until(b"q quit"))
                os.write(master, b"f")
                self.assertIn(b"SECOND_FULL", until(b"q quit"))
                os.write(master, b"r")
                self.assertIn(b"FIRST", until(b"q quit"))
                resize(20, 60)
                until(b"Resize to at least")
                resize(32, 120)
                self.assertIn(b"FIRST", until(b"q quit"))
                os.write(master, b"q")
                until(b"\x1b[?1049l")
                proc.wait(timeout=5)
                self.assertEqual(proc.returncode, 0)
                restored = termios.tcgetattr(slave)
                # macOS marks already queued input for retyping when canonical
                # mode returns. PENDIN is kernel bookkeeping, not raw mode.
                for attrs in (restored, saved):
                    attrs[3] &= ~getattr(termios, "PENDIN", 0)
                self.assertEqual(restored, saved, "raw mode restored")
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=5)
                os.close(master)
                os.close(slave)


if __name__ == "__main__":
    unittest.main()
