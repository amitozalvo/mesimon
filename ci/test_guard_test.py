"""Bounded regression tests for the test owner. All workloads sleep, never spin."""
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from test_guard import Owner, descendants, process_table, same_process


class OwnershipTests(unittest.TestCase):
    def test_descendants_are_exact_and_pid_reuse_is_not_ownership(self):
        table = {1: (0, "S", "old", "user-board"), 2: (1, "S", "now", "fixture"),
                 3: (2, "S", "now", "sleep"), 4: (1, "S", "now", "unrelated")}
        self.assertEqual(set(descendants(table, {2})), {2, 3})
        self.assertFalse(same_process(table[2], (1, "S", "later", "fixture")))
        self.assertFalse(same_process(table[2], (1, "Z", "now", "fixture")))

    def test_process_table_parser(self):
        with patch("test_guard.subprocess.run") as run:
            run.return_value.stdout = "  42  1 S Sat Sep 5 12:00:00 2026 /bin/sleep 60\n"
            self.assertEqual(process_table()[42], (1, "S", "Sat Sep 5 12:00:00 2026", "/bin/sleep 60"))

    def test_custom_audit_root_retains_owned_registry_and_rejects_other_parents(self):
        with tempfile.TemporaryDirectory(prefix="msmn-audit-root-") as directory:
            root = Path(directory)
            run = root / "msmn-test-run-fixture"
            run.mkdir(mode=0o700)
            with patch.dict(os.environ, MESIMON_TEST_AUDIT_ROOT=str(root)):
                owner = Owner("retained-audit", "/bin/false", run)
                registry = owner.registry
                self.assertEqual(registry.parent, run.resolve())
                self.assertTrue(owner.cleanup("finish"))
                self.assertEqual(json.loads(registry.read_text())["status"], "cleaned")
                other = root / "other"
                other.mkdir(mode=0o700)
                with patch("test_guard.tempfile.mkdtemp") as allocate:
                    with self.assertRaises(ValueError):
                        Owner("invalid-audit", "/bin/false", other)
                    allocate.assert_not_called()
                root.chmod(0o755)
                with self.assertRaises(ValueError):
                    Owner("shared-audit", "/bin/false", run)

    def test_registration_refuses_external_repo_and_existing_socket(self):
        owner = Owner("validation", "/bin/false")
        try:
            with self.assertRaises(ValueError):
                owner.register("/tmp", None, None, "/tmp/not-ours.sock")
            sock = owner.root / "t.sock"
            sock.touch()
            with self.assertRaises(ValueError):
                owner.register(owner.root, None, None, sock)
        finally:
            self.assertTrue(owner.cleanup("finish"))


