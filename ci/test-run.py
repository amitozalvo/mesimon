#!/usr/bin/env python3
"""One bounded local/CI verification command, with a checked fixture audit.

Default: cargo nextest run --workspace (cargo's own parallelism; --jobs N caps
build jobs and test threads together).
Other gates: python3 ci/test-run.py -- cargo test -p mesimon --test hook_e2e.
Python 3 is a test dependency only; the shipped binary does not use it.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=int, default=1200, help="whole command deadline in seconds")
    parser.add_argument("--jobs", type=int, help="cap cargo build jobs and test threads (default: cargo's own)")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("timeout must be positive")
    if args.jobs is not None and args.jobs < 1:
        parser.error("jobs must be positive")
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        command = ["cargo", "nextest", "run", "--workspace"]
        if args.jobs:
            command += ["--test-threads", str(args.jobs)]
    os.umask(0o077)
    lock_path = f"/tmp/msmn-test-command-{os.getuid()}.lock"
    lock = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print("Another bounded Mesimon check is running; refusing overlapping workloads.", file=sys.stderr)
        os.close(lock)
        return 2
    run = Path(tempfile.mkdtemp(prefix="msmn-test-run-", dir="/tmp")).resolve()
    env = dict(os.environ, MESIMON_TEST_RUN=str(run))
    if args.jobs:
        env.update(CARGO_BUILD_JOBS=str(args.jobs), RUST_TEST_THREADS=str(args.jobs))
    env.setdefault("MESIMON_REQUIRE_TMUX", "1")
    guard = subprocess.Popen(
        [sys.executable, "-B", "-u", str(Path(__file__).with_name("test_guard.py")),
         "--name", "command", "--tmux", env.get("MESIMON_TMUX_BIN", "tmux"),
         "--timeout", str(args.timeout)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env,
    )

    def request(value):
        guard.stdin.write(json.dumps(value) + "\n")
        guard.stdin.flush()
        line = guard.stdout.readline()
        if not line:
            raise RuntimeError("command supervisor exited (deadline or cleanup failure)")
        reply = json.loads(line)
        if "error" in reply:
            raise RuntimeError(reply["error"])
        return reply["ok"]

    interrupted = []
    def stop(signum, _frame):
        interrupted.append(signum)
        # Closing the only control pipe invokes supervisor cleanup even if
        # this runner subsequently receives SIGKILL.
        guard.stdin.close()
    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGINT, signal.SIGTERM)}
    result = 1
    log = None
    try:
        root = Path(json.loads(guard.stdout.readline())["ok"])
        pid = request(dict(op="spawn", argv=command, env=env))
        log = open(root / "child-0.log")
        workers = f", {args.jobs} build/test workers" if args.jobs else ""
        print(f"Bounded check ({args.timeout}s deadline{workers}); audit: {run}", flush=True)
        while not interrupted:
            output = log.read()
            if output:
                print(output, end="", flush=True)
            code = request(dict(op="poll", pid=pid))
            if code is not None:
                result = 0 if code == 0 else 1
                break
            time.sleep(0.2)
        if interrupted:
            result = 128 + interrupted[0]
    except (RuntimeError, ValueError, OSError) as exc:
        print(str(exc), file=sys.stderr)
        result = 1
    finally:
        if not guard.stdin.closed:
            try:
                guard.stdin.write('{"op":"finish"}\n')
                guard.stdin.flush()
            except BrokenPipeError:
                pass
            guard.stdin.close()
        try:
            if guard.wait(timeout=25) != 0:
                result = result or 1
        except subprocess.TimeoutExpired:
            print("Cleanup supervisor still running; retained audit registry.", file=sys.stderr)
            result = 1
        guard.stdout.close()
        if log:
            print(log.read(), end="", flush=True)
            log.close()
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        # Independent fixture owners need a short bounded drain after runner
        # cancellation. An active/failed manifest is never an implied pass.
        deadline = time.monotonic() + 20
        while True:
            manifests = [json.loads(p.read_text()) for p in run.glob("*.json")]
            if not any(m["status"] == "active" for m in manifests) or time.monotonic() >= deadline:
                break
            time.sleep(0.1)
        bad = [m for m in manifests if m["status"] != "cleaned"]
        if bad:
            result = result or 1
            print(f"Fixture audit FAILED ({len(bad)} incomplete/abnormal owners): {run}", file=sys.stderr)
        elif result:
            print(f"Fixture audit clean ({len(manifests)} owners). Registry kept: {run}", flush=True)
        else:
            # Evidence is for a failure; a clean pass leaves nothing in /tmp.
            shutil.rmtree(run, ignore_errors=True)
            print(f"Fixture audit clean ({len(manifests)} owners).", flush=True)
        os.close(lock)
    return result


if __name__ == "__main__":
    sys.exit(main())
