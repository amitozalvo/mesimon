#!/usr/bin/env python3
"""Test-only resource owner. Never discovers or sweeps unrelated Mesimon servers.

Each fixture has a separate session, a non-inherited control pipe, a deadline,
and a durable manifest. The Rust parent must receive registration ACKs before
starting tmux. Daemons are children of this supervisor, not test-runner threads.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time


def process_table():
    out = subprocess.run(
        ["ps", "-axo", "pid=,ppid=,stat=,lstart=,command="],
        capture_output=True, text=True, timeout=3, check=True,
    )
    result = {}
    for line in out.stdout.splitlines():
        fields = line.split(None, 8)
        if len(fields) == 9:
            pid, parent, state, *rest = fields
            result[int(pid)] = (int(parent), state, " ".join(rest[:5]), rest[5])
    return result


def descendants(table, roots):
    owned = set(roots)
    while True:
        more = {pid for pid, row in table.items() if row[0] in owned}
        if more <= owned:
            return {pid: table[pid] for pid in owned if pid in table}
        owned.update(more)


def same_process(before, after):
    # PID alone is not ownership. Exclude zombies (already dead, awaiting reap).
    return after is not None and before[2:] == after[2:] and "Z" not in after[1]


class Owner:
    def __init__(self, name, tmux, run_dir=None):
        if not name or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-_" for c in name):
            raise ValueError("invalid fixture name")
        self.root = Path(tempfile.mkdtemp(prefix=f"msmn-e2e-{name}-", dir="/tmp")).resolve()
        self.tmux = tmux
        self.children = {}
        self.sockets = []
        self.repos = []
        self.dirs = []
        self.errors = []
        self.manifest = self.root / "owner.json"
        self.registry = None
        if run_dir:
            run = Path(run_dir).resolve(strict=True)
            if run.parent != Path("/tmp").resolve() or not run.name.startswith("msmn-test-run-"):
                raise ValueError("invalid test-run registry")
            if run.stat().st_uid != os.getuid() or run.stat().st_mode & 0o077:
                raise ValueError("test-run registry must be private and owned")
            self.registry = run / f"{self.root.name}.json"
        self.remember_dir(self.root)
        self.persist("active")

    def remember_dir(self, path):
        st = path.lstat()
        if path.is_symlink() or not path.is_dir() or st.st_uid != os.getuid():
            raise ValueError(f"not an owned directory: {path}")
        self.dirs.append((str(path), st.st_dev, st.st_ino))

    def persist(self, status):
        data = dict(root=str(self.root), supervisor=os.getpid(), status=status,
                    dirs=self.dirs, sockets=self.sockets, repos=self.repos,
                    children=list(self.children), errors=self.errors)
        for path in [self.manifest, self.registry]:
            if path:
                if path == self.manifest:
                    try:
                        st = self.root.lstat()
                    except FileNotFoundError:
                        continue
                    root_identity = self.dirs[0][1:]
                    if self.root.is_symlink() or (st.st_dev, st.st_ino) != root_identity:
                        continue  # Never write through a replaced fixture root.
                tmp = path.with_suffix(".tmp")
                tmp.write_text(json.dumps(data) + "\n")
                tmp.chmod(0o600)
                tmp.replace(path)

    def register(self, repo, state, runtime, sock):
        repo = Path(repo).resolve(strict=True)
        if repo != self.root and self.root not in repo.parents:
            raise ValueError("fixture repo is outside its allocated root")
        if state is not None:
            key = hashlib.sha256(os.fsencode(repo)).hexdigest()[:16]
            expected_runtime = Path(f"/tmp/mesimon-{os.getuid()}/{key}")
            state = Path(state)
            if (state.name != key or state.parent.name != "mesimon"
                    or state.parent.parent.name != "state"
                    or state.parent.parent.parent.name != ".local"
                    or Path(runtime) != expected_runtime or Path(sock) != expected_runtime / "tmux.sock"):
                raise ValueError("invalid derived Mesimon test paths")
            for path in (state, Path(runtime)):
                # A collision is not permission to remove a pre-existing tree.
                path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                path.mkdir(mode=0o700)
                self.remember_dir(path)
            self.repos.append(str(repo))
        elif Path(sock).parent.resolve() != self.root:
            raise ValueError("backend socket is outside fixture root")
        if Path(sock).exists():
            raise ValueError("refusing an existing test socket")
        self.sockets.append(str(sock))
        self.persist("active")

    def spawn(self, argv, env):
        # Control fds are closed in every child, including its descendants.
        log = open(self.root / f"child-{len(self.children)}.log", "ab")
        try:
            child = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                                     stdout=log, stderr=log, close_fds=True,
                                     start_new_session=True)
        finally:
            log.close()
        self.children[child.pid] = child
        self.persist("active")
        return child.pid

    def tmux_output(self, sock, *args):
        return subprocess.run([self.tmux, "-S", sock, *args], capture_output=True,
                              text=True, timeout=3)

    def roots(self, table):
        roots = {pid for pid, child in self.children.items() if child.poll() is None}
        for sock in self.sockets:
            if not Path(sock).exists():
                continue
            out = self.tmux_output(sock, "display-message", "-p", "#{pid}")
            if out.returncode == 0 and out.stdout.strip().isdigit():
                roots.add(int(out.stdout.strip()))
        # Client-restart tests intentionally replace the supervised daemon.
        # Authenticate its PID by BOTH our private socket and exact repo argv.
        for repo in self.repos:
            key = hashlib.sha256(os.fsencode(repo)).hexdigest()[:16]
            orch = f"/tmp/mesimon-{os.getuid()}/{key}/orch.sock"
            try:
                with socket.socket(socket.AF_UNIX) as client:
                    client.settimeout(0.3)
                    client.connect(orch)
                    client.sendall(b'{"principal":{"kind":"local"},"command":{"cmd":"hello","version":2,"client":"test-cleanup"}}\n')
                    line = client.recv(65536).split(b"\n", 1)[0]
                    pid = json.loads(line).get("daemon_pid")
                    row = table.get(pid)
                    if row and row[3].endswith(" daemon --repo " + repo):
                        roots.add(pid)
            except (OSError, ValueError):
                pass
        return roots

    def cleanup(self, reason):
        try:
            table = process_table()
            owned = descendants(table, self.roots(table))
            # An outer command supervisor must leave fixture supervisors alive
            # to handle EOF. Their own registered resources are their job.
            guardians = {pid for pid, row in owned.items() if "test_guard.py --name" in row[3]}
            excluded = descendants(table, guardians)
            owned = {pid: row for pid, row in owned.items() if pid not in excluded}
            # Stop writers first, including descendants that ignored pane HUP.
            for pid, before in owned.items():
                if same_process(before, table.get(pid)):
                    try:
                        os.kill(pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
            for sock in self.sockets:
                if Path(sock).exists():
                    self.tmux_output(sock, "kill-server")
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                for child in self.children.values():
                    child.poll()
                table = process_table()
                if not any(same_process(row, table.get(pid)) for pid, row in owned.items()):
                    break
                time.sleep(0.05)
            table = process_table()
            for pid, before in owned.items():
                if same_process(before, table.get(pid)):
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            for child in self.children.values():
                child.wait(timeout=3)
            deadline = time.monotonic() + 2
            while True:
                table = process_table()
                live = [pid for pid, row in owned.items() if same_process(row, table.get(pid))]
                if not live or time.monotonic() >= deadline:
                    break
                time.sleep(0.05)
            if live:
                raise RuntimeError(f"owned processes survived cleanup: {live}")
            # Never remove a path replaced since registration (symlinks included).
            for name, dev, ino in reversed(self.dirs):
                path = Path(name)
                if not path.exists() and not path.is_symlink():
                    continue
                st = path.lstat()
                if path.is_symlink() or (st.st_dev, st.st_ino) != (dev, ino):
                    raise RuntimeError(f"ownership changed; retained {path}")
                if path != self.root:
                    shutil.rmtree(path)
            self.persist("cleaned" if reason == "finish" else reason)
            shutil.rmtree(self.root)
            return True
        except Exception as exc:
            self.errors.append(str(exc))
            # Even if process enumeration is denied, direct Popen children
            # are ours to stop and reap. Never strand them on a diagnostic error.
            for child in self.children.values():
                try:
                    if child.poll() is None:
                        child.terminate()
                        try:
                            child.wait(timeout=2)
                        except subprocess.TimeoutExpired:
                            child.kill()
                            child.wait(timeout=2)
                except OSError as error:
                    self.errors.append(str(error))
            self.persist("cleanup-failed")
            print(f"test cleanup failed; inspect {self.manifest}: {exc}", file=sys.stderr)
            return False


def serve(args):
    os.setsid()  # Must survive Ctrl-C / termination of the cargo process group.
    os.umask(0o077)
    owner = Owner(args.name, args.tmux, os.environ.get("MESIMON_TEST_RUN"))
    selector = selectors.DefaultSelector()
    selector.register(sys.stdin, selectors.EVENT_READ)
    os.set_blocking(sys.stdin.fileno(), False)
    pending = bytearray()
    deadline = time.monotonic() + args.timeout
    reason = "runner-lost"
    try:
        print(json.dumps({"ok": str(owner.root)}), flush=True)
        while time.monotonic() < deadline:
            if not selector.select(min(0.2, max(0, deadline - time.monotonic()))):
                continue
            chunk = os.read(sys.stdin.fileno(), 65536)
            if not chunk:
                break
            pending.extend(chunk)
            if len(pending) > 8 * 1024 * 1024:
                raise ValueError("test supervisor request exceeds size limit")
            if b"\n" not in pending:
                continue  # A partial request must not disable the deadline.
            line, rest = pending.split(b"\n", 1)
            pending = bytearray(rest)
            request = json.loads(line)
            op = request["op"]
            if op == "finish":
                reason = "finish"
                break
            try:
                if op == "register":
                    owner.register(request["repo"], request.get("state"),
                                   request.get("runtime"), request["sock"])
                    value = None
                elif op == "spawn":
                    value = owner.spawn(request["argv"], request["env"])
                elif op == "poll":
                    value = owner.children[request["pid"]].poll()
                else:
                    raise ValueError(f"unknown operation: {op}")
                print(json.dumps({"ok": value}), flush=True)
            except Exception as exc:
                print(json.dumps({"error": str(exc)}), flush=True)
        else:
            reason = "fixture-timeout"
    finally:
        selector.close()
        clean = owner.cleanup(reason)
    return 0 if clean and reason == "finish" else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True)
    parser.add_argument("--tmux", required=True)
    parser.add_argument("--timeout", type=float, default=180)
    sys.exit(serve(parser.parse_args()))
