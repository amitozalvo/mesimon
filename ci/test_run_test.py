"""Disk pressure checks; never fill the real disk to exercise the failure path.
Concurrency (T-830): real bounded runs over throwaway Cargo packages, every
wait bounded, every workload a sleep."""
import contextlib
import fcntl
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("test_run", Path(__file__).with_name("test-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RoadPassTests(unittest.TestCase):
    def test_a_full_workspace_suite_is_followed_by_the_mod_road(self):
        self.assertEqual(runner.road_pass(["cargo", "nextest", "run", "--workspace"], None),
                         ["cargo", "nextest", "run", "-p", "mesimon", "-E", "kind(test)"])
        self.assertEqual(runner.road_pass(["cargo", "nextest", "run", "--workspace"], 4)[-2:],
                         ["--test-threads", "4"])
        self.assertEqual(runner.road_pass(["cargo", "test", "--workspace"], None),
                         ["cargo", "test", "-p", "mesimon", "--tests"])

    def test_no_fail_fast_reaches_the_mod_road(self):
        self.assertEqual(runner.road_pass(["cargo", "nextest", "run", "--workspace", "--no-fail-fast"], None),
                         ["cargo", "nextest", "run", "-p", "mesimon", "-E", "kind(test)", "--no-fail-fast"])
        self.assertEqual(runner.road_pass(["cargo", "test", "--workspace", "--no-fail-fast"], None),
                         ["cargo", "test", "-p", "mesimon", "--tests", "--no-fail-fast"])
        # Past `--` it is the test binary's word, not the suite's.
        self.assertEqual(runner.road_pass(["cargo", "test", "--workspace", "--", "--no-fail-fast"], None),
                         ["cargo", "test", "-p", "mesimon", "--tests"])

    def test_anything_narrower_runs_once(self):
        for command in (["cargo", "test", "-p", "mesimon", "--test", "hook_e2e"],
                        ["cargo", "clippy", "--workspace"], ["echo", "hello"]):
            self.assertIsNone(runner.road_pass(command, None), command)


class DiskBudgetTests(unittest.TestCase):
    def test_nonexistent_output_uses_existing_volume_and_checks_it_once(self):
        with tempfile.TemporaryDirectory() as root:
            with patch.object(runner.shutil, "disk_usage", return_value=SimpleNamespace(free=6 * 2**30)) as usage:
                runner.check_disk_space([Path(root) / "new/target", Path(root)], 5)
            usage.assert_called_once_with(Path(root).resolve())

    def test_low_space_reports_path_and_reserve(self):
        with patch.object(runner.shutil, "disk_usage", return_value=SimpleNamespace(free=2**30)):
            with self.assertRaisesRegex(RuntimeError, "1.0 GiB free.*requires 5 GiB"):
                runner.check_disk_space([Path.cwd()], 5)

    def test_reserve_boundary_is_allowed(self):
        with patch.object(runner.shutil, "disk_usage", return_value=SimpleNamespace(free=5 * 2**30)):
            runner.check_disk_space([Path.cwd()], 5)

    def test_cargo_config_and_separate_build_directory_are_watched(self):
        info = {"target_directory": "/configured/target", "build_directory": "/other/build"}
        env = dict(os.environ, CARGO_TARGET_DIR="/configured/target")
        with patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps(info))) as run:
            paths = runner.disk_paths(["cargo", "test", "--manifest-path", "app/Cargo.toml",
                                       "--config=build.incremental=false"], env)
        self.assertIn(Path("/configured/target"), paths)
        self.assertIn(Path("/other/build"), paths)
        self.assertIn("--config=build.incremental=false", run.call_args.args[0])
        self.assertEqual(run.call_args.kwargs["env"], env)

    def test_explicit_target_overrides_metadata_and_test_arguments_are_ignored(self):
        info = {"target_directory": "/configured/target"}
        with patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps(info))):
            for option in (["--target-dir", "/explicit"], ["--target-dir=/explicit"]):
                paths = runner.disk_paths(["cargo", "test", *option, "--", "--target-dir=/ignored"], os.environ)
                self.assertIn(Path("/explicit"), paths)
                self.assertNotIn(Path("/configured/target"), paths)
                self.assertNotIn(Path("/ignored"), paths)

    def test_refusal_starts_no_supervisor_or_workload(self):
        with patch.object(runner.sys, "argv", ["test-run.py", "--", "echo", "hello"]), \
             patch.object(runner, "build_output", return_value=[]), \
             patch.object(runner.shutil, "disk_usage", return_value=SimpleNamespace(free=0)), \
             patch.object(runner.subprocess, "Popen") as spawn, \
             contextlib.redirect_stderr(io.StringIO()) as errors:
            self.assertEqual(runner.main(), 2)
        spawn.assert_not_called()
        self.assertIn("Check not started: Low disk space", errors.getvalue())

    def test_space_loss_during_run_uses_supervisor_cleanup(self):
        # A fake supervisor exercises the runner's real polling/finally path.
        # The real process ownership implementation has its own bounded tests.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "child-0.log").touch()
            replies = "\n".join(json.dumps({"ok": value}) for value in [str(root), 123, None]) + "\n"
            control = io.StringIO()
            class Control(io.StringIO):
                def close(self):
                    if not self.closed:
                        control.write(self.getvalue())
                        super().close()
            guard = SimpleNamespace(stdin=Control(), stdout=io.StringIO(replies), wait=lambda timeout: 0)
            real_mkdtemp = tempfile.mkdtemp
            with patch.object(runner.sys, "argv", ["test-run.py", "--", "echo", "hello"]), \
                 patch.object(runner, "check_disk_space", side_effect=[None, None, RuntimeError("Low disk space")]), \
                 patch.object(runner.time, "monotonic", side_effect=[0, 0, 2, 3, 3]), \
                 patch.object(runner, "build_output", return_value=[]), \
                 patch.object(runner, "lock_dir", return_value=root), \
                 patch.object(runner.Claims, "take"), \
                 patch.object(runner.tempfile, "mkdtemp", side_effect=lambda **kw: real_mkdtemp(dir=root, prefix=kw["prefix"])), \
                 patch.object(runner.subprocess, "Popen", return_value=guard), \
                 patch.object(runner.subprocess, "run"), \
                 patch.object(runner, "stamp_pass") as stamp, \
                 contextlib.redirect_stdout(io.StringIO()), \
                 contextlib.redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(runner.main(), 1)
            self.assertIn('"op":"finish"', control.getvalue())
            self.assertIn("Low disk space", errors.getvalue())
            stamp.assert_not_called()

