#!/usr/bin/env python3
"""Capture the installed agent CLIs' screens and check the ticket page reads them (T-1156).

The ticket page draws an agent's pane at its foot: it finds the composer's box by its
rules (Codex: by the cursor's prompt row), cuts everything under it, and takes the
agent's key hints out of its status rows. Those shapes are the agent's words and move
with its versions. This launches each installed CLI in a private tmux server, puts up
each shape the page reads, captures it with its colours and cursor, and runs the
page's reader over the captures (`cargo test -p mesimon-tui every_capture`).

Captures land under ~/.local/state/mesimon/release-watch/captures/<cli>-<version>/,
never in the repository: a real banner can carry the account's name. One tiny Haiku
turn per prompted shape; a wall-clock deadline per shape; no retries.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import test_guard

ROOT = Path(__file__).resolve().parent.parent
CAPTURES = Path.home() / ".local/state/mesimon/release-watch/captures"
COLS, ROWS = 100, 30
STATUS = "printf 'fixture-status | ~/repo | opus'"
# Each shape: what Claude is asked (None: nothing), its settings, its tools, and
# the screen text that says the shape is up.
CLAUDE = {
    "idle": (None, {}, "", "❯"),
    "idle-statusline": (None, {"statusLine": {"type": "command", "command": STATUS}}, "", "fixture-status"),
    "working": ("Use Bash to run sleep 20, then reply LAB_DONE.", {}, "Bash", "esc to interrupt|tokens"),
    "permission": ("Use Bash to run printf hi > hi.txt, then reply LAB_DONE.", {}, "Bash", "Do you want to proceed?"),
}
CODEX = {
    "idle": (None, "›"),
}


def run_shape(tmux, argv, repo, want, deadline, prompt_after=None):
    """Start `argv` in a private server, answer a fresh folder's trust prompt, type
    `prompt_after` once its box is drawn, and capture the screen once `want` (a
    `|`-separated list of alternatives) is on it. Returns (screen, cursor) or None."""
    with tempfile.TemporaryDirectory(prefix="msmn-pane-") as scratch:
        sock = str(Path(scratch) / "t.sock")

        def tm(*a):
            return subprocess.run([tmux, "-S", sock, *a], capture_output=True, text=True, timeout=5)

        env = {k: v for k, v in os.environ.items() if not k.startswith("MESIMON_")
               and k not in ("CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_ENTRYPOINT", "TMUX", "TMUX_PANE")}
        start = subprocess.run([tmux, "-S", sock, "-f", "/dev/null", "new-session", "-d", "-s", "p",
                                "-x", str(COLS), "-y", str(ROWS), "-c", str(repo), "sleep 300"],
                               env=env, capture_output=True, text=True, timeout=10)
        if start.returncode:
            raise RuntimeError(start.stderr)
        tm("set-option", "-w", "-t", "p", "remain-on-exit", "on")
        tm("respawn-pane", "-k", "-t", "p", shlex.join(argv))
        pane_pid = int(tm("display-message", "-p", "-t", "p", "#{pane_pid}").stdout.strip())
        started, typed, trusted_at, got = time.monotonic(), False, 0.0, None
        try:
            while time.monotonic() - started < deadline:
                screen = tm("capture-pane", "-p", "-t", "p").stdout
                now = time.monotonic() - started
                if "rust this folder" in screen and now - trusted_at > 0.7:
                    # Claude Code lists `❯ No, exit` first; Codex 0.166's
                    # `Trust this folder?` selects `› 1. Trust and continue`.
                    key = "Down" if "❯ No, exit" in screen else "Enter"
                    tm("send-keys", "-t", "p", key)
                    trusted_at = now
                elif "esc skip" in screen and now - trusted_at > 0.7:
                    # Codex's "Update available" offer: skipped, never taken.
                    tm("send-keys", "-t", "p", "Escape")
                    trusted_at = now
                elif prompt_after and not typed and box_drawn(screen):
                    tm("send-keys", "-t", "p", "-l", prompt_after)
                    time.sleep(0.3)
                    tm("send-keys", "-t", "p", "Enter")
                    typed = True
                elif (not prompt_after or typed) and any(w in screen for w in want.split("|")) \
                        and (prompt_after or box_drawn(screen)):
                    time.sleep(0.5)  # one more frame: the box's rules settle
                    raw = tm("capture-pane", "-p", "-e", "-t", "p").stdout
                    cursor = tm("display-message", "-p", "-t", "p", "#{cursor_x} #{cursor_y}").stdout.split()
                    # The shape must still be up on the screen saved: Codex
                    # draws its box, then an update offer over it.
                    again = tm("capture-pane", "-p", "-t", "p").stdout
                    if any(w in again for w in want.split("|")) and (prompt_after or box_drawn(again)):
                        got = (raw, (int(cursor[0]), int(cursor[1])))
                        break
                time.sleep(0.25)
        finally:
            owned = test_guard.descendants(test_guard.process_table(), {pane_pid})
            tm("kill-server")
            for sig in (signal.SIGTERM, signal.SIGKILL):
                table = test_guard.process_table()
                for pid, before in owned.items():
                    if test_guard.same_process(before, table.get(pid)):
                        try:
                            os.kill(pid, sig)
                        except ProcessLookupError:
                            pass
                time.sleep(0.2)
        return got


def box_drawn(screen):
    """Claude Code's composer: a `❯` row under a rule. Codex's: a `›` row at the
    left edge that is not a dialog's numbered option."""
    rows = screen.splitlines()
    option = re.compile(r"^› \d+\.")
    return any(r.lstrip().startswith(("❯", "›")) and i and set(rows[i - 1].strip()) == {"─"}
               for i, r in enumerate(rows)) or any(r.startswith("›") and not option.match(r) for r in rows)


