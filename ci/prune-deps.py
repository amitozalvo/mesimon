#!/usr/bin/env python3
"""Delete the split-debuginfo objects in target/debug/deps that no current
binary references.

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
build lock (`target/debug/.cargo-lock`), so it cannot delete an object a
link in progress is about to read.

    python3 ci/prune-deps.py            # prune, print what went
    python3 ci/prune-deps.py --dry-run  # count only

ci/test-run.py runs it after every bounded check. Python 3 is a test
dependency only; nothing shipped uses it.
"""
import argparse
import fcntl
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


def referenced_objects(debug: Path) -> set:
    refs = set()
    for exe in executables(debug):
        try:
            out = subprocess.run(["nm", "-ap", str(exe)], capture_output=True, text=True, timeout=120).stdout
        except (subprocess.SubprocessError, OSError):
            continue
        for line in out.splitlines():
            if " OSO " not in line:
                continue
            path = re.sub(r"\(.*\)$", "", line.split(" OSO ", 1)[1].strip())
            if path.endswith(".o"):
                refs.add(os.path.realpath(path))
    return refs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target-dir", default="target/debug", help="the profile dir holding deps/ (default: target/debug)")
    parser.add_argument("--dry-run", action="store_true", help="count, delete nothing")
    parser.add_argument("--quiet", action="store_true", help="print only when something was pruned or is worth advising")
    args = parser.parse_args()
    debug = Path(args.target_dir).resolve()
    deps = debug / "deps"
    if not deps.is_dir():
        if not args.quiet:
            print(f"prune-deps: no {deps}; nothing to do")
        return 0
    # Cargo's build lock: a link in progress reads its objects from deps/,
    # and this must not run under it. Wait a little, then step aside.
    lock_path = debug / ".cargo-lock"
    lock = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o644)
    deadline = time.monotonic() + 30
    while True:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            if time.monotonic() >= deadline:
                print("prune-deps: cargo holds the build lock; not pruning this time", file=sys.stderr)
                os.close(lock)
                return 0
            time.sleep(0.5)
    try:
        refs = referenced_objects(debug)
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
        verb = "would prune" if args.dry_run else "pruned"
        if orphans or not args.quiet:
            print(
                f"prune-deps: {verb} {len(orphans)} orphaned objects ({freed / 1048576:.0f} MB); "
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
        fcntl.flock(lock, fcntl.LOCK_UN)
        os.close(lock)
    return 0


if __name__ == "__main__":
    sys.exit(main())
