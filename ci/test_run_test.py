"""Disk pressure checks; never fill the real disk to exercise the failure path."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("test_run", Path(__file__).with_name("test-run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


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
                 patch.object(runner, "check_disk_space", side_effect=[None, RuntimeError("Low disk space")]), \
                 patch.object(runner.time, "monotonic", side_effect=[0, 2, 3, 3]), \
                 patch.object(runner.fcntl, "flock"), \
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


if __name__ == "__main__":
    unittest.main()
