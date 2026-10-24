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
                listing=None, version="v0.0.0-test", channel=None, installed=None,
                marker=None, fail_channel=False, wrong_channel=False, expect_ok=None):
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
case "$url" in */LATEST) [ -f "$FIXTURE_MARKER" ] || exit 22; cat "$FIXTURE_MARKER"; exit 0 ;; esac
case "$url" in *.sha256) cp "$FIXTURE_ASSET.sha256" "$out" ;; *) cp "$FIXTURE_ASSET" "$out" ;; esac''')
            name = f"mesimon-{version}-x86_64-unknown-linux-musl"
            archive = root / (name + ".tar.gz")
            # An older mesimon listed `welcome` in --help; it is recorded if run.
            # `channel <word>` is the binary's own preference writer (T-946):
            # recorded, as the downloaded build, unless the test makes it fail.
            channel_arm = (b"  channel) [ -z \"$FAIL_CHANNEL\" ] || { echo 'prefs.json is not writable'; exit 1; }\n"
                           b"    [ -z \"$WRONG_CHANNEL\" ] || { echo 'follows stable'; exit 0; }\n"
                           b"    echo \"new $2\" >> \"$CHANNEL_LOG\"; echo \"follows $2\"; exit 0 ;;\n")
            binary = (b"#!/bin/sh\n"
                      b"case \"$1\" in --help) echo '  mesimon welcome   the intro' ;;\n"
                      b"  welcome) echo ran >> \"$WELCOME_LOG\" ;;\n" + channel_arm + b"esac\n"
                      b"echo mesimon-test\n" + (b"exit 1\n" if broken else b""))
            with tarfile.open(archive, "w:gz") as tar:
                entry = tarfile.TarInfo(name + "/mesimon")
                entry.size, entry.mode = len(binary), 0o755
                tar.addfile(entry, io.BytesIO(binary))
            Path(str(archive) + ".sha256").write_text(
                hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n"
            )
            prefix = root / "install with spaces"
            channel_log = root / "channel.log"
            env = dict(os.environ, PATH=str(commands) + ":/usr/bin:/bin", LC_ALL=locale,
                       TERM=term, PREFIX=str(prefix), FIXTURE_ASSET=str(archive),
                       WELCOME_LOG=str(root / "welcome.log"), CHANNEL_LOG=str(channel_log),
                       FIXTURE_MARKER=str(root / "LATEST"))
            env.pop("FAIL_CHANNEL", None)
            env.pop("WRONG_CHANNEL", None)
            if fail_channel:
                env["FAIL_CHANNEL"] = "1"
            if wrong_channel:
                env["WRONG_CHANNEL"] = "1"
            if marker is not None:
                (root / "LATEST").write_text(marker + "\n")
            if installed is not None:
                # What `--version` prints: `mesimon <version> (<sha>, <date>)`;
                # `installed=""` is a binary whose version cannot be read.
                prefix.mkdir()
                said = f"mesimon {installed} (abc1234, 2026-10-24)" if installed else "garbage"
                (prefix / "mesimon").write_text(
                    "#!/bin/sh\n"
                    'if [ "$1" = channel ]; then echo "kept $2" >> "$CHANNEL_LOG"; echo "follows $2"; exit 0; fi\n'
                    f"echo '{said}'\n")
                (prefix / "mesimon").chmod(0o755)
            args = ["sh", str(ROOT / "install.sh"), "--version", version]
            if listing is not None:
                (root / "listing.json").write_text(listing)
                env["FIXTURE_LISTING"] = str(root / "listing.json")
                args = args[:2]
            if channel is not None:
                args += ["--channel", channel]
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
            self.assertEqual(result.returncode == 0,
                             (not broken) if expect_ok is None else expect_ok, output)
            # The intro is the first board's (T-903): the installer never runs it.
            self.assertFalse((root / "welcome.log").exists(), "the installer ran mesimon welcome")
            self.recorded = channel_log.read_text().split("\n")[:-1] if channel_log.exists() else []
            self.replaced = (prefix / "mesimon").exists() and \
                b"mesimon-test" in (prefix / "mesimon").read_bytes()
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
        self.assertIn("could not find a published stable release", str(failed.exception))


class InstallerChannel(unittest.TestCase):
    """T-946: both channels share one list; the tag says which is which."""

    install = InstallerWelcome.install

    def listing(self, *tags):
        return json.dumps([{"tag_name": t, "draft": False, "prerelease": True} for t in tags])

    TRAIN = ("unstable-0.1.0-beta.13.unstable.2", "v0.1.0-beta.13",
             "unstable-0.1.0-beta.13.unstable.1", "v0.1.0-beta.12",
             # Never minted: an unstable version under `v`, a stable one
             # under `unstable-`. Neither channel takes either.
             "v0.1.0-beta.13.unstable.9", "unstable-0.1.0-beta.99")

    def test_stable_takes_no_unstable_tag(self):
        output = self.install(tty=False, listing=self.listing(*self.TRAIN),
                              version="v0.1.0-beta.13")
        self.assertIn("downloading mesimon-v0.1.0-beta.13-x86_64", output)
        self.assertEqual(self.recorded, [], "a default channel records nothing")

    def test_unstable_takes_the_higher_of_both_and_records_the_choice(self):
        tag = "unstable-0.1.0-beta.13.unstable.2"
        output = self.install(tty=False, listing=self.listing(*self.TRAIN), version=tag,
                              channel="unstable")
        self.assertIn(f"downloading mesimon-{tag}-x86_64", output)
        self.assertEqual(self.recorded, ["new unstable"], "the choice, by the new binary")

    def test_a_promotion_picked_on_unstable_keeps_unstable(self):
        # The highest release is stable; the explicit choice still persists,
        # over an opposite one already saved (the binary's writer replaces it).
        output = self.install(tty=False, listing=self.listing("v0.1.0-beta.14", *self.TRAIN),
                              version="v0.1.0-beta.14", channel="unstable",
                              installed="0.1.0-beta.13")
        self.assertIn("downloading mesimon-v0.1.0-beta.14-x86_64", output)
        self.assertEqual(self.recorded, ["new unstable"])

    def test_a_newer_install_is_kept_and_the_choice_recorded(self):
        output = self.install(tty=False, listing=self.listing(*self.TRAIN),
                              version="v0.1.0-beta.13", channel="stable",
                              installed="0.1.0-beta.13.unstable.2")
        self.assertIn("mesimon 0.1.0-beta.13.unstable.2 is newer than the newest stable", output)
        self.assertNotIn("downloading", output)
        self.assertEqual(self.recorded, ["kept stable"], "the waiting binary records it")
        # An older one is replaced, as ever, and with no --channel nothing
        # is recorded.
        output = self.install(tty=False, listing=self.listing(*self.TRAIN),
                              version="v0.1.0-beta.13", installed="0.1.0-beta.12")
        self.assertIn("downloading mesimon-v0.1.0-beta.13-x86_64", output)
        self.assertEqual(self.recorded, [])

    def test_a_choice_that_cannot_be_saved_replaces_nothing(self):
        output = self.install(tty=False, listing=self.listing(*self.TRAIN),
                              version="unstable-0.1.0-beta.13.unstable.2", channel="unstable",
                              installed="0.1.0-beta.12", fail_channel=True, expect_ok=False)
        self.assertIn("the unstable channel could not be recorded: prefs.json is not writable",
                      output)
        self.assertFalse(self.replaced, "the binary was replaced after the save failed")

    def test_a_choice_answered_with_another_channel_cannot_proceed(self):
        # A binary that exits 0 but says it follows something else did not
        # save this choice (T-946 round 2): nothing proceeds.
        output = self.install(tty=False, listing=self.listing(*self.TRAIN),
                              version="unstable-0.1.0-beta.13.unstable.2", channel="unstable",
                              installed="0.1.0-beta.12", wrong_channel=True, expect_ok=False)
        self.assertIn("the unstable channel could not be recorded: follows stable", output)
        self.assertFalse(self.replaced, "the binary was replaced on another channel's answer")

    def test_an_unreadable_installed_version_is_not_replaced(self):
        output = self.install(tty=False, listing=self.listing(*self.TRAIN),
                              version="v0.1.0-beta.13", installed="", expect_ok=False)
        self.assertIn("could not read the version of the mesimon at", output)
        self.assertNotIn("downloading", output)

    def test_an_explicit_version_alone_records_no_channel(self):
        self.install(tty=False, version="v0.0.0-test")
        self.assertEqual(self.recorded, [])

    def test_a_page_of_unstable_releases_falls_back_to_the_stable_marker(self):
        page = [f"unstable-0.1.0-beta.14.unstable.{n}" for n in range(1, 101)]
        output = self.install(tty=False, listing=self.listing(*page), version="v0.1.0-beta.14",
                              marker="v0.1.0-beta.14")
        self.assertIn("downloading mesimon-v0.1.0-beta.14-x86_64", output)
        # A marker that names no stable tag is no answer.
        output = self.install(tty=False, listing=self.listing(*page), version="v0.1.0-beta.14",
                              marker="unstable-0.1.0-beta.14.unstable.101", expect_ok=False)
        self.assertIn("could not find a published stable release", output)

    def test_a_body_that_is_no_listing_never_reads_the_marker(self):
        # A rate limit's answer is not a page without stable releases.
        output = self.install(tty=False, listing='{"message": "API rate limit exceeded"}',
                              version="v0.1.0-beta.14", marker="v0.1.0-beta.14", expect_ok=False)
        self.assertIn("could not find a published stable release", output)
        self.assertNotIn("downloading", output)

    def test_an_unknown_channel_is_refused(self):
        output = self.install(tty=False, listing=self.listing(*self.TRAIN), channel="nightly",
                              expect_ok=False)
        self.assertIn("unknown channel: nightly", output)


if __name__ == "__main__":
    unittest.main()
