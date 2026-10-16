#!/usr/bin/env python3
"""One bounded local/CI verification command, with a checked fixture audit.

Default: cargo nextest run --workspace (cargo's own parallelism; --jobs N caps
build jobs and test threads together). A full-workspace suite is followed by
the mod road's pass (T-574): the `mesimon` crate's integration tests again
under MESIMON_TEST_ROAD=mod, the stand-in engine beside every stub; a stamp
needs both. --one-road skips it.
Other gates: python3 ci/test-run.py -- cargo test -p mesimon --test hook_e2e.
Runs in independent checkouts overlap (T-830): a run holds a lock on each
Cargo build directory it writes (target dir and build-dir, however
configured), so only runs sharing build output take turns, and the
waiting run says whose turn it is. --exclusive (the release gate) waits for
every other bounded run and holds new ones off.
Requires 5 GiB free on build and fixture/state volumes, checked every second;
low space stops this workload through its cleanup supervisor. --min-free-gib
changes the reserve (0 disables it). This is not a filesystem quota.
Python 3 is a test dependency only; the shipped binary does not use it.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def build_output(command, env):
    """The Cargo build directories this run writes: the target directory and,
    when configured apart, the build directory (`build.build-dir`), read the
    way cargo reads them (config, environment, an explicit --target-dir).
    A command that is not cargo gets its checkout's, when it is in one: a
    script may build (the rig does). Paths are resolved, so a symlinked or
    /tmp-vs-/private/tmp spelling of one directory is one directory."""
    metadata = ["cargo", "metadata", "--offline", "--no-deps", "--format-version=1"]
    target = None
    if command[:1] == ["cargo"]:
        # Metadata reads Cargo's target-dir config and environment. An explicit
        # build --target-dir wins; metadata itself does not accept that flag.
        args = iter(command[1:])
        for arg in args:
            if arg == "--":
                break
            if arg in ("--manifest-path", "--config", "--target-dir"):
                value = next(args, None)
                if value is None:
                    raise ValueError(f"missing value for {arg}")
                if arg == "--target-dir":
                    target = value
                else:
                    metadata.extend([arg, value])
            elif arg.startswith("--target-dir="):
                target = arg.split("=", 1)[1]
            elif arg.startswith(("--manifest-path=", "--config=")):
                metadata.append(arg)
        result = subprocess.run(metadata, env=env, capture_output=True, text=True,
                                check=True, timeout=30)
    else:
        try:
            result = subprocess.run(metadata, env=env, capture_output=True, text=True,
                                    check=True, timeout=30)
        except (OSError, subprocess.SubprocessError):
            return []
    info = json.loads(result.stdout)
    dirs = [Path(target or info["target_directory"]).resolve()]
    # Newer Cargo can place intermediate artifacts on a separate volume.
    if info.get("build_directory"):
        build = Path(info["build_directory"]).resolve()
        if build not in dirs:
            dirs.append(build)
    return dirs


def disk_paths(command, env, output=None):
    """Watch build output and fixture/state volumes, including Cargo config."""
    paths = [Path.cwd(), Path(tempfile.gettempdir()),
             Path(env.get("MESIMON_TEST_AUDIT_ROOT", "/tmp")),
             Path.home() / ".local/state/mesimon"]
    return paths + (build_output(command, env) if output is None else output)


def check_disk_space(paths, minimum_gib):
    """Check existing ancestors without creating output dirs or deleting files."""
    checked = set()
    for path in paths:
        path = path.resolve()
        while not path.exists() and path != path.parent:
            path = path.parent
        device = path.stat().st_dev
        if device in checked:
            continue
        checked.add(device)
        free = shutil.disk_usage(path).free / 2**30
        if free < minimum_gib:
            raise RuntimeError(
                f"Low disk space at {path}: {free:.1f} GiB free; "
                f"this check requires {minimum_gib:g} GiB. "
                "Free unused build output before retrying. No build caches were deleted.")


def lock_dir(env):
    """Where bounded runs coordinate: private to this user, like the audit
    root. MESIMON_TEST_LOCK_DIR moves it (the runner's own tests do, so a
    test never waits on, or holds off, a real run)."""
    path = Path(env.get("MESIMON_TEST_LOCK_DIR") or f"/tmp/msmn-test-locks-{os.getuid()}")
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    st = path.lstat()
    if path.is_symlink() or not path.is_dir() or st.st_uid != os.getuid() or st.st_mode & 0o077:
        raise ValueError(f"lock directory must be private and owned: {path}")
    return path


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def read_note(path):
    try:
        note = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    return note if isinstance(note, dict) and alive(note.get("pid", 0)) else None


def describe(note):
    if not note:
        return "another run"
    phase = PHASE_WORDS.get(note.get("phase"), note.get("phase"))
    # A clock time, not a running count, so the words change only when the
    # holder or its phase does, and the waiting line is said once per change.
    since = time.strftime("%H:%M:%S", time.localtime(note.get("phase_since", time.time())))
    return (f"pid {note['pid']} in {note.get('cwd')} ({phase} since {since}): "
            f"{' '.join(note.get('command', []))}")


# What a waiting run is told the holder is doing: the phases that are not
# tests are named as what they are (T-830).
PHASE_WORDS = {
    "starting": "starting",
    "draining": "waiting for the runs before it to finish",
    "tests": "running tests",
    "mod road": "running the mod road's pass",
    "audit": "auditing its fixtures",
    "pruning": "pruning the build cache",
}


class Claims:
    """The locks one bounded run holds, and the notes that tell a waiting
    run who holds them and in which phase.

    Every run holds `machine.lock` shared and an exclusive run holds it
    exclusively; `machine.gate` is the turnstile that keeps a stream of
    shared runs from starving an exclusive one. Then one exclusive lock per
    build directory, in sorted order, so two runs never hold one each of
    the other's. A lock an enclosing run already holds (MESIMON_TEST_HELD)
    is not taken twice: a run inside a run would otherwise wait on its
    parent until the deadline."""

    def __init__(self, directory, command, held):
        self.dir = directory
        self.command = command
        self.inherited = set(held)
        self.fds = []
        self.notes = []
        self.keys = []
        self.phase_name = "starting"
        self.since = time.time()

    def note(self, path):
        self.notes.append(path)
        self.write(path)

    def write(self, path):
        body = dict(pid=os.getpid(), cwd=os.getcwd(), command=self.command,
                    phase=self.phase_name, phase_since=self.since)
        tmp = path.with_name(path.name + f".{os.getpid()}.tmp")
        tmp.write_text(json.dumps(body) + "\n")
        tmp.replace(path)

    def phase(self, name):
        self.phase_name = name
        self.since = time.time()
        for path in self.notes:
            self.write(path)

    def open(self, name):
        fd = os.open(self.dir / name, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        self.fds.append(fd)
        return fd

    def wait(self, fd, mode, deadline, waiting_for):
        """flock, saying whom it waits for whenever that changes."""
        said = None
        start = time.monotonic()
        while True:
            try:
                fcntl.flock(fd, mode | fcntl.LOCK_NB)
                if said:
                    print(f"== waited {time.monotonic() - start:.0f} s; starting", flush=True)
                return
            except BlockingIOError:
                pass
            words = waiting_for()
            if words != said:
                print(f"== waiting (not testing yet): {words}", flush=True)
                said = words
            if time.monotonic() >= deadline:
                raise TimeoutError(f"still waiting after {time.monotonic() - start:.0f} s: {words}")
            time.sleep(LOCK_POLL)

    def shared_holders(self):
        notes = [read_note(p) for p in self.dir.glob("machine-*.json")]
        notes = [n for n in notes if n and n["pid"] != os.getpid()]
        if not notes:
            return "other bounded runs to finish"
        more = f" and {len(notes) - 1} more" if len(notes) > 1 else ""
        return f"every other bounded run to finish: {describe(notes[0])}{more}"

    def take(self, output, exclusive, deadline):
        if "machine" not in self.inherited:
            gate, machine = self.open("machine.gate"), self.open("machine.lock")
            exclusive_note = self.dir / "machine-exclusive.json"
            holder = lambda: f"an exclusive run (the release gate): {describe(read_note(exclusive_note))}"
            if exclusive:
                self.wait(gate, fcntl.LOCK_EX, deadline, holder)
                # Holding the gate already holds new runs off: say so to them.
                self.phase_name = "draining"
                self.note(exclusive_note)
                self.wait(machine, fcntl.LOCK_EX, deadline, self.shared_holders)
                self.phase("starting")
            else:
                self.wait(gate, fcntl.LOCK_SH, deadline, holder)
                self.wait(machine, fcntl.LOCK_SH, deadline, holder)
                fcntl.flock(gate, fcntl.LOCK_UN)
                self.note(self.dir / f"machine-{os.getpid()}.json")
            self.keys.append("machine")
        for path in sorted(output):
            key = "build-" + hashlib.sha256(os.fsencode(path)).hexdigest()[:16]
            if key in self.inherited or key in self.keys:
                continue
            fd = self.open(key + ".lock")
            note = self.dir / f"{key}.json"
            self.wait(fd, fcntl.LOCK_EX, deadline,
                      lambda path=path, note=note: f"build output {path} is in use by {describe(read_note(note))}")
            self.note(note)
            self.keys.append(key)

    def env(self):
        """For the run's children: what this run and its parents hold."""
        return ",".join(sorted(self.inherited | set(self.keys)))

    def release(self):
        # Notes go while their locks are still held, so a note never
        # outlives its holder and is never one the next holder just wrote.
        for path in self.notes:
            try:
                path.unlink()
            except OSError:
                pass
        self.notes = []
        for fd in self.fds:
            os.close(fd)
        self.fds = []


