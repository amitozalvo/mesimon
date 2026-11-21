#!/usr/bin/env python3
"""One bounded local/CI verification command, with a checked fixture audit.

Default: cargo nextest run --workspace (cargo's own parallelism; --jobs N caps
build jobs and test threads together). Checks run once by default. --both-roads
adds the mod road's pass to a recognized full-workspace suite: the `mesimon`
integration tests under MESIMON_TEST_ROAD=mod, with a stand-in engine.
Nextest skips exact cases audited as independent in mod-road-independent.txt. The two
passes run side by side (T-1101): the mod pass waits on cargo's own lock while
the first pass builds, then both run their tests at once. The --jobs cap applies
to each road when supplied. Both roads take about one road's time (the e2es wait on their own
timers and barely use the CPU, T-1023). Release verification requests both
explicitly; a one-road stamp is not a release gate. --one-road remains an
explicit spelling of the default for existing callers. The deadline is 1200 s
for one pass and 2400 s for both unless --timeout says otherwise.
Other gates: python3 ci/test-run.py -- cargo test -p mesimon --test agents hook_e2e::.
Runs in independent checkouts overlap (T-830): a run holds a lock on each
Cargo build directory it writes (target dir and build-dir, however
configured), so only runs sharing build output take turns, and the
waiting run says whose turn it is. --exclusive (the release gate) waits for
every other bounded run and holds new ones off. With --both-roads and nextest,
the exclusive gate finishes both roads and reruns each failure alone on its road.
Requires 5 GiB free on build and fixture/state volumes, checked every second;
low space stops this workload through its cleanup supervisor. --min-free-gib
changes the reserve (0 disables it). This is not a filesystem quota.
Python 3 is a test dependency only; the shipped binary does not use it.
"""
import argparse
import contextlib
import fcntl
import hashlib
import itertools
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET


def build_output(command, env):
    """The Cargo build directories this run writes: the target directory and,
    when configured apart, the build directory (`build.build-dir`), read the
    way cargo reads them (config, environment, an explicit --target-dir).
    Other commands own no Cargo output unless their caller declares it."""
    if command[:1] != ["cargo"]:
        return []
    metadata = ["cargo", "metadata", "--offline", "--no-deps", "--format-version=1"]
    target = None
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


class Tee:
    def __init__(self, console, log):
        self.console = console
        self.log = log

    def write(self, text):
        self.log.write(text)
        return self.console.write(text)

    def flush(self):
        self.log.flush()
        self.console.flush()


