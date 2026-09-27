#!/usr/bin/env python3
"""Run a command on a pty that speaks the kitty keyboard protocol's probe.

VHS records through xterm.js, which does not answer `CSI ? u`, so the board
reads the terminal as legacy-floor and every Shift+Enter binding is inert and
unhinted there -- exactly as it should be on such a terminal. The demo is
about the board a user sees in iTerm2, Ghostty, kitty or WezTerm, so this
shim stands in for that terminal and nothing else:

  * the board's `CSI ? u` query is answered with `CSI ? 1 u` on its input,
    ahead of the primary device attributes reply xterm.js sends;
  * the push/pop of the enhancement flags is dropped (xterm.js ignores it);
  * Alt+Enter from the tape (`ESC CR`, a chord the demo never needs) is
    delivered as the kitty report for Shift+Enter, `CSI 13 ; 2 u`.

Every other byte passes through unchanged, both ways.

Usage: kitty-term.py <command> [args...]
"""

import fcntl
import os
import pty
import re
import select
import signal
import sys
import termios
import tty

QUERY = b"\x1b[?u"
REPLY = b"\x1b[?1u"
# Push `CSI > flags u`, pop `CSI < n u`, set `CSI = flags ; mode u`.
FLAGS = re.compile(rb"\x1b\[[<>=][0-9;]*u")
SHIFT_ENTER_IN = b"\x1b\r"
SHIFT_ENTER_OUT = b"\x1b[13;2u"


def winsize(src, dst):
    size = fcntl.ioctl(src, termios.TIOCGWINSZ, b"\0" * 8)
    fcntl.ioctl(dst, termios.TIOCSWINSZ, size)


def split_tail(buf):
    """Hold back a trailing, possibly incomplete escape sequence."""
    esc = buf.rfind(b"\x1b")
    if esc == -1 or len(buf) - esc > 16:
        return buf, b""
    tail = buf[esc:]
    if re.fullmatch(rb"\x1b(\[[<>=?]?[0-9;]*)?", tail):
        return buf[:esc], tail
    return buf, b""


def main():
    argv = sys.argv[1:]
    if not argv:
        sys.exit(__doc__)
    pid, fd = pty.fork()
    if pid == 0:
        os.execvp(argv[0], argv)
    winsize(0, fd)
    signal.signal(signal.SIGWINCH, lambda *_: winsize(0, fd))
    saved = termios.tcgetattr(0)
    tty.setraw(0)
    held = b""
    try:
        while True:
            try:
                ready, _, _ = select.select([0, fd], [], [])
            except InterruptedError:
                continue
            if fd in ready:
                try:
                    out = os.read(fd, 65536)
                except OSError:
                    break
                if not out:
                    break
                out, held = split_tail(held + out)
                if QUERY in out:
                    os.write(fd, REPLY)
                    out = out.replace(QUERY, b"")
                os.write(1, FLAGS.sub(b"", out))
            if 0 in ready:
                keys = os.read(0, 4096)
                if not keys:
                    break
                os.write(fd, keys.replace(SHIFT_ENTER_IN, SHIFT_ENTER_OUT))
    finally:
        termios.tcsetattr(0, termios.TCSADRAIN, saved)
    _, status = os.waitpid(pid, 0)
    sys.exit(os.waitstatus_to_exitcode(status))


if __name__ == "__main__":
    main()