def save(out, name, shot):
    raw, (x, y) = shot
    (out / f"{name}.txt").write_text(raw)
    (out / f"{name}.cursor").write_text(f"{x} {y} {COLS}\n")
    print(f"  {name}: captured")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", choices=["claude", "codex"], help="one CLI")
    parser.add_argument("--deadline", type=int, default=60, help="seconds per shape")
    parser.add_argument("--no-check", action="store_true", help="capture only; skip the cargo check")
    args = parser.parse_args()
    os.umask(0o077)
    tmux = shutil.which("tmux")
    if not tmux:
        parser.error("tmux is required")
    dirs = []
    for cli in ("claude", "codex"):
        if args.only and args.only != cli:
            continue
        exe = shutil.which(cli)
        if not exe:
            print(f"{cli}: not installed, skipped")
            continue
        version = subprocess.check_output([exe, "--version"], text=True, timeout=10).split()[-1 if cli == "codex" else 0]
        out = CAPTURES / f"{cli}-{version}"
        shutil.rmtree(out, ignore_errors=True)  # a shape not seen this run is not checked
        out.mkdir(parents=True)
        print(f"{cli} {version} -> {out}")
        with tempfile.TemporaryDirectory(prefix="msmn-pane-repo-") as scratch:
            repo = Path(scratch)
            subprocess.run(["git", "init", "-q", str(repo)], check=True, timeout=10)
            if cli == "claude":
                for name, (prompt, settings, tools, want) in CLAUDE.items():
                    file = repo / f"{name}-settings.json"
                    file.write_text(json.dumps(settings))
                    argv = [exe, "--model", "haiku", "--setting-sources", "", "--settings", str(file),
                            "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
                            "--permission-mode", "default", "--tools", tools]
                    if name == "working":
                        argv += ["--allowedTools", "Bash(sleep*)"]
                    shot = run_shape(tmux, argv, repo, want, args.deadline, prompt_after=prompt)
                    if shot:
                        save(out, name, shot)
                    else:
                        print(f"  {name}: not seen within {args.deadline} s")
            else:
                for name, (prompt, want) in CODEX.items():
                    shot = run_shape(tmux, [exe], repo, want, args.deadline, prompt_after=prompt)
                    if shot:
                        save(out, name, shot)
                    else:
                        print(f"  {name}: not seen within {args.deadline} s")
        dirs.append(out)
    if args.no_check or not dirs:
        return 0
    failed = 0
    for out in dirs:
        env = dict(os.environ, MESIMON_PANE_CAPTURES=str(out))
        result = subprocess.run(["cargo", "test", "-q", "-p", "mesimon-tui", "--lib", "every_capture"],
                                cwd=ROOT, env=env)
        print(f"{out.name}: {'ok' if result.returncode == 0 else 'THE PAGE MISREADS IT'}")
        failed |= result.returncode
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
