"""Drive the built board and its tmux handover through a disposable terminal.

Called by slow_tmux_e2e with the fixture's HOME, repository and tmux socket.
"""
import fcntl
import os
from pathlib import Path
import pty
import select
import shlex
import struct
import subprocess
import sys
import termios
import time

binary, repo, sock, tmux = sys.argv[1:]
state = Path(os.environ['HOME']) / '.local/state/mesimon'
state.mkdir(parents=True, exist_ok=True)
(state / 'welcomed').write_text('skipped board test\n')
(state / 'prefs.json').write_text('{"schema_version":1,"follow_os":false,"enter_ticket_page":false}')
marker = state / 'hold-capture'
reached = state / 'capture-held'
wrapper = state / 'tmux-preview.sh'
wrapper.write_text(
    '#!/bin/sh\n'
    f'if [ -f {shlex.quote(str(marker))} ]; then\n'
    'for arg do\nif [ "$arg" = capture-pane ]; then\n'
    f': > {shlex.quote(str(reached))}\n'
    f'n=0; while [ -f {shlex.quote(str(marker))} ] && [ $n -lt 100 ]; do sleep 0.05; n=$((n+1)); done\n'
    'fi; done; fi\n'
    f'exec {shlex.quote(tmux)} "$@"\n'
)
wrapper.chmod(0o700)
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
env = dict(os.environ, TERM='xterm-256color', MESIMON_THEME='graphite',
           MESIMON_TMUX_BIN=str(wrapper), MESIMON_NO_DAEMON_RESTART='1')

def terminal_session():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)

proc = subprocess.Popen([binary, 'open', repo], stdin=slave, stdout=slave,
                        stderr=slave, env=env, preexec_fn=terminal_session)
output = bytearray()

def pump():
    if select.select([master], [], [], 0.02)[0]:
        chunk = os.read(master, 65536)
        output.extend(chunk)
        # Answer the cursor fence used when giving the terminal to tmux.
        if b'\x1b[6n' in chunk:
            os.write(master, b'\x1b[1;1R')
    assert proc.poll() is None, f'board exited: {output[-4000:]!r}'

def until(check, label, seconds=10):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        pump()
        if check():
            return
    raise AssertionError(f'{label}: {output[-4000:]!r}')

def attached():
    found = subprocess.run([tmux, '-S', sock, 'list-clients', '-F', '#{client_tty}'],
                           capture_output=True, timeout=2)
    return b'/dev/' in found.stdout

def key(value):
    output.clear()
    os.write(master, value)

try:
    until(lambda: b'TODO' in output, 'board drawn')
    key(b'\r')
    until(attached, 'board Enter attached to a real pane')
    until(lambda: b'LIVE_PREVIEW_READY' in output, 'attached pane rendered')
    key(b'\x1d')
    until(lambda: not attached(), 'detached back to the board')
    until(lambda: b'TODO' in output, 'board redrawn')
    key(b' ')
    until(lambda: b'LIVE_PREVIEW_READY' in output, 'ticket preview rendered')
    marker.touch()
    until(reached.exists, 'preview capture held')
    stalled_at = time.monotonic()
    key(b'\x1b')
    # A title appears on both screens; the board's column label proves
    # navigation and rendering continued while capture was still held.
    until(lambda: b'TODO' in output, 'page exit stayed responsive', seconds=1)
    assert marker.exists(), 'capture barrier still held during page exit'
    # Exercise the real event loop, including the acknowledgements after
    # every key. Each byte must be drawn before the next byte is sent.
    # This PTY advertises no enhanced keyboard: Escape+Return is the
    # supported prompt chord on that terminal, rather than Shift+Enter.
    for opening, label in [(b'o', 'new ticket'), (b'\x1b\r', 'prompt')]:
        key(opening)
        began = time.monotonic()
        latencies = []
        for character in b'ZYXWVUTSRQP':
            typed_at = time.monotonic()
            key(bytes([character]))
            until(lambda: bytes([character]) in output, label + ' typed character', seconds=0.5)
            latencies.append(time.monotonic() - typed_at)
        print(f'{label} character draw: max {max(latencies)*1000:.1f} ms, mean {sum(latencies)/len(latencies)*1000:.1f} ms')
        assert time.monotonic() - began < 1, label + ' typing took a second'
        key(b'\x1b[27u')
    assert time.monotonic() - stalled_at < 4, 'typing completed before capture barrier timed out'
    reached.unlink(missing_ok=True)
    key(b' ')
    until(lambda: b'LIVE_PREVIEW_READY' in output, 'preview recovered after cleanup')
    until(reached.exists, 'new preview capture held before Enter')
    entered = time.monotonic()
    key(b'\r')
    # LeaveAlternateScreen ends the board's part of the handoff. Only the
    # stub's marker after that boundary proves the attached pane was drawn.
    left_board = b'\x1b[?1049l'
    until(lambda: left_board in output, 'ticket handed over the terminal', seconds=1)
    released = time.monotonic()
    until(attached, 'ticket Enter cancelled capture and attached to a real pane',
          seconds=1 - (released - entered))
    marker.unlink()
    def pane_drawn():
        return b'LIVE_PREVIEW_READY' in output.partition(left_board)[2]

    if not pane_drawn():
        until(pane_drawn, 'ticket handover rendered')
    drawn = time.monotonic()
    print(f'Ticket Enter: board released terminal after {(released-entered)*1000:.0f} ms, '
          f'pane observed after {(drawn-entered)*1000:.0f} ms')
    key(b'\x1d')
    until(lambda: not attached(), 'ticket detach completed')
    print('PTY passed: board Enter, real pane rendering, preview, held-capture page exit and typing in creation/prompt fields, ticket Enter and detach')
finally:
    marker.unlink(missing_ok=True)
    # Release the terminal before waiting: a client restoring termios can
    # otherwise be stuck behind output nobody drains during teardown.
    os.close(master)
    os.close(slave)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
