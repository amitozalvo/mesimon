#!/usr/bin/env python3
"""Offline installer welcome checks; installs only a fake binary into a temp dir."""
import errno
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import tarfile
import tempfile
import termios
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]


class InstallerWelcome(unittest.TestCase):
    def install(self, *, tty=True, locale="en_US.UTF-8", term="xterm-256color", broken=False,
                listing=None, version="v0.0.0-test"):
        with tempfile.TemporaryDirectory(prefix="msmn-installer-") as tmp:
            root = Path(tmp)
            commands = root / "commands"
            commands.mkdir()

            def command(name, body):
                path = commands / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o755)

            command("uname", 'case "$1" in -s) echo Linux ;; -m) echo x86_64 ;; esac')
            command("git", "exit 0")
            command("claude", "exit 0")
            command("tmux", "exit 0")
            command("curl", '''while [ "$#" -gt 0 ]; do
  case "$1" in -o) out="$2"; shift 2 ;; *) url="$1"; shift ;; esac
done
case "$url" in *api.github.com*) cat "$FIXTURE_LISTING"; exit 0 ;; esac
case "$url" in *.sha256) cp "$FIXTURE_ASSET.sha256" "$out" ;; *) cp "$FIXTURE_ASSET" "$out" ;; esac''')
            name = f"mesimon-{version}-x86_64-unknown-linux-musl"
            archive = root / (name + ".tar.gz")
            # An older mesimon listed `welcome` in --help; it is recorded if run.
            binary = (b"#!/bin/sh\n"
                      b"case \"$1\" in --help) echo '  mesimon welcome   the intro' ;;\n"
                      b"  welcome) echo ran >> \"$WELCOME_LOG\" ;; esac\n"
                      b"echo mesimon-test\n" + (b"exit 1\n" if broken else b""))
            with tarfile.open(archive, "w:gz") as tar:
                entry = tarfile.TarInfo(name + "/mesimon")
                entry.size, entry.mode = len(binary), 0o755
                tar.addfile(entry, io.BytesIO(binary))
            Path(str(archive) + ".sha256").write_text(
                hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n"
            )
            env = dict(os.environ, PATH=str(commands) + ":/usr/bin:/bin", LC_ALL=locale,
                       TERM=term, PREFIX=str(root / "install with spaces"), FIXTURE_ASSET=str(archive),
                       WELCOME_LOG=str(root / "welcome.log"))
            args = ["sh", str(ROOT / "install.sh"), "--version", version]
            if listing is not None:
                (root / "listing.json").write_text(listing)
                env["FIXTURE_LISTING"] = str(root / "listing.json")
                args = args[:2]
            if tty:
                master, slave = pty.openpty()
                try:
                    # The pty is the installer's terminal, so its `/dev/tty` is this one.
                    def own_terminal():
                        os.setsid()
                        fcntl.ioctl(1, termios.TIOCSCTTY, 0)

                    process = subprocess.Popen(args, env=env, stdin=subprocess.DEVNULL,
                                               stdout=slave, stderr=slave,
                                               preexec_fn=own_terminal)
                    deadline = time.monotonic() + 15
                    os.set_blocking(master, False)
                    data = bytearray()
                    while True:
                        if time.monotonic() > deadline:
                            process.kill()
                            process.wait()
                            self.fail("installer timed out: " + data.decode(errors="replace"))
                        select.select([master], [], [], .05)
                        try:
                            part = os.read(master, 8192)
                        except BlockingIOError:
                            if process.poll() is not None:
                                break
                            continue
                        except OSError as error:
                            if error.errno == errno.EIO:
                                break
                            raise
                        if not part:
                            break
                        data.extend(part)
                    result = subprocess.CompletedProcess(args, process.wait(timeout=1))
                    output = data.decode()
                finally:
                    os.close(master)
                    if slave is not None:
                        os.close(slave)
            else:
                result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=15)
                output = result.stdout + result.stderr
            self.assertEqual(result.returncode == 0, not broken, output)
            # The intro is the first board's (T-903): the installer never runs it.
            self.assertFalse((root / "welcome.log").exists(), "the installer ran mesimon welcome")
            return output

    def test_utf8_terminal_gets_the_resting_mascot_after_success(self):
        output = self.install()
        self.assertIn("mesimon is ready", output)
        self.assertIn("███████████▄▄███████████", output)
        self.assertLess(output.index("installed mesimon-test"), output.index("mesimon is ready"))
        self.assertIn("mesimon doctor", output)
        self.assertNotIn("\x1b", output)

    def test_pipe_keeps_plain_install_instructions(self):
        output = self.install(tty=False)
        self.assertNotIn("█", output)
        self.assertIn("mesimon doctor", output)

    def test_dumb_and_non_utf8_terminals_keep_plain_text(self):
        for options in [dict(term="dumb"), dict(locale="C")]:
            with self.subTest(options=options):
                output = self.install(**options)
                self.assertNotIn("█", output)
                self.assertIn("mesimon doctor", output)

    def test_failed_binary_never_gets_a_welcome(self):
        output = self.install(broken=True)
        self.assertNotIn("█", output)
        self.assertNotIn("mesimon is ready", output)
        self.assertIn("would not run", output)


class InstallerVersion(unittest.TestCase):
    """The tag installed with no --version is the highest listed, not the first."""

    install = InstallerWelcome.install

    # The order GitHub listed on 2026-10-19 (T-877): beta.9 and beta.10 sat on
    # one commit of the releases repo, and beta.10 came back third.
    TAGS = ["v0.1.0-beta.9", "v0.1.0-beta.8", "v0.1.0-beta.10", "v0.1.0-beta.7", "v0.1.0-beta.6"]

    def listing(self, indent):
        return json.dumps([
            {"tag_name": tag, "draft": False, "prerelease": True,
             "body": 'notes quoting "tag_name": "v9.9.9"',
             "assets": [{"name": f"mesimon-{tag}-x86_64-unknown-linux-musl.tar.gz"}]}
            for tag in self.TAGS
        ], indent=indent)

    def test_the_highest_tag_is_installed_wherever_it_is_listed(self):
        for indent in [2, None]:
            with self.subTest(pretty=indent is not None):
                output = self.install(tty=False, listing=self.listing(indent),
                                      version="v0.1.0-beta.10")
                self.assertIn("downloading mesimon-v0.1.0-beta.10-x86_64-unknown-linux-musl", output)
                self.assertIn("checksum ok", output)

    def test_an_empty_listing_installs_nothing(self):
        with self.assertRaises(AssertionError) as failed:
            self.install(tty=False, listing="[]")
        self.assertIn("could not find a published release", str(failed.exception))


if __name__ == "__main__":
    unittest.main()
