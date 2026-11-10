#!/usr/bin/env python3
"""Delete the stale test executables in target/debug/deps, and the
split-debuginfo objects no remaining binary references.

Why this exists (2026-09-06): on macOS every compile leaves its `.rcgu.o`
files loose in `deps/` (the default `split-debuginfo = "unpacked"`), each
named by a content hash, and cargo never removes the old set — one edit to
a core crate mints ~6,500 new objects. In a week `deps/` reached 879,000
entries and 50 GB, and Gatekeeper's first-exec check of a fresh binary
walks the executable's directory, so every freshly linked e2e binary was
held 25–37 s before it ran (0.4 s from a small directory). The release gate
paid that 36 times over.

What is safe to delete is decidable: a linked executable or dylib lists the
objects its debuginfo lives in as `OSO` stab entries (`nm -ap`), so a loose
`.o` named by no current binary is an orphan. Everything referenced stays,
which is what keeps file:line in a panic's backtrace. Runs under cargo's own
build locks (every `.cargo*lock` in the profile dir: Cargo 1.98 holds
`.cargo-build-lock` and `.cargo-artifact-lock` exclusively for a whole build
and `.cargo-lock` shared, and a separate `build.build-dir` keeps its own
`.cargo-build-lock`), so it cannot delete an object a link in progress is
about to read. Taken all at once or not at all, so it never holds one while
a build holds another.

Stale executables (T-1193, 2026-11-10): a version bump, a toolchain update
or a changed flag gives every test binary a new hash, the old one stays
beside it, and cargo never removes those either (a relink of unchanged
flags overwrites in place). They kept their objects alive, and this script ran
`nm` over every one of them after every test run: a ticket worktree seeded
from the checkout held 1,149 executables (18.7 GB) of which 91 were
current, and the scan took 138 s after a 2 s test. So the executables
superseded by newer links of the same target go first (`KEEP_PER_TARGET`
newest stay), and what is left is scanned four at a time, each executable
once: a linked file never changes, so its object list is kept in
`.msmn-prune-objects.json` beside deps/ and only a new link is read. 13 s
for the first prune of that worktree where it was 138, and about a second
after one relinked test binary. Cargo relinks an executable it finds
missing, so a wrong guess costs one link, and `target/debug/<bin>` is a
copy of its own that this never touches.

    python3 ci/prune-deps.py            # prune, print what went
    python3 ci/prune-deps.py --dry-run  # count only

ci/test-run.py runs it after every bounded check, on the profile dir of the
Cargo build directory that check used (T-830). Python 3 is a test
dependency only; nothing shipped uses it.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

# Past this many entries something other than orphaned objects is piling
# up (old-hash executables after a version bump, say) and a `cargo clean`
# is the honest answer — the full rebuild is ~25 s.
ADVISE_CLEAN_AT = 50_000

KNOWN_NON_EXE = (".o", ".d", ".rmeta", ".rlib", ".dylib", ".a", ".so")

# How many links of one target stay in deps/, newest first. Two, not one:
# the `mesimon` bin and its unit-test harness share a name, and a build
# under other flags lands beside the usual one.
KEEP_PER_TARGET = 2
# `nm` runs beside whatever else the machine is building; four is a third of it.
SCANS_AT_ONCE = 4
OBJECTS_KEPT = ".msmn-prune-objects.json"
HASHED = re.compile(r"^(?P<target>.+)-[0-9a-f]{16}$")


def stale_executables(deps: Path) -> list:
    """The hashed executables in deps/ that newer links of the same target
    have superseded."""
    by_target = {}
    for f in deps.iterdir():
        found = HASHED.match(f.name)
        if not found or f.name.endswith(KNOWN_NON_EXE):
            continue
        try:
            st = f.stat()
        except OSError:
            continue
        if not f.is_file() or not os.access(f, os.X_OK):
            continue
        by_target.setdefault(found["target"], []).append((st.st_mtime_ns, f))
    stale = []
    for links in by_target.values():
        links.sort(key=lambda link: link[0], reverse=True)
        stale.extend(f for _, f in links[KEEP_PER_TARGET:])
    return stale


def executables(debug: Path):
    """Every linked Mach-O whose debuginfo could point at a loose object:
    the test/bin executables and proc-macro dylibs in deps/, the build
    scripts under build/, and the bins cargo hardlinks to the top."""
    deps = debug / "deps"
    for f in deps.iterdir():
        if f.is_file() and (f.suffix == ".dylib" or (not f.name.endswith(KNOWN_NON_EXE) and os.access(f, os.X_OK))):
            yield f
    build = debug / "build"
    if build.is_dir():
        for d in build.iterdir():
            for f in d.glob("build_script_*"):
                if f.is_file() and os.access(f, os.X_OK):
                    yield f
    for f in debug.iterdir():
        if f.is_file() and os.access(f, os.X_OK) and not f.name.startswith("."):
            yield f


def objects_of(exe: Path) -> list:
    try:
        out = subprocess.run(["nm", "-ap", str(exe)], capture_output=True, text=True, timeout=120).stdout
    except (subprocess.SubprocessError, OSError):
        return []
    found = []
    for line in out.splitlines():
        if " OSO " not in line:
            continue
        path = re.sub(r"\(.*\)$", "", line.split(" OSO ", 1)[1].strip())
        if path.endswith(".o"):
            found.append(os.path.realpath(path))
    return found


def referenced_objects(debug: Path, skip=frozenset(), remember=True) -> set:
    """Every loose object a remaining executable names. `nm` reads only the
    executables linked since the last prune; the rest come from the list
    kept beside deps/, keyed by path, size and mtime."""
    kept = debug / OBJECTS_KEPT
    try:
        known = json.loads(kept.read_text())
    except (OSError, ValueError):
        known = {}
    now, unread = {}, []
    for exe in executables(debug):
        if exe in skip:
            continue
        try:
            st = exe.stat()
        except OSError:
            continue
        was = known.get(str(exe))
        if isinstance(was, list) and was[:2] == [st.st_size, st.st_mtime_ns]:
            now[str(exe)] = was
        else:
            unread.append((exe, st))
    with ThreadPoolExecutor(max_workers=SCANS_AT_ONCE) as pool:
        for (exe, st), found in zip(unread, pool.map(objects_of, (exe for exe, _ in unread))):
            now[str(exe)] = [st.st_size, st.st_mtime_ns, found]
    if remember:
        try:
            kept.write_text(json.dumps(now))
        except OSError:
            pass
    return {path for entry in now.values() for path in entry[2]}


def take_build_locks(debug: Path, wait: float):
    """Every cargo lock in the profile dir, exclusively, or None after `wait`
    seconds. One attempt takes all of them or none, so a build that holds
    one is never kept waiting on another this holds. Saying it waits is what
    tells cache maintenance from a test that is running (T-830)."""
    paths = sorted(debug.glob(".cargo*lock")) or [debug / ".cargo-lock"]
    deadline = time.monotonic() + wait
    said = False
    while True:
        held = []
        try:
            for path in paths:
                fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o644)
                held.append(fd)
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return held
        except BlockingIOError:
            for fd in held:
                os.close(fd)
        if time.monotonic() >= deadline:
            return None
        if not said:
            print(f"prune-deps: waiting up to {wait:g} s for cargo's build lock on {debug}", flush=True)
            said = True
        time.sleep(0.5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target-dir", default="target/debug", help="the profile dir holding deps/ (default: target/debug)")
    parser.add_argument("--dry-run", action="store_true", help="count, delete nothing")
    parser.add_argument("--quiet", action="store_true", help="print only when something was pruned or is worth advising")
    parser.add_argument("--wait", type=float, default=30, help="seconds to wait for cargo's build locks before stepping aside (default: 30)")
    args = parser.parse_args()
    debug = Path(args.target_dir).resolve()
    deps = debug / "deps"
    if not deps.is_dir():
        if not args.quiet:
            print(f"prune-deps: no {deps}; nothing to do")
        return 0
    # Cargo's build locks: a link in progress reads its objects from deps/,
    # and this must not run under it. Wait a little, then step aside.
    locks = take_build_locks(debug, args.wait)
    if locks is None:
        print(f"prune-deps: cargo holds the build lock on {debug}; not pruning this time", file=sys.stderr)
        return 0
    try:
        # deps/'s own mtime moves with every entry added or removed, so one
        # that has not moved since the last prune has nothing new to scan:
        # a run that linked nothing skips minutes of `nm` (T-830).
        stamp = debug / ".msmn-pruned"
        mark = str(deps.stat().st_mtime_ns)
        try:
            if stamp.read_text().strip() == mark:
                if not args.quiet:
                    print("prune-deps: deps/ unchanged since the last prune; nothing to do")
                return 0
        except OSError:
            pass
        stale = stale_executables(deps)
        stale_bytes = 0
        for f in stale:
            try:
                stale_bytes += f.stat().st_size
                if not args.dry_run:
                    f.unlink()
                    f.with_name(f.name + ".d").unlink(missing_ok=True)
            except OSError:
                pass
        refs = referenced_objects(debug, skip=set(stale) if args.dry_run else set(), remember=not args.dry_run)
        loose = [f for f in deps.iterdir() if f.suffix == ".o" and f.is_file()]
        orphans = [f for f in loose if os.path.realpath(f) not in refs]
        freed = 0
        for f in orphans:
            try:
                freed += f.stat().st_size
                if not args.dry_run:
                    f.unlink()
            except OSError:
                pass
        entries = sum(1 for _ in deps.iterdir())
        if not args.dry_run:
            stamp.write_text(str(deps.stat().st_mtime_ns) + "\n")
        verb = "would prune" if args.dry_run else "pruned"
        if stale or orphans or not args.quiet:
            print(
                f"prune-deps: {verb} {len(stale)} stale executables ({stale_bytes / 1048576:.0f} MB) and "
                f"{len(orphans)} orphaned objects ({freed / 1048576:.0f} MB); "
                f"{len(loose) - len(orphans)} referenced by {len(refs) and 'current binaries' or 'nothing'} kept; "
                f"deps/ now {entries} entries",
                flush=True,
            )
        if entries > ADVISE_CLEAN_AT:
            print(
                f"prune-deps: deps/ still holds {entries} entries — a freshly linked binary's first exec "
                f"is held by macOS in proportion to that; `cargo clean` (rebuild ~25 s) is the fix",
                flush=True,
            )
    finally:
        for lock in locks:
            fcntl.flock(lock, fcntl.LOCK_UN)
            os.close(lock)
    return 0


if __name__ == "__main__":
    sys.exit(main())