class PassTests(unittest.TestCase):
    def passes_run(self, command, codes):
        """The argv of every pass a fake supervisor was asked to spawn, and
        the run's result, when each pass exits with its code in turn."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index in range(len(codes)):
                (root / f"child-{index}.log").touch()
            values = [str(root)]
            for index, code in enumerate(codes):
                values += [100 + index, code]
            replies = "\n".join(json.dumps({"ok": value}) for value in values) + "\n"
            guard = SimpleNamespace(stdin=io.StringIO(), stdout=io.StringIO(replies), wait=lambda timeout: 0)
            spawned = []
            real_write = guard.stdin.write
            def write(text):
                for line in text.splitlines():
                    request = json.loads(line)
                    if request.get("op") == "spawn":
                        spawned.append(request["argv"])
                return real_write(text)
            guard.stdin.write = write
            real_mkdtemp = tempfile.mkdtemp
            with patch.object(runner.sys, "argv", ["test-run.py", "--min-free-gib", "0", "--", *command]), \
                 patch.object(runner, "build_output", return_value=[]), \
                 patch.object(runner, "lock_dir", return_value=root), \
                 patch.object(runner.Claims, "take"), \
                 patch.object(runner.tempfile, "mkdtemp", side_effect=lambda **kw: real_mkdtemp(dir=root, prefix=kw["prefix"])), \
                 patch.object(runner.subprocess, "Popen", return_value=guard), \
                 patch.object(runner.subprocess, "run"), \
                 patch.object(runner, "stamp_pass"), \
                 contextlib.redirect_stdout(io.StringIO()), \
                 contextlib.redirect_stderr(io.StringIO()):
                return runner.main(), spawned

    def test_a_red_suite_asked_past_its_first_red_still_runs_the_mod_road(self):
        result, spawned = self.passes_run(["cargo", "test", "--workspace", "--no-fail-fast"], [1, 0])
        self.assertEqual(result, 1)
        self.assertEqual(spawned, [["cargo", "test", "--workspace", "--no-fail-fast"],
                                   ["cargo", "test", "-p", "mesimon", "--tests", "--no-fail-fast"]])

    def test_a_red_suite_otherwise_ends_the_run(self):
        result, spawned = self.passes_run(["cargo", "test", "--workspace"], [1])
        self.assertEqual(result, 1)
        self.assertEqual(spawned, [["cargo", "test", "--workspace"]])

    def test_a_red_mod_road_fails_the_run(self):
        result, spawned = self.passes_run(["cargo", "test", "--workspace", "--no-fail-fast"], [0, 1])
        self.assertEqual(result, 1)
        self.assertEqual(len(spawned), 2)


HERE = Path(__file__).resolve().parent

# A run's workload, as steps: touch:<file>, await:<file>, absent:<file>,
# fixture:<name> (a nested fixture supervisor owning a sleep, its pid in
# <name>.pid, finished on <name>.end), exit:<code>. Files live in the box.
SCRIPT = """
import json, pathlib, subprocess, sys, time
box, guard = pathlib.Path(sys.argv[1]), sys.argv[2]
def wait(name):
    end = time.monotonic() + 40
    while not (box / name).exists():
        if time.monotonic() > end:
            sys.exit('never saw ' + name)
        time.sleep(0.05)
