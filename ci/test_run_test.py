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
                         ["cargo", "nextest", "run", "--workspace", "-E", "package(mesimon) & kind(test)"])
        self.assertEqual(runner.road_pass(["cargo", "nextest", "run", "--workspace"], 4)[-2:],
                         ["--test-threads", "4"])
        self.assertEqual(runner.road_pass(["cargo", "test", "--workspace"], None),
                         ["cargo", "test", "-p", "mesimon", "--tests"])

    def test_no_fail_fast_reaches_the_mod_road(self):
        self.assertEqual(runner.road_pass(["cargo", "nextest", "run", "--workspace", "--no-fail-fast"], None),
                         ["cargo", "nextest", "run", "--workspace", "-E", "package(mesimon) & kind(test)", "--no-fail-fast"])
        self.assertEqual(runner.road_pass(["cargo", "test", "--workspace", "--no-fail-fast"], None),
                         ["cargo", "test", "-p", "mesimon", "--tests", "--no-fail-fast"])
        # Past `--` it is not a recognized harness option, so coverage is unknown.
        self.assertIsNone(runner.road_pass(["cargo", "test", "--workspace", "--", "--no-fail-fast"], None))

    def test_anything_narrower_runs_once(self):
        for command in (["cargo", "test", "-p", "mesimon", "--test", "hook_e2e"],
                        ["cargo", "clippy", "--workspace"], ["echo", "hello"]):
            self.assertIsNone(runner.road_pass(command, None), command)

    def test_workspace_filters_never_expand_into_a_second_suite_or_get_stamped(self):
        selections = (["--lib", "--bins"], ["--test", "crown_e2e"], ["--tests"],
                      ["-p", "mesimon-core"], ["--exclude=mesimon"], ["crown"],
                      ["-E", "test(crown)"], ["--filter-expr=test(crown)"],
                      ["--no-run"], ["--", "--list"], ["--", "--skip", "crown"],
                      ["--", "--ignored"], ["--partition", "count:1/2"],
                      ["--profile", "ci"], ["--future-selection-option"])
        for prefix in (["cargo", "test"], ["cargo", "nextest", "run"]):
            for selection in selections:
                command = [*prefix, "--workspace", *selection]
                with self.subTest(command=command), patch.object(runner.subprocess, "run") as run:
                    self.assertIsNone(runner.road_pass(command, None))
                    runner.stamp_pass(command, {}, ["hooks", "mod"])
                    run.assert_not_called()

    def test_output_and_concurrency_options_preserve_full_coverage(self):
        for suffix in ([], ["--locked", "--no-fail-fast"], ["--test-threads", "4"],
                       ["--jobs=2", "--color", "never"], ["--target-dir", "/tmp/build"],
                       ["--", "--nocapture", "--test-threads=4"]):
            with self.subTest(suffix=suffix):
                self.assertTrue(runner.full_workspace_suite(["cargo", "test", "--workspace", *suffix]))

    def test_custom_nextest_profile_cannot_certify_release_coverage(self):
        with patch.object(runner.subprocess, "run") as run:
            runner.stamp_pass(["cargo", "nextest", "run", "--workspace"],
                              {"NEXTEST_PROFILE": "ci"}, ["hooks", "mod"])
            run.assert_not_called()

    def test_an_unfiltered_clean_suite_records_the_new_coverage_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tmux = root / "tmux"
            tmux.write_bytes(b"fixture tmux identity")
            with patch.object(runner.subprocess, "run", side_effect=[SimpleNamespace(stdout="abc123\n"),
                                                                     SimpleNamespace(stdout="")]), \
                 patch.object(runner.shutil, "which", return_value=str(tmux)), \
                 contextlib.redirect_stdout(io.StringIO()):
                runner.stamp_pass(["cargo", "nextest", "run", "--workspace"], {},
                                  ["hooks", "mod"], target=root)
            stamp = json.loads((root / "suite-passed.json").read_text())
            self.assertEqual(stamp["coverage"], "full-workspace-v2")
            self.assertEqual(stamp["roads"], ["hooks", "mod"])
            self.assertEqual(stamp["sha"], "abc123")


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

