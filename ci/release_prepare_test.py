"""Worktree release preflights and the post-merge handoff, in disposable repositories."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from zipfile import ZipFile

HERE = Path(__file__).resolve().parent


class WorktreeRelease(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name).resolve()
        self.main = root / "mesimon"
        self.tree = root / "ticket"
        self.relay = root / "mesimon-relay"
        self.env = {**os.environ, "CARGO_NET_OFFLINE": "true",
                    "MESIMON_RELAY_DIR": str(self.relay)}
        self.main.mkdir()
        self.command(self.main, "git", "init", "-qb", "main")
        ci = self.main / "ci"
        ci.mkdir()
        for name in ("release-prepare.sh", "release-lib.sh", "release-handoff.py"):
            shutil.copy(HERE / name, ci)
        (self.main / ".gitignore").write_text("/target/\n/vendor/\n")
        (self.main / "Cargo.toml").write_text(
            '[package]\nname = "core-fixture"\nversion = "0.1.0"\nedition = "2021"\n')
        (self.main / "src").mkdir()
        (self.main / "src/lib.rs").write_text("pub fn old_api() {}\n")
        (self.main / "CHANGELOG.md").write_text("## v0.1.0 — 2026-11-20\n")
        self.command(self.main, "cargo", "generate-lockfile")
        self.commit(self.main)
        self.command(self.main, "git", "worktree", "add", "-qb", "candidate", str(self.tree))
        self.tmux = self.main / "vendor/tmux/tmux"
        self.tmux.parent.mkdir(parents=True)
        self.tmux.write_text("fixture tmux")
        self.tmux.chmod(0o755)

    def command(self, cwd, *args, check=True):
        out = subprocess.run(args, cwd=cwd, env=self.env, text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        if check:
            self.assertEqual(out.returncode, 0, out.stdout)
        return out

    def commit(self, cwd):
        self.command(cwd, "git", "add", ".")
        self.command(cwd, "git", "-c", "user.name=fixture", "-c", "user.email=t@example.invalid",
                     "commit", "-qm", "candidate")

    def test_prebump_and_candidate_lint_stop_before_relay_or_gate(self):
        (self.tree / "src/lib.rs").write_text("pub fn lint(x: u32) -> u32 { x + 0 }\n")
        # A dirty pre-bump checkout and the committed candidate both catch the real lint.
        for args in (["--lint"], []):
            if not args:
                self.commit(self.tree)
            out = self.command(self.tree, "ci/release-prepare.sh", *args, check=False)
            self.assertNotEqual(out.returncode, 0)
            self.assertIn("identity_op", out.stdout)
            self.assertNotIn("compile relay tests", out.stdout)
            self.assertFalse((self.tree / "target/release-prepare/gate.log").exists())

    def test_relay_integration_test_compiles_against_worktree_before_gate(self):
        (self.tree / "src/lib.rs").write_text("pub fn new_api() {}\n")
        self.commit(self.tree)
        self.relay.mkdir()
        self.command(self.relay, "git", "init", "-q")
        (self.relay / "Cargo.toml").write_text(
            '[package]\nname = "mesimon-relay"\nversion = "0.1.0"\nedition = "2021"\n'
            '[dependencies]\ncore-fixture = { path = "../mesimon" }\n')
        (self.relay / "tests").mkdir()
        test = self.relay / "tests/board.rs"
        test.write_text("#[test] fn board() { core_fixture::old_api(); }\n")
        self.commit(self.relay)
        out = self.command(self.tree, "ci/release-prepare.sh", check=False)
        self.assertNotEqual(out.returncode, 0)
        self.assertIn("cannot find function `old_api`", out.stdout)
        self.assertFalse((self.tree / "target/release-prepare/gate.log").exists())
        self.assertFalse((self.tree / "target/suite-passed.json").exists())
        test.write_text("#[test] fn board() { core_fixture::new_api(); }\n")
        self.commit(self.relay)
        self.command(self.tree, "bash", "-ec",
                     '. ci/release-lib.sh; release_relay_compile "$MESIMON_RELAY_DIR"')
        self.assertEqual(self.command(self.relay, "git", "status", "--porcelain").stdout, "")

    def test_handoff_requires_tested_commit_both_roads_and_intact_binaries(self):
        (self.tree / "candidate").touch()
        self.commit(self.tree)
        sha = self.command(self.tree, "git", "rev-parse", "HEAD").stdout.strip()
        bundle = self.main / "target/release-prepared" / f"{sha}.zip"
        outputs = self.tree / "target"
        targets = ("aarch64-apple-darwin", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl")
        for target in targets:
            binary = outputs / target / "release/mesimon"
            binary.parent.mkdir(parents=True)
            binary.write_text(target)
        stamp = {"sha": sha, "tmux": str(self.tmux), "roads": ["hooks"],
                 "tmux_sha256": hashlib.sha256(self.tmux.read_bytes()).hexdigest(),
                 "coverage": "full-workspace-v2"}
        original = outputs / "suite-passed.json"
        original.write_text(json.dumps(stamp))
        save = ["python3", "-B", "ci/release-handoff.py", "save", str(bundle), sha, str(self.tmux)]
        self.assertNotEqual(self.command(self.tree, *save, check=False).returncode, 0)
        self.assertFalse(bundle.exists())
        stamp["roads"].append("mod")
        original.write_text(json.dumps(stamp))
        self.command(self.tree, *save)
        activate = ["ci/release-prepare.sh", "--handoff"]
        self.assertNotEqual(self.command(self.tree, *activate, check=False).returncode, 0)
        self.command(self.main, "git", "merge", "--ff-only", "candidate")
        self.command(self.tree, *activate)
        self.assertEqual((self.main / "target/suite-passed.json").read_bytes(), original.read_bytes())
        dest = outputs / "unpacked"
        self.command(self.main, "python3", "-B", "ci/release-handoff.py",
                     "unpack", str(bundle), sha, str(self.tmux), str(dest))
        for target in targets:
            binary = dest / target / "release/mesimon"
            self.assertEqual(binary.read_text(), target)
            self.assertTrue(os.access(binary, os.X_OK))
        self.tmux.write_text("replacement")
        self.assertNotEqual(self.command(self.tree, *activate, check=False).returncode, 0)
        self.tmux.write_text("fixture tmux")
        with ZipFile(bundle, "w") as broken:
            broken.writestr(original.name, original.read_bytes())
        self.assertNotEqual(self.command(self.tree, *activate, check=False).returncode, 0)
