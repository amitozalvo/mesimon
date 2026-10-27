"""Exercise the Claude rig's actual Peer fixture without calling a model.

Build mesimon first, then: python3 -B ci/test-run.py -- python3 -B ci/rig_peer_test.py
Two shell stubs, an isolated HOME, and registered fixture resources; no login.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


rig = module("peer_fixture_rig", ROOT / "ci/rig.py")
Guard = module("peer_fixture_guard", ROOT / "ci/codex-runtime-probe.py").Guard


class PeerFixtureTests(unittest.TestCase):
    def wait_for(self, predicate):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.05)
        self.fail("timed out waiting for the stub's state or actual input")

    def test_peer_receives_an_ask_and_delivers_its_reply(self):
        binary = str(ROOT / "target/debug/mesimon")
        self.assertTrue(Path(binary).is_file(), "cargo build -p mesimon first")
        tmux = str(ROOT / "target/debug/mesimon-tmux")
        if not Path(tmux).exists():
            tmux = shutil.which("tmux")
        self.assertTrue(tmux, "tmux is required")
        guard = Guard(90, tmux)
        peer = wire = None
        try:
            home = guard.root / "home"
            home.mkdir()
            with patch.dict(os.environ, {"HOME": str(home)}):
                repo = guard.root / "local-board"
                repo.mkdir()
                rig.git(str(repo), "init", "-q")
                (repo / "README.md").write_text("fixture\n")
                rig.git(str(repo), "add", "README.md")
                self.assertIsNotNone(rig.git(str(repo), "-c", "user.name=rig", "-c",
                                            "user.email=rig@example.test", "commit", "-qm", "fixture"))
                out = repo / "target/rig"
                out.mkdir(parents=True)
                stub = repo / "claude-stub.sh"
                stub.write_text(rig.PEER_STUB)
                stub.chmod(0o700)
                paths = rig.board_paths(str(repo))
                guard.request(op="register", repo=str(repo), state=paths.state_dir,
                              runtime=paths.rt_dir, sock=paths.tmux_sock)
                env = rig.terminal_env({"MESIMON_DETACHED": "1", "MESIMON_CLAUDE_BIN": str(stub),
                                        "MESIMON_CLAUDE_ROAD": "hooks", "MESIMON_TMUX_BIN": tmux})
                guard.request(op="spawn", argv=[binary, "daemon", "--repo", str(repo)], env=env)
                wire = rig.Wire(paths.orch_sock, connect_for=15)
                ticket = wire.create_ticket("TODO", "Local crown")
                wire.set_workspace(ticket, "shared_checkout")
                session = wire.spawn(ticket, "claude", submit_prompt=False)["id"]
                # The local stand-in, like boardlink_e2e's one_turn, emits a
                # turn so that the real daemon can deliver the peer's reply.
                for event, body, state in (("UserPromptSubmit", {"prompt": "ready"}, "working"),
                                           ("Stop", {"stop_hook_active": False}, "idle")):
                    subprocess.run([binary, "hook", "--sock", str(Path(paths.rt_dir) / "hook.sock"),
                                    "--session", session, "--event", event],
                                   input=json.dumps(body), text=True, check=True, timeout=10)
                    self.wait_for(lambda: any(s["id"] == session and rig.word_of(s["state"]) == state
                                             for s in wire.board()["sessions"]))
                wire.crown(ticket)
                peer_repo = out / rig.PEER_NAME
                peer_repo.mkdir()
                pp = rig.board_paths(str(peer_repo))
                guard.request(op="register", repo=str(peer_repo), state=pp.state_dir,
                              runtime=pp.rt_dir, sock=pp.tmux_sock)
                fixture = types.SimpleNamespace(repo=str(repo), out=str(out), bin=binary, wire=wire,
                                                seams={"MESIMON_TMUX_BIN": tmux})
                peer = rig.Peer(fixture)
                words = "release 0.1.0-beta.2; /version reference " + rig.ulid()
                receipt = wire.ask_board(session, rig.PEER_NAME, words)
                self.assertEqual((receipt["road"], receipt["reply"]), ("queued", False))
                self.wait_for(lambda: words in peer.received())
                self.assertIn('The crown of the local-board board (T-1 "Local crown") asks:', peer.received())
                self.assertIn("ask_board board=local-board replies", peer.received())
                reply = "answer " + rig.ulid()
                receipt = peer.reply("local-board", reply)
                self.assertEqual((receipt["road"], receipt["reply"]), ("queued", True))
                got = repo / "got.txt"
                self.wait_for(lambda: got.exists() and reply in got.read_text())
                self.assertIn("The crown of the rig-peer board replied: " + reply, got.read_text())
        finally:
            try:
                if peer:
                    peer.stop()
                if wire:
                    wire.close()
            finally:
                try:
                    self.assertTrue(guard.close(), "fixture cleanup failed")
                finally:
                    guard.process.stdout.close()


if __name__ == "__main__":
    unittest.main()