class FakeGuard:
    """A supervisor that answers by request, not by position: passes run side
    by side (T-1101), so spawns and polls interleave. Each spawned pass exits
    with its code in turn; a code of None is a pass that never exits."""

    def __init__(self, root, codes):
        self.replies = [json.dumps({"ok": str(root)}) + "\n"]
        self.codes = list(codes)
        self.exit = {}
        self.spawned = []
        self.spawned_env = []
        self.ops = []
        self.stdin = self.stdout = self
        self.closed = False

    def write(self, text):
        for line in text.splitlines():
            request = json.loads(line)
            self.ops.append(request["op"])
            if request["op"] == "spawn":
                pid = 100 + len(self.spawned)
                self.exit[pid] = self.codes[len(self.spawned)]
                self.spawned.append(request["argv"])
                self.spawned_env.append(request["env"])
                self.replies.append(json.dumps({"ok": pid}) + "\n")
            elif request["op"] == "poll":
                self.replies.append(json.dumps({"ok": self.exit[request["pid"]]}) + "\n")

    def flush(self):
        pass

    def readline(self):
        return self.replies.pop(0) if self.replies else ""

    def close(self):
        self.closed = True

    def wait(self, timeout):
        return 0


class PassTests(unittest.TestCase):
    def passes_run(self, command, codes, flags=(), logs=()):
        """The argv of every pass a fake supervisor was asked to spawn, and
        the run's result, when each pass exits with its code in turn. `logs`
        is what each pass's child-N.log already holds."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index in range(len(codes)):
                (root / f"child-{index}.log").write_text(logs[index] if index < len(logs) else "")
            guard = self.guard = FakeGuard(root, codes)
            self.spawned_env = guard.spawned_env
            real_mkdtemp = tempfile.mkdtemp
            self.stdout = io.StringIO()
            with patch.object(runner.sys, "argv", ["test-run.py", "--min-free-gib", "0", *flags, "--", *command]), \
                 patch.object(runner, "build_output", return_value=[]), \
                 patch.object(runner, "lock_dir", return_value=root), \
                 patch.object(runner.Claims, "take"), \
                 patch.object(runner.tempfile, "mkdtemp", side_effect=lambda **kw: real_mkdtemp(dir=root, prefix=kw["prefix"])), \
                 patch.object(runner.subprocess, "Popen", return_value=guard) as popen, \
                 patch.object(runner.subprocess, "run"), \
                 patch.object(runner, "stamp_pass"), \
                 contextlib.redirect_stdout(self.stdout), \
                 contextlib.redirect_stderr(io.StringIO()):
                result = runner.main()
            self.guard_argv = popen.call_args.args[0]
            return result, guard.spawned

    def test_a_red_suite_asked_past_its_first_red_still_runs_the_mod_road(self):
        result, spawned = self.passes_run(["cargo", "test", "--workspace", "--no-fail-fast"], [1, 0],
                                          flags=["--both-roads"])
        self.assertEqual(result, 1)
        self.assertEqual(spawned, [["cargo", "test", "--workspace", "--no-fail-fast"],
                                   ["cargo", "test", "-p", "mesimon", "--tests", "--no-fail-fast"]])

    def test_a_red_suite_otherwise_ends_the_run_without_waiting_for_the_mod_road(self):
        # The mod pass never exits here: the run ends on the first red all
        # the same, and the supervisor's cleanup ends the pass with it.
        result, spawned = self.passes_run(["cargo", "test", "--workspace"], [1, None], flags=["--both-roads"])
        self.assertEqual(result, 1)
        self.assertEqual(spawned, [["cargo", "test", "--workspace"],
                                   ["cargo", "test", "-p", "mesimon", "--tests"]])
        self.assertEqual(self.guard.ops[-1], "finish")
        self.assertIn("a pass is red: the other pass is stopped with it", self.stdout.getvalue())

    def test_both_roads_start_together_and_the_mod_road_s_lines_carry_a_prefix(self):
        result, spawned = self.passes_run(["cargo", "nextest", "run", "--workspace"], [0, 0],
                                          flags=["--both-roads"],
                                          logs=["Starting 3339 tests\n", "Starting 419 tests\nhalf a"])
        self.assertEqual(result, 0)
        self.assertEqual(len(spawned), 2)
        # Both spawned before either is asked how it is doing.
        self.assertEqual(self.guard.ops[:3], ["spawn", "spawn", "poll"])
        self.assertEqual(self.spawned_env[1]["MESIMON_TEST_ROAD"], "mod")
        self.assertNotIn("MESIMON_TEST_ROAD", self.spawned_env[0])
        out = self.stdout.getvalue()
        self.assertIn("\nStarting 3339 tests\n", out)
        self.assertIn("\nmod | Starting 419 tests\n", out)
        self.assertIn("\nmod | half a\n", out)

    def test_the_deadline_covers_both_roads_unless_given(self):
        self.passes_run(["cargo", "nextest", "run", "--workspace"], [0])
        self.assertEqual(self.guard_argv[-2:], ["--timeout", "1200"])
        self.passes_run(["cargo", "nextest", "run", "--workspace"], [0, 0], flags=["--both-roads"])
        self.assertEqual(self.guard_argv[-2:], ["--timeout", "2400"])
        self.passes_run(["cargo", "nextest", "run", "--workspace"], [0, 0], flags=["--both-roads", "--timeout", "50"])
        self.assertEqual(self.guard_argv[-2:], ["--timeout", "50"])

    def test_a_red_mod_road_fails_the_run(self):
        result, spawned = self.passes_run(["cargo", "test", "--workspace", "--no-fail-fast"], [0, 1],
                                          flags=["--both-roads"])
        self.assertEqual(result, 1)
        self.assertEqual(len(spawned), 2)

    def test_a_filtered_workspace_runs_only_the_requested_command(self):
        command = ["cargo", "nextest", "run", "--workspace", "-E", "test(crown)"]
        result, spawned = self.passes_run(command, [0])
        self.assertEqual((result, spawned), (0, [command]))

    def test_an_unfiltered_development_suite_also_runs_once_by_default(self):
        command = ["cargo", "nextest", "run", "--workspace"]
        result, spawned = self.passes_run(command, [0])
        self.assertEqual((result, spawned), (0, [command]))

    def test_both_roads_rejects_a_partial_check_before_starting_anything(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as exited:
            self.passes_run(["cargo", "test", "--workspace", "--lib"], [], flags=["--both-roads"])
        self.assertEqual(exited.exception.code, 2)

    def test_an_explicit_mod_road_is_not_repeated(self):
        command = ["cargo", "nextest", "run", "--workspace"]
        with patch.dict(os.environ, MESIMON_TEST_ROAD="mod"):
            result, spawned = self.passes_run(command, [0])
        self.assertEqual((result, spawned), (0, [command]))

    def test_jobs_caps_nextest_even_for_an_explicit_command(self):
        command = ["cargo", "nextest", "run", "--workspace", "-E", "test(crown)"]
        result, spawned = self.passes_run(command, [0], flags=["--jobs", "2"])
        self.assertEqual((result, spawned), (0, [command]))
        self.assertEqual(self.spawned_env[0]["NEXTEST_TEST_THREADS"], "2")
        self.assertEqual(self.spawned_env[0]["CARGO_BUILD_JOBS"], "2")


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

    def test_superseded_executables_go_and_the_two_newest_links_of_a_target_stay(self):
        prune = HERE / "prune-deps.py"
        with tempfile.TemporaryDirectory() as directory:
            debug = Path(directory) / "debug"
            deps = debug / "deps"
            deps.mkdir(parents=True)
            links = []
            for age, digest in enumerate(("a" * 16, "b" * 16, "c" * 16, "d" * 16)):
                exe = deps / f"crown_e2e-{digest}"
                exe.write_bytes(b"")
                exe.chmod(0o755)
                os.utime(exe, (1_000_000 + age, 1_000_000 + age))
                exe.with_name(exe.name + ".d").write_text("")
                links.append(exe)
            # Not test executables: a library, a proc-macro and an unhashed file stay whatever their age.
            others = [deps / ("libserde-" + "e" * 16 + ".rlib"), deps / ("libmacro-" + "f" * 16 + ".dylib"), deps / "notes"]
            for other in others:
                other.write_bytes(b"")
                other.chmod(0o755)
            alone = deps / ("hook_e2e-" + "0" * 16)
            alone.write_bytes(b"")
            alone.chmod(0o755)
            dry = subprocess.run([sys.executable, "-B", str(prune), "--target-dir", str(debug), "--dry-run"],
                                 capture_output=True, text=True, timeout=30)
            self.assertIn("would prune 2 stale executables", dry.stdout)
            self.assertTrue(all(exe.exists() for exe in links))
            self.assertFalse((debug / ".msmn-prune-objects.json").exists())
            out = subprocess.run([sys.executable, "-B", str(prune), "--target-dir", str(debug)],
                                 capture_output=True, text=True, timeout=30)
            self.assertIn("pruned 2 stale executables", out.stdout, out.stderr)
            self.assertEqual([exe.exists() for exe in links], [False, False, True, True])
            self.assertEqual([exe.with_name(exe.name + ".d").exists() for exe in links], [False, False, True, True])
            self.assertTrue(all(other.exists() for other in others) and alone.exists())
            # Each executable left is read once: its object list is kept beside deps/.
            kept = json.loads((debug / ".msmn-prune-objects.json").read_text())
            self.assertIn(str(links[3].resolve()), kept)
            self.assertNotIn(str(links[0].resolve()), kept)


class RustcSlotTests(unittest.TestCase):
    """ci/rustc-slot.py with a shell standing in for rustc: `sh -c <script> --crate-name x`."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.log = Path(self.directory.name) / "log"
        self.env = {**os.environ, "MESIMON_RUSTC_SLOT_DIR": str(Path(self.directory.name) / "slots")}

    def compile(self, slots, seconds, name="x", **popen):
        # `exec`: the stand-in is the one process holding the slot, as rustc is.
        script = (f"echo start {name} >> {self.log}; exec sleep {seconds}" if name == "held"
                  else f"echo start {name} >> {self.log}; sleep {seconds}; echo end {name} >> {self.log}")
        return subprocess.Popen([sys.executable, "-B", str(HERE / "rustc-slot.py"), "sh", "-c", script, "--crate-name", name],
                                env={**self.env, "MESIMON_RUSTC_SLOTS": str(slots)}, **popen)

    def hold(self):
        held = self.compile(1, 3, "held")
        self.addCleanup(held.wait, 30)
        deadline = time.monotonic() + 10
        while not self.log.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        return held

    def test_a_compile_that_waited_its_limit_goes_ahead_without_a_slot(self):
        self.hold()
        self.env["MESIMON_RUSTC_SLOT_WAIT"] = "0.3"
        late = self.compile(1, 0, "late", stderr=subprocess.PIPE, text=True)
        _, said = late.communicate(timeout=2.5)
        self.assertEqual(late.returncode, 0)
        self.assertIn("every slot still held after 0.3 s", said)

    def test_a_slot_directory_open_to_others_or_a_symlinked_slot_is_not_used(self):
        slots = Path(self.env["MESIMON_RUSTC_SLOT_DIR"])
        slots.mkdir()
        slots.chmod(0o777)
        open_dir = self.compile(1, 0, "open", stderr=subprocess.PIPE, text=True)
        _, said = open_dir.communicate(timeout=30)
        self.assertEqual(open_dir.returncode, 0)
        self.assertIn("is not a directory of this user's alone", said)
        slots.chmod(0o700)
        target = Path(self.directory.name) / "elsewhere"
        (slots / "0").symlink_to(target)
        linked = self.compile(1, 0, "linked", stderr=subprocess.PIPE, text=True)
        _, said = linked.communicate(timeout=30)
        self.assertEqual(linked.returncode, 0)
        self.assertIn("compiling without one", said)
        self.assertFalse(target.exists())

    def test_one_slot_runs_compiles_one_after_another_and_two_slots_overlap_them(self):
        first, second = self.compile(1, 0.6, "a"), self.compile(1, 0.6, "b")
        self.assertEqual((first.wait(30), second.wait(30)), (0, 0))
        words = [line.split()[0] for line in self.log.read_text().splitlines()]
        self.assertEqual(words, ["start", "end", "start", "end"])
        self.log.unlink()
        first, second = self.compile(2, 0.6, "a"), self.compile(2, 0.6, "b")
        self.assertEqual((first.wait(30), second.wait(30)), (0, 0))
        self.assertEqual([line.split()[0] for line in self.log.read_text().splitlines()][:2], ["start", "start"])

    def test_a_killed_compile_gives_its_slot_back(self):
        held = self.compile(1, 60, "held")
        deadline = time.monotonic() + 10
        while not self.log.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        held.kill()
        held.wait(10)
        started = time.monotonic()
        self.assertEqual(self.compile(1, 0, "next").wait(30), 0)
        self.assertLess(time.monotonic() - started, 5)

    def test_a_probe_takes_no_slot_and_the_exit_code_is_the_compilers(self):
        held = self.compile(1, 2, "held")
        self.addCleanup(held.wait, 30)
        deadline = time.monotonic() + 10
        while not self.log.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        out = subprocess.run([sys.executable, "-B", str(HERE / "rustc-slot.py"), "sh", "-c", "echo probed; exit 3"],
                             env={**self.env, "MESIMON_RUSTC_SLOTS": "1"}, capture_output=True, text=True, timeout=1.5)
        self.assertEqual((out.returncode, out.stdout), (3, "probed\n"))


