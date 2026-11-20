"""Carry finished worktree release binaries and the runner's stamp across a fast-forward."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
from zipfile import BadZipFile, ZipFile

STAMP = Path("target/suite-passed.json")
TARGETS = ("aarch64-apple-darwin", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl")
BINARIES = [f"{target}/release/mesimon" for target in TARGETS]


def check_stamp(data, sha, tmux):
    stamp = json.loads(data)
    if (stamp["sha"] != sha or stamp["coverage"] != "full-workspace-v2"
            or stamp["roads"] != ["hooks", "mod"] or stamp["tmux"] != str(tmux.resolve())
            or stamp["tmux_sha256"] != hashlib.sha256(tmux.read_bytes()).hexdigest()):
        raise ValueError("the two-road stamp does not match HEAD and the bundled tmux")


def handoff(action, bundle, sha, tmux, *dest):
    if action == "check-stamp":
        check_stamp(bundle.read_bytes(), sha, tmux)
        return
    if action == "save":
        data = STAMP.read_bytes()
        check_stamp(data, sha, tmux)
        bundle.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=bundle.parent) as stage:
            pending = Path(stage) / bundle.name
            with ZipFile(pending, "w") as archive:
                for name in BINARIES:
                    archive.write(STAMP.parent / name, name)
                archive.writestr(STAMP.name, data)
            os.replace(pending, bundle)
        print(f"Prepared worktree binaries and suite stamp at {bundle}")
        return

    with ZipFile(bundle) as archive:
        data = archive.read(STAMP.name)
        check_stamp(data, sha, tmux)
        if set(archive.namelist()) != {*BINARIES, STAMP.name}:
            raise ValueError("incomplete worktree release bundle")
        if action == "activate":
            # Discover damaged binaries before ship's preflight accepts the stamp.
            if bad := archive.testzip():
                raise ValueError(f"checksum failed for {bad}")
            STAMP.parent.mkdir(exist_ok=True)
            with tempfile.TemporaryDirectory(dir=STAMP.parent) as stage:
                pending = Path(stage) / STAMP.name
                pending.write_bytes(data)
                os.replace(pending, STAMP)
            print(f"Activated the worktree's suite stamp and verified all three binaries for {sha}")
        elif action == "unpack":
            for name in BINARIES:
                binary = Path(dest[0]) / name
                binary.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(name) as src, binary.open("wb") as dst:
                    shutil.copyfileobj(src, dst)
                binary.chmod(0o755)
            print(f"Using prepared worktree binaries for {sha}")
        else:
            raise ValueError(f"unknown action {action}")


if __name__ == "__main__":
    action, path, sha, tmux, *dest = sys.argv[1:]
    try:
        handoff(action, Path(path), sha, Path(tmux), *dest)
    except (BadZipFile, KeyError, OSError, ValueError) as error:
        sys.exit(f"Release handoff: {error}")