class RunRecord:
    """A check publishes its own result. Replacements only leave it a message."""
    def __init__(self, directory):
        self.root = Path(tempfile.mkdtemp(prefix="check-", dir=directory)).resolve()
        self.path = self.root / "result.json"
        self.data = dict(started_at=time.time(), exited_at=None, result=None,
                         log=str(self.root / "run.log"))
        self.active = None

    def save(self):
        tmp = self.path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.data) + "\n")
        tmp.replace(self.path)

    @contextlib.contextmanager
    def registry(self):
        # Only the handoff itself is serialized. Never hold this during a check.
        with (self.root.parent / "registration.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            yield

    def register(self, signature):
        digest = hashlib.sha256(json.dumps(signature, sort_keys=True).encode()).hexdigest()
        self.active = self.root.parent / f"active-{digest}"
        with self.registry():
            if self.active.is_symlink():
                prior = self.active.resolve()
                prior_data = json.loads(prior.read_text()) if prior.is_file() else None
                if prior_data and prior_data["result"] is None:
                    # Metadata for an older launch can finish after a newer one.
                    if prior_data["started_at"] > self.data["started_at"]:
                        (self.root / "superseded").symlink_to(prior)
                        raise Superseded(f"Check superseded by {prior}")
                    handoff = self.root / "handoff"
                    handoff.symlink_to(self.path)
                    handoff.replace(prior.parent / "superseded")
                    print(f"== superseding previous check: {prior}", flush=True)
                self.active.unlink()
            self.active.symlink_to(self.path)

    def retire(self):
        if self.active:
            with self.registry():
                if self.active.resolve() == self.path:
                    self.active.unlink()
                self.active = None
            successor = self.root / "superseded"
            if successor.is_symlink():
                self.data["superseded_by"] = os.readlink(successor)

    def check_superseded(self):
        try:
            successor = os.readlink(self.root / "superseded")
        except FileNotFoundError:
            return
        raise Superseded(f"Check superseded by {successor}")


class Superseded(RuntimeError):
    pass


def wait_for_record(path, timeout):
    # The runner holds the log lock from publication through its final write.
    # flock also wakes on a dead runner. Neither case needs a polling process.
    def expired(_sig, _frame):
        raise TimeoutError

    with path.with_name("run.log").open() as log:
        previous = signal.signal(signal.SIGALRM, expired)
        signal.alarm(timeout)
        try:
            fcntl.flock(log, fcntl.LOCK_SH)
            ended = True
        except TimeoutError:
            ended = False
        finally:
            signal.alarm(0)
            signal.signal(signal.SIGALRM, previous)
        data = json.loads(path.read_text())
        if ended and data["result"] is None:
            raise RuntimeError(f"Runner exited without a result: {path}")
        print(json.dumps(data))
        return data["result"] if data["result"] is not None else 124


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
    "both roads": "running both roads' passes",
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

    def __init__(self, directory, command, held, record=None):
        self.dir = directory
        self.command = command
        self.inherited = set(held)
        self.fds = []
        self.notes = []
        self.keys = []
        self.phase_name = "starting"
        self.since = time.time()
        self.record = record

    def check_superseded(self):
        if self.record and self.record.active:
            self.record.check_superseded()

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
            self.check_superseded()
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


def full_workspace_suite(command):
    """Recognize an unfiltered executing suite, conservatively.

    Unknown options still run as requested, but cannot expand a targeted
    check or certify release coverage. In particular --workspace alone says
    nothing about test filters, target selection, listing or compilation.
    """
    if command[:3] == ["cargo", "nextest", "run"]:
        words = command[3:]
    elif command[:2] == ["cargo", "test"]:
        words = command[2:]
    else:
        return False
    switches = {"--workspace", "--no-fail-fast", "--locked", "--offline", "--frozen",
                "--quiet", "-q", "--verbose", "-v", "-vv"}
    values = {"--jobs", "-j", "--test-threads", "--color", "--target-dir"}
    workspace = False
    index = 0
    while index < len(words):
        word = words[index]
        if word == "--":
            # Harness output/concurrency options do not narrow coverage.
            tail = words[index + 1:]
            for item in tail:
                if item not in {"--nocapture", "--show-output", "--quiet"} and not (
                        item.startswith("--test-threads=") and item.split("=", 1)[1].isdigit()):
                    return False
            break
        key, equal, value = word.partition("=")
        if key in values:
            if not equal:
                index += 1
                if index >= len(words) or words[index].startswith("-"):
                    return False
            elif not value:
                return False
        elif word in switches:
            workspace |= word == "--workspace"
        else:
            return False
        index += 1
    return workspace


MOD_INDEPENDENT = Path(__file__).with_name("mod-road-independent.txt")


def mod_filter(manifest):
    """Subtract only exact binary/test pairs. A new test or binary runs on both roads."""
    by_binary = {}
    for line in manifest.read_text().splitlines():
        entry = line.partition("#")[0].strip()
        if not entry:
            continue
        match = re.fullmatch(r"(\w+)\s+(\w+(?:::\w+)*)", entry, re.ASCII)
        if not match:
            raise ValueError(f"Expected a binary and an exact test name in {manifest}: {entry}")
        binary, name = match.groups()
        by_binary.setdefault(binary, []).append(f"test(={name})")
    selection = "package(mesimon) & kind(test)"
    excluded = [f"(binary_id(=mesimon::{binary}) & ({' | '.join(tests)}))"
                for binary, tests in by_binary.items()]
    if excluded:
        selection += " - (" + " | ".join(excluded) + ")"
    return selection


def road_pass(command, jobs):
    """The mod road's pass after a full-workspace suite (T-574), or None.

    The harness selects the mod transport for unpinned Claude launches and
    MCP tool calls. Nextest omits cases that exercise the same path on hooks.
    A suite asked to run past its first red (--no-fail-fast) asks the same of this pass, so
    one run reports every red on both roads (T-1023)."""
    if not full_workspace_suite(command):
        return None
    if command[1:3] == ["nextest", "run"]:
        # The whole workspace, filtered, rather than `-p mesimon`: one
        # package alone unifies features differently and relinks for a
        # minute on a runner what the first pass just built (T-1023).
        mod = ["cargo", "nextest", "run", "--workspace", "-E", mod_filter(MOD_INDEPENDENT)]
        if jobs:
            mod += ["--test-threads", str(jobs)]
    elif command[1:2] == ["test"]:
        mod = ["cargo", "test", "-p", "mesimon", "--tests"]
    else:
        return None
    if keeps_going(command):
        mod.append("--no-fail-fast")
    return mod


def keeps_going(command):
    """Whether the suite was asked to run past its first red: its own
    flag, before any `--` that hands the rest to the test binaries."""
    words = command[:command.index("--")] if "--" in command else command
    return "--no-fail-fast" in words


def clean_head():
    sha = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    dirty = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, check=True).stdout
    return None if dirty else sha