for step in sys.argv[3:]:
    verb, _, arg = step.partition(':')
    if verb == 'touch':
        (box / arg).touch()
    elif verb == 'await':
        wait(arg)
    elif verb == 'absent' and (box / arg).exists():
        sys.exit(arg + ' was already there')
    elif verb == 'fixture':
        g = subprocess.Popen([sys.executable, '-B', '-u', guard, '--name', 'overlap-' + arg,
                              '--tmux', '/bin/false', '--timeout', '60'],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        json.loads(g.stdout.readline())
        g.stdin.write(json.dumps({'op': 'spawn', 'argv': ['sleep', '60'], 'env': {}}) + '\\n')
        g.stdin.flush()
        (box / (arg + '.pid')).write_text(str(json.loads(g.stdout.readline())['ok']))
        wait(arg + '.end')
        g.stdin.write('{"op":"finish"}\\n')
        g.stdin.close()
        g.wait(timeout=15)
    elif verb == 'exit':
        sys.exit(int(arg))
"""


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    out = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True)
    return bool(out.stdout.strip()) and "Z" not in out.stdout


class Run:
    """One real `ci/test-run.py` over a throwaway package, read as it goes."""

    def __init__(self, test, cwd, *steps, env=None, flags=()):
        self.proc = subprocess.Popen(
            [sys.executable, "-B", str(HERE / "test-run.py"), "--min-free-gib", "0", "--timeout", "90",
             *flags, "--", sys.executable, "-B", "-c", SCRIPT, str(test.box), str(HERE / "test_guard.py"), *steps],
            cwd=cwd, env=dict(test.env, **(env or {})), stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True)
        test.addCleanup(self.kill)
        self.lines = []
        self.changed = threading.Condition()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        for line in self.proc.stdout:
            with self.changed:
                self.lines.append(line)
                self.changed.notify_all()

    def text(self):
        with self.changed:
            return "".join(self.lines)

    def said(self, words):
        return words in self.text()

    def wait_for(self, words, timeout=30):
        with self.changed:
            if not self.changed.wait_for(lambda: words in "".join(self.lines), timeout):
                raise AssertionError(f"never said {words!r}:\n{''.join(self.lines)}")

    def finish(self, timeout=60):
        code = self.proc.wait(timeout=timeout)
        self.reader.join(timeout=5)
        return code

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(timeout=10)
        self.proc.stdout.close()


@unittest.skipUnless(shutil.which("cargo"), "cargo is required to resolve build output")
class ConcurrencyTests(unittest.TestCase):
    """Two bounded runs at once (T-830): independent build output overlaps,
    shared build output takes turns and says whose turn it is."""

    def setUp(self):
        top = Path(tempfile.mkdtemp(prefix="msmn-overlap-")).resolve()
        self.addCleanup(shutil.rmtree, top, ignore_errors=True)
        self.top = top
        self.box = top / "box"
        self.box.mkdir()
        self.locks = top / "locks"
        self.locks.mkdir(mode=0o700)
        env = {k: v for k, v in os.environ.items() if k not in (
            "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR",
            "MESIMON_TEST_RUN", "MESIMON_TEST_HELD")}
        # A lock dir of the test's own: a real run never waits on these.
        self.env = dict(env, MESIMON_TEST_LOCK_DIR=str(self.locks))

    def package(self, name, config=None):
        root = self.top / name
        (root / "src").mkdir(parents=True)
        (root / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2021"\n')
        (root / "src/lib.rs").write_text("")
        if config:
            (root / ".cargo").mkdir()
            (root / ".cargo/config.toml").write_text(config)
        return root

    def test_independent_build_output_runs_overlap(self):
        a = Run(self, self.package("a"), "touch:a.started", "await:b.started")
        b = Run(self, self.package("b"), "touch:b.started", "await:a.started")
        self.assertEqual(a.finish(), 0, a.text())
        self.assertEqual(b.finish(), 0, b.text())
        for run in (a, b):
            self.assertNotIn("waiting", run.text())
            self.assertIn("Fixture audit clean (1 owners).", run.text())

    def test_shared_build_output_takes_turns_and_says_whose_turn(self):
        shared = self.top / "shared"
        for n, (how, (config, env)) in enumerate({
            "CARGO_TARGET_DIR": (None, {"CARGO_TARGET_DIR": str(shared)}),
            "build.target-dir": (f'[build]\ntarget-dir = "{shared}"\n', {}),
            "build.build-dir": (f'[build]\nbuild-dir = "{shared}"\n', {}),
        }.items()):
            with self.subTest(how):
                for f in self.box.iterdir():
                    f.unlink()
                a_dir, b_dir = self.package(f"a{n}", config), self.package(f"b{n}", config)
                a = Run(self, a_dir, "touch:a.started", "await:a.go", "touch:a.done", env=env)
                a.wait_for("== tests:")
                b = Run(self, b_dir, "absent:b.unexpected", "await:a.done", env=env)
                b.wait_for("== waiting (not testing yet): build output")
                self.assertIn(f"in use by pid {a.proc.pid} in {a_dir} (running tests since", b.text())
                self.assertNotIn("== tests:", b.text())
                (self.box / "a.go").touch()
                self.assertEqual(a.finish(), 0, a.text())
                self.assertEqual(b.finish(), 0, b.text())
                self.assertIn("starting", b.text())

    def test_cancel_or_fail_one_run_and_the_other_survives_with_accurate_audits(self):
        for mode in ("cancel", "fail"):
            with self.subTest(mode):
                for f in self.box.iterdir():
                    f.unlink()
                a_steps = ["fixture:a"] + (["exit:1"] if mode == "fail" else [])
                a = Run(self, self.package(f"a-{mode}"), *a_steps)
                b = Run(self, self.package(f"b-{mode}"), "fixture:b")
                deadline = time.monotonic() + 30
                while not all((self.box / f"{n}.pid").exists() for n in "ab"):
                    self.assertLess(time.monotonic(), deadline, a.text() + b.text())
                    time.sleep(0.05)
                a_sleep, b_sleep = (int((self.box / f"{n}.pid").read_text()) for n in "ab")
                if mode == "cancel":
                    a.proc.send_signal(signal.SIGINT)
                    self.assertEqual(a.finish(), 128 + signal.SIGINT, a.text())
                    # Its own two owners, cancelled: the command and its fixture.
                    self.assertIn("Fixture audit FAILED (2 incomplete/abnormal owners)", a.text())
                    registry = Path(a.text().split("Fixture audit FAILED (2 incomplete/abnormal owners): ")[1].split()[0])
                    owners = [json.loads(m.read_text()) for m in registry.glob("*.json")]
                    self.addCleanup(shutil.rmtree, registry, ignore_errors=True)
                    self.assertEqual(sorted(m["status"] for m in owners), ["runner-lost", "runner-lost"])
                    self.assertFalse(any("overlap-b" in m["root"] for m in owners))
                else:
                    (self.box / "a.end").touch()
                    self.assertEqual(a.finish(), 1, a.text())
                    self.assertIn("Fixture audit clean (2 owners). Registry kept", a.text())
                    registry = Path(a.text().split("Registry kept: ")[1].split()[0])
                    self.addCleanup(shutil.rmtree, registry, ignore_errors=True)
                self.assertFalse(alive(a_sleep))
                self.assertTrue(alive(b_sleep), "the other run's fixture was touched")
                self.assertIsNone(b.proc.poll(), b.text())
                (self.box / "b.end").touch()
                self.assertEqual(b.finish(), 0, b.text())
                self.assertIn("Fixture audit clean (2 owners).", b.text())
                self.assertFalse(alive(b_sleep))

    def test_pruning_holds_off_only_runs_that_share_its_build_output(self):
        busy = self.package("busy")
        claims = runner.Claims(self.locks, ["cargo", "test"], [])
        dirs = runner.build_output(["cargo", "test", "--manifest-path", str(busy / "Cargo.toml")], self.env)
        claims.take(dirs, False, time.monotonic() + 5)
        claims.phase("pruning")
        try:
            other = Run(self, self.package("other"), "exit:0")
            self.assertEqual(other.finish(), 0, other.text())
            self.assertNotIn("waiting", other.text())
            same = Run(self, busy, "exit:0")
            same.wait_for("(pruning the build cache since")
        finally:
            claims.release()
        self.assertEqual(same.finish(), 0, same.text())

    def test_exclusive_run_waits_for_every_run_and_holds_new_ones_off(self):
        a = Run(self, self.package("a"), "touch:a.started", "await:a.go")
        a.wait_for("== tests:")
        e = Run(self, self.package("e"), "touch:e.ran", "await:e.go", flags=["--exclusive"])
        e.wait_for(f"every other bounded run to finish: pid {a.proc.pid}")
        c = Run(self, self.package("c"), "touch:c.ran")
        c.wait_for("an exclusive run (the release gate)")
        (self.box / "a.go").touch()
        self.assertEqual(a.finish(), 0, a.text())
        e.wait_for("== tests:")
        time.sleep(1)
        self.assertFalse((self.box / "c.ran").exists(), c.text())
        (self.box / "e.go").touch()
        self.assertEqual(e.finish(), 0, e.text())
        self.assertEqual(c.finish(), 0, c.text())

    def test_a_run_inside_a_run_takes_no_lock_its_parent_holds(self):
        pkg = self.package("nested")
        inner = [sys.executable, "-B", str(HERE / "test-run.py"), "--min-free-gib", "0", "--timeout", "60",
                 "--", "true"]
        outer = subprocess.run([sys.executable, "-B", str(HERE / "test-run.py"), "--min-free-gib", "0",
                                "--timeout", "60", "--", *inner],
                               cwd=pkg, env=self.env, capture_output=True, text=True, timeout=60)
        self.assertEqual(outer.returncode, 0, outer.stdout + outer.stderr)
        self.assertNotIn("waiting", outer.stdout)


class PruneTests(unittest.TestCase):
    def test_prune_steps_aside_while_cargo_builds_and_prunes_after(self):
        prune = HERE / "prune-deps.py"
        with tempfile.TemporaryDirectory() as directory:
            debug = Path(directory) / "debug"
            (debug / "deps").mkdir(parents=True)
            orphan = debug / "deps/x-0.rcgu.o"
            orphan.write_bytes(b"")
            for name in (".cargo-lock", ".cargo-build-lock"):
                (debug / name).touch()
            # A build in flight holds its build lock exclusively.
            fd = os.open(debug / ".cargo-build-lock", os.O_RDWR)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX)
                out = subprocess.run([sys.executable, "-B", str(prune), "--target-dir", str(debug),
                                      "--wait", "1"], capture_output=True, text=True, timeout=30)
            finally:
                os.close(fd)
            self.assertIn("waiting up to 1 s for cargo's build lock", out.stdout)
            self.assertIn("not pruning this time", out.stderr)
            self.assertTrue(orphan.exists())
            out = subprocess.run([sys.executable, "-B", str(prune), "--target-dir", str(debug), "--wait", "1"],
                                 capture_output=True, text=True, timeout=30)
            self.assertFalse(orphan.exists(), out.stdout + out.stderr)
            # Nothing linked since: the next run skips the scan, and a new
            # object is pruned again.
            out = subprocess.run([sys.executable, "-B", str(prune), "--target-dir", str(debug)],
                                 capture_output=True, text=True, timeout=30)
            self.assertIn("unchanged since the last prune", out.stdout)
            time.sleep(0.01)
            orphan.write_bytes(b"")
            subprocess.run([sys.executable, "-B", str(prune), "--target-dir", str(debug), "--quiet"],
                           capture_output=True, text=True, timeout=30)
            self.assertFalse(orphan.exists())


if __name__ == "__main__":
    unittest.main()