LOCK_POLL = 0.5


def road_pass(command, jobs):
    """The mod road's pass after a full-workspace suite (T-574), or None.

    The e2e suite again with every Claude launch on the mod road: the stub
    wrapped so the stand-in engine and the real `mesimon mod-bridge` run
    beside it, and every hook frame sent with its mod twin first, so the
    shadow pairs the whole suite's frames."""
    if command[:1] != ["cargo"] or "--workspace" not in command:
        return None
    if command[1:3] == ["nextest", "run"]:
        mod = ["cargo", "nextest", "run", "-p", "mesimon", "-E", "kind(test)"]
        if jobs:
            mod += ["--test-threads", str(jobs)]
        return mod
    if command[1:2] == ["test"]:
        return ["cargo", "test", "-p", "mesimon", "--tests"]
    return None


def stamp_pass(command, env, roads, target=None):
    """Record a clean FULL-workspace run so ci/release.sh can accept it as its
    gate instead of running the same suite a second time, serially, paying
    macOS's first-exec hold on every freshly linked e2e binary (~25 s each,
    36 of them — alpha.16's gate spent ~25 minutes on 2.5 minutes of tests).

    The stamp names what the run proved and nothing more: the commit, that
    the tree was clean (so the commit IS what ran), and which tmux drove it,
    by path and by hash — the release honours it only for the bundled one.
    A partial command (one crate, one e2e) never stamps."""
    if "--workspace" not in command:
        return
    if not (command[:1] == ["cargo"] and command[1:2] and command[1] in ("test", "nextest")):
        return
    try:
        sha = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
        dirty = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, check=True).stdout
    except (subprocess.CalledProcessError, OSError):
        return
    if dirty:
        print("Suite passed on a dirty tree: not stamped (the commit is not what ran).", flush=True)
        return
    tmux = shutil.which(env.get("MESIMON_TMUX_BIN", "tmux"))
    if not tmux:
        return
    tmux = str(Path(tmux).resolve())
    digest = hashlib.sha256(Path(tmux).read_bytes()).hexdigest()
    # Linux mounts the checkout read-only and builds in CARGO_TARGET_DIR.
    stamp = Path(target or env.get("CARGO_TARGET_DIR") or "target") / "suite-passed.json"
    stamp.parent.mkdir(parents=True, exist_ok=True)
    stamp.write_text(json.dumps(dict(sha=sha, tmux=tmux, tmux_sha256=digest, roads=roads,
                                     at=int(time.time())), indent=2) + "\n")
    print(f"Suite passed at {sha[:7]} against {tmux}; stamped {stamp}.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=int, default=1200, help="whole command deadline in seconds")
    parser.add_argument("--jobs", type=int, help="cap cargo build jobs and test threads (default: cargo's own)")
    parser.add_argument("--min-free-gib", type=int, default=5,
                        help="stop below this free disk reserve (default: 5 GiB; 0 disables)")
    parser.add_argument("--one-road", action="store_true",
                        help="skip the mod road's pass after a full-workspace suite (T-574)")
    parser.add_argument("--exclusive", action="store_true",
                        help="wait for every other bounded run to end and hold new ones off (the release gate)")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("timeout must be positive")
    if args.jobs is not None and args.jobs < 1:
        parser.error("jobs must be positive")
    if args.min_free_gib < 0:
        parser.error("min-free-gib must be nonnegative")
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        command = ["cargo", "nextest", "run", "--workspace"]
        if args.jobs:
            command += ["--test-threads", str(args.jobs)]
    passes = [(command, {})]
    mod = None if args.one_road else road_pass(command, args.jobs)
    if mod:
        passes.append((mod, {"MESIMON_TEST_ROAD": "mod"}))
    os.umask(0o077)
    held = [k for k in os.environ.get("MESIMON_TEST_HELD", "").split(",") if k]
    claims = Claims(None, command, held)
    try:
        build_dirs = build_output(command, os.environ)
        watched_paths = disk_paths(command, os.environ, build_dirs) if args.min_free_gib else []
        check_disk_space(watched_paths, args.min_free_gib)
        # Only runs that write the same build output take turns (T-830);
        # waiting is bounded by the run's own deadline and spends none of it.
        claims.dir = lock_dir(os.environ)
        claims.take(build_dirs, args.exclusive, time.monotonic() + args.timeout)
        check_disk_space(watched_paths, args.min_free_gib)
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as exc:
        claims.release()
        print(f"Check not started: {exc}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        claims.release()
        print("Check not started: interrupted while waiting", file=sys.stderr)
        return 128 + signal.SIGINT
    try:
        return run_claimed(args, command, passes, build_dirs, watched_paths, claims)
    finally:
        claims.release()


def run_claimed(args, command, passes, build_dirs, watched_paths, claims):
    # Containers can retain audit metadata on their build volume even after
    # their process namespace and temporary fixture directories disappear.
    audit_root = Path(os.environ.get("MESIMON_TEST_AUDIT_ROOT", "/tmp"))
    audit_root.mkdir(parents=True, exist_ok=True, mode=0o700)
    run = Path(tempfile.mkdtemp(prefix="msmn-test-run-", dir=audit_root)).resolve()
    env = dict(os.environ, MESIMON_TEST_RUN=str(run), MESIMON_TEST_HELD=claims.env())
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
        workers = f", {args.jobs} build/test workers" if args.jobs else ""
        print(f"Bounded check ({args.timeout}s deadline{workers}); audit: {run}", flush=True)
        if build_dirs:
            print(f"Build output held by this run: {', '.join(map(str, build_dirs))}", flush=True)
        last_disk_check = time.monotonic()
        # One supervisor and one deadline for every pass; a failing pass ends
        # the run.
        for index, (argv, extra) in enumerate(passes):
            if index:
                claims.phase("mod road")
                print(f"\n== the mod road's pass (T-574): {' '.join(argv)}", flush=True)
            else:
                claims.phase("tests")
                print(f"== tests: {' '.join(argv)}", flush=True)
            pid = request(dict(op="spawn", argv=argv, env=dict(env, **extra)))
            if log:
                log.close()
            log = open(root / f"child-{index}.log")
            code = None
            while not interrupted:
                output = log.read()
                if output:
                    print(output, end="", flush=True)
                code = request(dict(op="poll", pid=pid))
                if code is not None:
                    result = 0 if code == 0 else 1
                    check_disk_space(watched_paths, args.min_free_gib)
                    break
                if time.monotonic() - last_disk_check >= 1:
                    check_disk_space(watched_paths, args.min_free_gib)
                    last_disk_check = time.monotonic()
                time.sleep(0.2)
            if interrupted or result:
                break
            output = log.read()
            if output:
                print(output, end="", flush=True)
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
            try:
                guard.stdin.close()
            except BrokenPipeError:
                pass
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
        claims.phase("audit")
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
            stamp_pass(command, env, ["hooks", "mod"] if len(passes) > 1 else ["hooks"],
                       build_dirs[0] if build_dirs else None)
        # Every relink leaves its split-debuginfo objects behind and cargo
        # collects none of them; a week of that made deps/ 879k entries and
        # every fresh binary's first exec a 25 s Gatekeeper walk. Pruned
        # here, after each run, under cargo's own locks (ci/prune-deps.py),
        # in the build directory this run used. Still under this run's lock
        # on it, which holds off only runs that share it: they would wait on
        # cargo's lock all the same, and this way they are told it is
        # pruning (T-830).
        # This pruner interprets Mach-O OSO entries. It cannot establish live
        # references for ELF objects, and Linux builds may mount /work read-only.
        if sys.platform == "darwin" and build_dirs and (build_dirs[-1] / "debug/deps").is_dir():
            claims.phase("pruning")
            profile = build_dirs[-1] / "debug"
            print(f"== pruning the build cache in {profile} (the tests are done)", flush=True)
            subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("prune-deps.py")),
                            "--quiet", "--target-dir", str(profile)], check=False)
    return result


if __name__ == "__main__":
    sys.exit(main())