def failure_report(root, road):
    """Give simultaneous nextest runs separate JUnit reports, preserving their console output."""
    report = root / f"{road}.xml"
    config = root / f"{road}.toml"
    config.write_text(f"[profile.default.junit]\npath = {json.dumps(str(report))}\n")
    return ["--tool-config-file", f"mesimon:{config}", "--retries", "0"]


def failed_tests(report):
    return [(case.attrib["classname"], case.attrib["name"])
            for case in ET.parse(report).iter("testcase")
            if case.find("failure") is not None or case.find("error") is not None]


def rerun_failures(codes, root, candidate, execute, interrupted):
    """Diagnose the red roads under the gate's existing lock and supervisor."""
    for road, code in codes.items():
        if not code:
            continue
        report = root / f"{road}.xml"
        if not report.exists():
            print(f"== {road} exited {code} without a test report; no named tests to rerun", flush=True)
            continue
        for binary, test in failed_tests(report):
            if interrupted:
                return
            if clean_head() != candidate:
                raise RuntimeError("Candidate changed before the isolated rerun")
            argv = ["cargo", "nextest", "run", "--workspace", "-E", f"binary_id(={binary})",
                    "--retries", "0", "--no-tests", "fail", "--", "--exact", test]
            label = f"{road} {binary} {test}"
            print(f"== rerunning {label} alone at {candidate}", flush=True)
            retry = execute({road: argv})
            if clean_head() != candidate:
                raise RuntimeError("Candidate changed during the isolated rerun")
            # nextest reserves exit 100 for a test failure. A build error or
            # empty selection cannot establish how the test did alone.
            status = {0: "passed-alone", 100: "failed-alone"}.get(retry.get(road), "inconclusive")
            print(f"Rerun {status} {label}", flush=True)


