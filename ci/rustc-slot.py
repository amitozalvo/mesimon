#!/usr/bin/env python3
"""One machine-wide cap on running compilers, shared by every ticket worktree.

Cargo's `build.rustc-wrapper`: cargo runs `rustc-slot.py rustc <args>` for
every crate, this takes one of `MESIMON_RUSTC_SLOTS` slots and becomes
rustc. `.mesimon-worktree-init.sh` installs it beside the worktrees' shared
cargo config; nothing shipped uses it, and a build without it is the same
build.

Why (T-1193, 2026-11-10): each `cargo` caps its own jobs and knows nothing
of the others. Six ticket builds at once were 60 compilers on 11 cores and
4 GB each on an 18 GB machine: `cargo ut` took 53 s alone, 63 s beside one
other build and 600 s beside three, and the person stopped every ticket to
get the machine back. A cap per cargo (`build.jobs`) bounds one build; this
bounds their sum, so the fifth ticket's build waits seconds for a slot
where it used to take the machine down with it.

A slot is a file lock (`flock`) on an inherited descriptor: it is held by
the rustc process itself, with the linker it spawns, and the kernel drops
it when that exits or is killed. Nothing here outlives the compile and no
slot can leak. Probes (`rustc -vV`, `--print`) name no crate and pass
straight through.

The slot files sit in /tmp so every session of this user finds the same
ones. That name is guessable, so the directory must be this user's own and
closed to others, a slot file is never opened through a symlink, and a
compile that has waited `MESIMON_RUSTC_SLOT_WAIT` seconds goes ahead
without a slot: another account on the machine can neither hold a build
back by taking the locks nor point a slot at a file of its choosing. The
cap paces builds; it must never be what fails one.

    MESIMON_RUSTC_SLOTS=6        compilers at once, machine-wide (0: no cap)
    MESIMON_RUSTC_SLOT_WAIT=300  seconds to wait for a slot before compiling without one
    MESIMON_RUSTC_SLOT_DIR=…     where the slot files live (the tests' own)
"""
import fcntl
import os
import stat
import sys
import time

DEFAULT_SLOTS = 6
DEFAULT_WAIT = 300.0


def number(name: str, default: float) -> float:
    try:
        return float(os.environ.get(name, default))
    except ValueError:
        return default


def take_slot(slots: int, wait: float) -> None:
    """Hold one of `slots` locks on a descriptor the exec'd rustc inherits.
    Raises OSError when there is no slot to be had safely or in time."""
    directory = os.environ.get("MESIMON_RUSTC_SLOT_DIR") or f"/tmp/mesimon-rustc-slots-{os.getuid()}"
    os.makedirs(directory, mode=0o700, exist_ok=True)
    held = os.lstat(directory)
    if not stat.S_ISDIR(held.st_mode) or held.st_uid != os.getuid() or held.st_mode & 0o077:
        raise OSError(f"{directory} is not a directory of this user's alone")
    first = os.getpid() % slots
    deadline = time.monotonic() + wait
    while True:
        for step in range(slots):
            fd = os.open(f"{directory}/{(first + step) % slots}", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                os.close(fd)
                continue
            os.set_inheritable(fd, True)
            return
        if time.monotonic() >= deadline:
            raise OSError(f"every slot still held after {wait:g} s")
        time.sleep(0.05)


def main() -> int:
    command = sys.argv[1:]
    if not command:
        print("rustc-slot: no compiler to run; this is cargo's build.rustc-wrapper", file=sys.stderr)
        return 2
    if "--crate-name" in command:
        slots = int(number("MESIMON_RUSTC_SLOTS", DEFAULT_SLOTS))
        if slots > 0:
            try:
                take_slot(slots, number("MESIMON_RUSTC_SLOT_WAIT", DEFAULT_WAIT))
            except OSError as error:
                # A cap that cannot be taken must not fail the build it was meant to pace.
                print(f"rustc-slot: no slot ({error}); compiling without one", file=sys.stderr)
    os.execvp(command[0], command)


if __name__ == "__main__":
    sys.exit(main())