class LifecycleTests(unittest.TestCase):
    def start(self, timeout=15, tmux="/bin/false"):
        env = dict(os.environ)
        # These tests deliberately lose runners and expire deadlines. Their
        # own assertions audit cleanup; do not label those expected outcomes
        # as failures of an enclosing cargo/CI run.
        env.pop("MESIMON_TEST_RUN", None)
        self.guard = subprocess.Popen(
            [sys.executable, "-u", str(Path(__file__).with_name("test_guard.py")),
             "--name", "guard-test", "--tmux", tmux, "--timeout", str(timeout)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env,
        )
        self.root = Path(json.loads(self.guard.stdout.readline())["ok"])
        self.addCleanup(self.finish)

    def request(self, **body):
        self.guard.stdin.write(json.dumps(body) + "\n")
        self.guard.stdin.flush()
        response = json.loads(self.guard.stdout.readline())
        self.assertNotIn("error", response)
        return response["ok"]

    def finish(self):
        if self.guard.poll() is None:
            try:
                self.guard.stdin.write('{"op":"finish"}\n')
                self.guard.stdin.flush()
            except BrokenPipeError:
                pass
        self.guard.stdin.close()
        self.guard.wait(timeout=15)
        self.guard.stdout.close()

    def test_finish_reaps_child_and_leaves_unrelated_process_alive(self):
        self.start()
        unrelated = subprocess.Popen(["sleep", "20"])
        try:
            pid = self.request(op="spawn", argv=["sleep", "20"], env=dict(os.environ))
            self.finish()
            self.assertEqual(self.guard.returncode, 0)
            self.assertIsNone(unrelated.poll())
            self.assertNotIn(pid, process_table())
            self.assertFalse(self.root.exists())
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=3)

    def test_eof_handles_panic_or_killed_runner_without_inherited_pipe(self):
        self.start()
        pid = self.request(op="spawn", argv=["sleep", "20"], env=dict(os.environ))
        self.guard.stdin.close()  # Same pipe EOF as runner panic/exit/SIGKILL.
        self.guard.wait(timeout=15)
        self.assertEqual(self.guard.returncode, 1)
        self.assertNotIn(pid, process_table())
        self.assertFalse(self.root.exists())

    def test_absolute_deadline_stops_unresponsive_child(self):
        self.start(timeout=1)
        pid = self.request(op="spawn", argv=[sys.executable, "-c",
                           "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(20)"],
                           env=dict(os.environ))
        self.guard.wait(timeout=10)
        self.assertEqual(self.guard.returncode, 1)
        self.assertNotIn(pid, process_table())
        self.assertFalse(self.root.exists())

    def test_partial_request_cannot_disable_the_deadline(self):
        self.start(timeout=1)
        self.guard.stdin.write('{"op":')
        self.guard.stdin.flush()
        self.guard.wait(timeout=10)
        self.assertEqual(self.guard.returncode, 1)
        self.assertFalse(self.root.exists())

    def test_failed_spawn_still_cleans_partial_startup(self):
        self.start()
        self.guard.stdin.write(json.dumps(dict(op="spawn", argv=[str(self.root / "missing")], env={})) + "\n")
        self.guard.stdin.flush()
        self.assertIn("error", json.loads(self.guard.stdout.readline()))
        self.finish()
        self.assertEqual(self.guard.returncode, 0)
        self.assertFalse(self.root.exists())

    def test_actual_runner_sigkill_and_ctrl_c(self):
        helper = '''
import json, subprocess, sys, time
g = subprocess.Popen([sys.executable, '-B', '-u', sys.argv[1], '--name', 'lost-runner',
                      '--tmux', '/bin/false', '--timeout', '15'],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
root = json.loads(g.stdout.readline())['ok']
g.stdin.write(json.dumps({'op': 'spawn', 'argv': ['sleep', '30'], 'env': {}}) + '\\n')
g.stdin.flush()
pid = json.loads(g.stdout.readline())['ok']
print(json.dumps([root, pid]), flush=True)
time.sleep(30)
'''
        env = dict(os.environ)
        env.pop("MESIMON_TEST_RUN", None)
        for sig in (signal.SIGKILL, signal.SIGINT):
            with self.subTest(signal=sig):
                runner = subprocess.Popen([sys.executable, "-B", "-c", helper,
                                           str(Path(__file__).with_name("test_guard.py"))],
                                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                          text=True, env=env)
                try:
                    root, pid = json.loads(runner.stdout.readline())
                    runner.send_signal(sig)
                    runner.wait(timeout=3)
                    deadline = time.monotonic() + 12
                    while Path(root).exists() and time.monotonic() < deadline:
                        time.sleep(0.05)
                    self.assertFalse(Path(root).exists())
                    self.assertNotIn(pid, process_table())
                finally:
                    if runner.poll() is None:
                        runner.kill()
                        runner.wait(timeout=3)
                    runner.stdout.close()

    def test_detached_tmux_is_reaped_on_runner_loss(self):
        tmux = os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux")
        if not tmux:
            if os.environ.get("MESIMON_REQUIRE_TMUX"):
                self.fail("tmux required")
            self.skipTest("tmux unavailable")
        self.start(tmux=tmux)
        sock = str(self.root / "t.sock")
        self.request(op="register", repo=str(self.root), sock=sock)
        launcher = self.request(op="spawn", argv=[tmux, "-S", sock, "-f", "/dev/null",
                                "new-session", "-d", "-s", "owned", "sleep 60"], env=dict(os.environ))
        deadline = time.monotonic() + 5
        while self.request(op="poll", pid=launcher) is None:
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.05)
        out = subprocess.run([tmux, "-S", sock, "display-message", "-p", "#{pid} #{pane_pid}"],
                             check=True, capture_output=True, text=True, timeout=3)
        pids = [int(p) for p in out.stdout.split()]
        self.guard.stdin.close()
        self.guard.wait(timeout=15)
        table = process_table()
        self.assertFalse(any(pid in table and "Z" not in table[pid][1] for pid in pids))
        self.assertFalse(self.root.exists())


if __name__ == "__main__":
    unittest.main()
