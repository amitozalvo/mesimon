#!/usr/bin/env python3
"""Replay the TUI's comparison export in an ordinary external terminal.

Generate once (from a ticket checkout):
  env -u NO_COLOR MESIMON_TERMINAL_COMPARISON=/tmp/mesimon-comparison.json \
    python3 -B ci/test-run.py -- cargo test -p mesimon-tui terminal_comparison_export
Then run this script with that file in each terminal to compare. No daemon,
agents, tmux sessions, settings writes or terminal capability queries are used.
The export contains real UI buffers encoded by the production Quiet backend.
"""
import argparse
import json
import os
import select
import signal
import sys
import termios
import tty


def arguments():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("data", nargs="?", help="comparison JSON exported by the Rust test")
    p.add_argument("--theme", choices=["graphite", "chalk"], default="graphite")
    p.add_argument("--width", type=int, choices=[60, 120], default=120)
    p.add_argument("--endpoint", choices=["auto", "apple", "native"], default="auto")
    p.add_argument("--autoplay", action="store_true", help="play once, then allow stepping")
    p.add_argument("--frame", type=int, default=1, help="first frame, numbered from 1")
    p.add_argument("--through", type=int, help="pause autoplay at this frame")
    p.add_argument("--keys", action="store_true", help="show received key bytes; q exits")
    return p.parse_args()


def read_key(timeout=None):
    if not select.select([sys.stdin], [], [], timeout)[0]:
        return b""
    return os.read(sys.stdin.fileno(), 4096)


def main():
    args = arguments()
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise SystemExit("Run from an ordinary external terminal, not a pipe or agent pane.")
    if args.keys:
        frames = None
    else:
        if not args.data:
            raise SystemExit("Supply the exported comparison JSON, or --keys.")
        with open(args.data, encoding="utf-8") as f:
            data = json.load(f)
        if data.get("version") != 1:
            raise SystemExit("Unsupported comparison export version.")
        endpoint = args.endpoint
        if endpoint == "auto":
            endpoint = "apple" if os.environ.get("TERM_PROGRAM") == "Apple_Terminal" else "native"
        case = next(s for s in data["sets"] if s["theme"] == args.theme and s["width"] == args.width and s["endpoint"] == endpoint)
        frames = case["frames"]
        if not 1 <= args.frame <= len(frames) or (args.through is not None and not args.frame <= args.through <= len(frames)):
            raise SystemExit("Frame range is outside this comparison.")

    out = sys.stdout
    saved = termios.tcgetattr(sys.stdin)

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        tty.setraw(sys.stdin.fileno())
        out.write("\x1b[?1049h\x1b[?25l\x1b[0m\x1b[2J\x1b[H")
        out.flush()
        if frames is None:
            out.write("Key bytes: try Enter, Shift+Enter, arrows, modifiers and a short paste. q exits.\r\n")
            out.flush()
            while True:
                key = read_key()
                if key in (b"q", b"\x03"):
                    break
                # Print an escaped representation, never execute typed escapes.
                out.write(repr(key) + "\r\n")
                out.flush()
            return

        index, full, autoplay = args.frame - 1, False, args.autoplay
        previous, shown = None, None
        while True:
            size = os.get_terminal_size(sys.stdout.fileno())
            fits = size.columns >= case["width"] and size.lines >= case["height"] + 2
            if fits and shown != (index, full, size):
                frame = frames[index]
                fresh = full or previous is None or index != previous + 1 or (shown and shown[2] != size)
                if fresh:
                    out.write("\x1b[0m\x1b[2J\x1b[H")
                out.write(frame["full"] if fresh else frame["diff"])
                label = f"{index+1}/{len(frames)} {frame['name']} | {endpoint} {args.theme} | {'full' if full else 'diff'}"
                out.write(f"\x1b[0m\x1b[{case['height']+1};1H\x1b[2K{label[:size.columns]}")
                hints = "space/n next  p previous  a animate  r restart  f full/diff  q quit"
                out.write(f"\x1b[{case['height']+2};1H\x1b[2K{hints[:size.columns]}")
                out.flush()
                previous = index
                shown = (index, full, size)
            elif not fits and shown != (index, full, size):
                out.write(f"\x1b[0m\x1b[2J\x1b[HResize to at least {case['width']} columns x {case['height']+2} rows; q exits.")
                out.flush()
                previous = None
                shown = (index, full, size)
            key = read_key(frames[index]["ms"] / 1000 if fits and autoplay else 0.2)
            # AppleScript's Terminal `do script` appends a newline. Each
            # diagnostic control is a single byte; ignore that terminator.
            key = key[:1]
            if key in (b"q", b"\x03", b"\x1b"):
                break
            if key == b"f":
                full = not full
                previous = None
            elif key == b"r":
                index, previous = 0, None
                shown = None
            elif key == b"a":
                autoplay = not autoplay
            elif key == b"p":
                index = max(0, index - 1)
            elif key in (b" ", b"n", b"\r") or (not key and fits and autoplay):
                if not key and args.through is not None and index + 1 >= args.through:
                    autoplay = False
                elif index + 1 < len(frames):
                    index += 1
                else:
                    autoplay = False
    except KeyboardInterrupt:
        pass
    finally:
        out.write("\x1b[?2026l\x1b[0m\x1b[?25h\x1b[?1049l")
        out.flush()
        termios.tcsetattr(sys.stdin, termios.TCSADRAIN, saved)


if __name__ == "__main__":
    main()