def stamp_pass(command, env, roads, target=None, candidate=None):
    """Record a clean FULL-workspace run so ci/release.sh can accept it as its
    gate instead of running the same suite a second time, serially, paying
    macOS's first-exec hold on every freshly linked e2e binary (~25 s each,
    36 of them — alpha.16's gate spent ~25 minutes on 2.5 minutes of tests).

    The stamp names what the run proved and nothing more: the commit, that
    the tree was clean (so the commit IS what ran), and which tmux drove it,
    by path and by hash — the release honours it only for the bundled one.
    A partial command (one crate, one e2e) never stamps."""
    if not full_workspace_suite(command):
        return
    if command[1] == "nextest" and env.get("NEXTEST_PROFILE", "default") != "default":
        return
    try:
        sha = clean_head()
    except (subprocess.CalledProcessError, OSError):
        return
    if candidate is not None and sha != candidate:
        print("Candidate changed before the release stamp could be written.", file=sys.stderr)
        return
    if sha is None:
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
    stamp.write_text(json.dumps(dict(coverage="full-workspace-v2", sha=sha, tmux=tmux, tmux_sha256=digest, roads=roads,
                                     at=int(time.time())), indent=2) + "\n")
    print(f"Suite passed at {sha[:7]} against {tmux}; stamped {stamp}.", flush=True)
    return True


class PassLog:
    """A pass's output, relayed as it is written. Passes run side by side,
    so each is relayed a whole line at a time, and every pass after the
    first carries a prefix, so the two roads' lines never mix mid-line."""

    def __init__(self, pid, file, prefix):
        self.pid = pid
        self.file = file
        self.prefix = prefix
        self.partial = ""

    def relay(self, final=False):
        text = self.partial + self.file.read()
        if final:
            lines, self.partial = text.splitlines(keepends=True), ""
        else:
            whole, sep, self.partial = text.rpartition("\n")
            lines = (whole + sep).splitlines(keepends=True)
        for line in lines:
            print(self.prefix + line, end="" if line.endswith("\n") else "\n", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=int,
                        help="whole command deadline in seconds (default: 1200, or 2400 with --both-roads)")
    parser.add_argument("--wait", type=Path, metavar="RECORD",
                        help="read the completion record, waiting up to --timeout seconds (default 30)")
    parser.add_argument("--cargo-manifest", metavar="PATH",
                        help="lock the Cargo output used by a script for this manifest")
    parser.add_argument("--jobs", type=int, help="cap cargo build jobs and test threads (default: cargo's own)")
    parser.add_argument("--min-free-gib", type=int, default=5,
                        help="stop below this free disk reserve (default: 5 GiB; 0 disables)")
    roads = parser.add_mutually_exclusive_group()
    roads.add_argument("--one-road", action="store_true", help="run once (the default)")
    roads.add_argument("--both-roads", action="store_true",
                       help="run a full workspace suite and the mod pass; required for release stamps")
    parser.add_argument("--exclusive", action="store_true",
                        help="wait for every other bounded run to end and hold new ones off (the release gate)")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout is not None and args.timeout < 1:
        parser.error("timeout must be positive")
    if args.wait:
        if args.command:
            parser.error("--wait takes a record and no command")
        try:
            return wait_for_record(args.wait, args.timeout or 30)
        except (OSError, ValueError, KeyError, RuntimeError) as exc:
            print(f"Cannot wait for check: {exc}", file=sys.stderr)
            return 2
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
    if args.cargo_manifest and command[0] == "cargo":
        parser.error("pass --manifest-path to cargo directly")
    # Both release entry points take the exclusive two-road nextest gate.
    release_gate = args.both_roads and args.exclusive and command[:3] == ["cargo", "nextest", "run"]
    if release_gate and os.environ.get("NEXTEST_PROFILE", "default") != "default":
        parser.error("the release gate requires the default nextest profile")
    if release_gate and not keeps_going(command):
        command = [*command[:3], "--no-fail-fast", *command[3:]]
    first_road = os.environ.get("MESIMON_TEST_ROAD", "hooks")
    passes = {first_road: command}
    mod = road_pass(command, args.jobs) if args.both_roads and first_road == "hooks" else None
    if args.both_roads and mod is None:
        parser.error("--both-roads needs an unfiltered full workspace suite starting on hooks; "
                     "run a targeted mod check with MESIMON_TEST_ROAD=mod instead")
    if mod:
        passes["mod"] = mod
    if args.timeout is None:
        # One road under load has taken ~1000 s (T-1099); both, side by
        # side, take about one road's time plus the build, and the room
        # stays so a loaded machine finishes late rather than dying late.
        args.timeout = 1200 * len(passes)
    os.umask(0o077)
    try:
        directory = lock_dir(os.environ)
        record = RunRecord(directory)
        log = Path(record.data["log"]).open("a", buffering=1)
    except (OSError, ValueError) as exc:
        print(f"Check not started: {exc}", file=sys.stderr)
        return 2
    with log, contextlib.redirect_stdout(Tee(sys.stdout, log)), contextlib.redirect_stderr(Tee(sys.stderr, log)):
        fcntl.flock(log, fcntl.LOCK_EX)
        record.save()

        def terminate(signum, _frame):
            raise SystemExit(128 + signum)

        previous_term = signal.signal(signal.SIGTERM, terminate)
        result = 1
        try:
            print(f"Completion record: {record.path}", flush=True)
            result = run(args, command, passes, release_gate, record)
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
            print(str(exc), file=sys.stderr)
        except KeyboardInterrupt:
            result = 128 + signal.SIGINT
        except SystemExit as exc:
            result = exc.code
        finally:
            signal.signal(signal.SIGTERM, previous_term)
            record.data.update(exited_at=time.time(), result=result)
            record.save()
        return result