class WorktreeInitTests(unittest.TestCase):
    """.mesimon-worktree-init.sh over a throwaway checkout and state directory."""

    def run_init(self, root, worktree):
        checkout = root / "checkout"
        if not checkout.exists():
            (checkout / "ci").mkdir(parents=True)
            shutil.copy(HERE / "rustc-slot.py", checkout / "ci/rustc-slot.py")
            git = ["git", "-C", str(checkout), "-c", "user.name=t", "-c", "user.email=t@example.invalid"]
            subprocess.run([*git, "init", "-q", "-b", "main"], check=True)
            subprocess.run([*git, "commit", "-q", "--allow-empty", "-m", "first"], check=True)
        worktree.mkdir(parents=True, exist_ok=True)
        return subprocess.run(["sh", str(HERE.parent / ".mesimon-worktree-init.sh")], cwd=worktree, timeout=60,
                              env={**os.environ, "MESIMON_CHECKOUT": str(checkout), "MESIMON_WORKTREE": str(worktree)},
                              capture_output=True, text=True)

    def test_the_worktrees_shared_cargo_config_is_written_with_its_wrapper(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            shared = root / "state/mesimon/abc123/worktrees"
            out = self.run_init(root, shared / "T-1-first")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            config = (shared / ".cargo/config.toml").read_text()
            self.assertIn("incremental = true", config)
            self.assertIn("jobs = 4", config)
            self.assertIn(f"rustc-wrapper = '{shared}/.cargo/rustc-slot.py'", config)
            self.assertTrue(os.access(shared / ".cargo/rustc-slot.py", os.X_OK))

    def test_a_worktree_kept_elsewhere_gets_no_config_beside_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            out = self.run_init(root, root / "elsewhere/T-1-first")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertFalse((root / "elsewhere/.cargo").exists())

    def test_an_idle_cache_of_a_landed_worktree_is_reclaimed_and_a_fresh_one_kept(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            shared = root / "state/mesimon/abc123/worktrees"
            self.run_init(root, shared / "T-1-first")
            checkout = root / "checkout"
            caches = {}
            for name, age in (("T-2-idle", 7200), ("T-3-building", 0)):
                subprocess.run(["git", "-C", str(checkout), "worktree", "add", "-q", "-b", name, str(shared / name)], check=True)
                caches[name] = shared / name / "target/debug/incremental"
                (caches[name] / "core-1").mkdir(parents=True)
                for path in (caches[name] / "core-1", caches[name]):
                    os.utime(path, (time.time() - age, time.time() - age))
            out = self.run_init(root, shared / "T-4-new")
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertIn("reclaimed the idle incremental cache of T-2-idle", out.stdout)
            self.assertFalse(caches["T-2-idle"].exists())
            self.assertTrue(caches["T-3-building"].exists())


if __name__ == "__main__":
    unittest.main()