def run(args, command, passes, release_gate, record):
    held = [k for k in os.environ.get("MESIMON_TEST_HELD", "").split(",") if k]
    claims = Claims(record.root.parent, command, held, record)
    try:
        try:
            cargo_command = command
            if args.cargo_manifest:
                cargo_command = ["cargo", "--manifest-path", args.cargo_manifest]
            build_dirs = build_output(cargo_command, os.environ)
            watched_paths = disk_paths(command, os.environ, build_dirs) if args.min_free_gib else []
            check_disk_space(watched_paths, args.min_free_gib)
            # Only runs that write the same build output take turns (T-830);
            # waiting is bounded by the run's own deadline and spends none of it.
            if not args.exclusive and not held:
                # Environment can select a test road, browser cell or build config.
                # Hash it privately. Two commands with different inputs may both be needed.
                launcher_vars = {"_", "SHLVL", "OLDPWD", "CODEX_TOOL_CALL_ID"}
                env = {k: v for k, v in os.environ.items() if k not in launcher_vars}
                record.register([os.getcwd(), passes, env, args.jobs, args.cargo_manifest])
            claims.take(build_dirs, args.exclusive, time.monotonic() + args.timeout)
            check_disk_space(watched_paths, args.min_free_gib)
        except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as exc:
            print(f"Check not started: {exc}", file=sys.stderr)
            return 2
        return run_claimed(args, command, passes, build_dirs, watched_paths, claims, release_gate)
    finally:
        claims.release()
        record.retire()


def run_claimed(args, command, passes, build_dirs, watched_paths, claims, release_gate):
    # Containers can retain audit metadata on their build volume even after
    # their process namespace and temporary fixture directories disappear.
    audit_root = Path(os.environ.get("MESIMON_TEST_AUDIT_ROOT", "/tmp"))
    audit_root.mkdir(parents=True, exist_ok=True, mode=0o700)
    run = Path(tempfile.mkdtemp(prefix="msmn-test-run-", dir=audit_root)).resolve()
    env = dict(os.environ, MESIMON_TEST_RUN=str(run), MESIMON_TEST_HELD=claims.env())
    if args.jobs:
        env.update(CARGO_BUILD_JOBS=str(args.jobs), RUST_TEST_THREADS=str(args.jobs),
                   NEXTEST_TEST_THREADS=str(args.jobs))
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
    logs = []
    candidate = None
    try:
        root = Path(json.loads(guard.stdout.readline())["ok"])
        workers = f", {args.jobs} build/test workers" if args.jobs else ""
        print(f"Bounded check ({args.timeout}s deadline{workers}); audit: {run}", flush=True)
        if build_dirs:
            print(f"Build output held by this run: {', '.join(map(str, build_dirs))}", flush=True)

        def execute(batch):
            """Start the batch together and supervise it through its last result."""
            running = {}
            codes = {}
            for road, argv in batch.items():
                mod_pass = road == "mod" and args.both_roads
                extra = {"MESIMON_TEST_ROAD": "mod"} if mod_pass else {}
                label = "the mod road's pass (T-574)" if mod_pass else "tests"
                print(f"== {label}: {' '.join(argv)}", flush=True)
                pid = request(dict(op="spawn", argv=argv, env=dict(env, **extra)))
                log = PassLog(pid, open(root / f"child-{len(logs)}.log"), "mod | " if mod_pass else "")
                logs.append(log)
                running[road] = log
            last_disk_check = time.monotonic()
            while running and not interrupted:
                claims.check_superseded()
                for road, log in list(running.items()):
                    log.relay()
                    code = request(dict(op="poll", pid=log.pid))
                    if code is not None:
                        log.relay(final=True)
                        log.file.close()
                        codes[road] = code
                        del running[road]
                if running and any(codes.values()) and not keeps_going(command):
                    print("== a pass is red: the other pass is stopped with it (its summary counts "
                          "the tests it was cancelling as failed); --no-fail-fast runs both to the end",
                          flush=True)
                    break
                if time.monotonic() - last_disk_check >= 1:
                    check_disk_space(watched_paths, args.min_free_gib)
                    last_disk_check = time.monotonic()
                if running:
                    time.sleep(0.2)
            check_disk_space(watched_paths, args.min_free_gib)
            return codes

        claims.phase("both roads" if len(passes) > 1 else "tests")
        if release_gate:
            candidate = clean_head()
            if not candidate:
                raise RuntimeError("The release gate needs a clean candidate commit")
            passes = {road: [*argv[:3], *failure_report(root, road), *argv[3:]]
                      for road, argv in passes.items()}
        claims.check_superseded()
        codes = execute(passes)
        result = int(any(codes.values()))
        if release_gate and result and not interrupted:
            rerun_failures(codes, root, candidate, execute, interrupted)
        if interrupted:
            result = 128 + interrupted[0]
    except Superseded:
        result = 1
    except (RuntimeError, ValueError, OSError, ET.ParseError, subprocess.SubprocessError) as exc:
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
        for log in logs:
            if not log.file.closed:
                log.relay(final=True)
                log.file.close()
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
        claims.record.retire()
        if "superseded_by" in claims.record.data:
            print(f"Check superseded by {claims.record.data['superseded_by']}", file=sys.stderr)
            result = 1
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
            stamped = stamp_pass(command, env, list(passes), build_dirs[0] if build_dirs else None, candidate)
            if release_gate and not stamped:
                print("Release gate could not stamp the candidate.", file=sys.stderr)
                result = 1
        if (sys.platform == "darwin" and command[0] == "cargo" and build_dirs
                and "superseded_by" not in claims.record.data
                and needs_prune(build_dirs[-1] / "debug/deps")):
            claims.phase("pruning")
            profile = build_dirs[-1] / "debug"
            print(f"== pruning the build cache in {profile} (the tests are done)", flush=True)
            subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("prune-deps.py")),
                            "--quiet", "--target-dir", str(profile)], check=False)
    return result


# Gatekeeper's first-exec scan scales with the number of directory entries.
# prune-deps already advises cleaning a deps directory above this size.
TICKET_PRUNE_ENTRIES = 50_000


def needs_prune(deps):
    if not deps.is_dir():
        return False
    tree = subprocess.run(["git", "rev-parse", "--show-toplevel", "--abbrev-ref", "HEAD"],
                          capture_output=True, text=True)
    lines = tree.stdout.splitlines()
    if len(lines) == 2 and lines[1].startswith("msmn/") and Path(lines[0]) in deps.parents:
        with os.scandir(deps) as entries:
            return next(itertools.islice(entries, TICKET_PRUNE_ENTRIES, None), None) is not None
    return True


if __name__ == "__main__":
    sys.exit(main())
