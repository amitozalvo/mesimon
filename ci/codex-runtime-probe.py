#!/usr/bin/env python3
"""Bounded native Codex TUI/app-server observation experiment, never a parity gate.

Run: python3 -B ci/test-run.py -- python3 -B ci/codex-runtime-probe.py
Add --live --model MODEL to submit one tiny synthetic prompt using existing
auth.json credentials. No personal settings or conversations are copied.
The observer never answers server requests (including approvals).
"""

import argparse
import base64
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
MAX_FRAME = 16 * 1024 * 1024


class ProbeComplete(Exception):
    pass


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n")


def read_frames(path):
    if not path.exists():
        return []
    frames = []
    for line in path.read_text().splitlines():
        try:
            frames.append(json.loads(line))
        except ValueError:
            pass  # The relay may still be appending the last line.
    return frames


class Guard:
    def __init__(self, timeout, tmux):
        self.process = subprocess.Popen([
            sys.executable, "-B", str(ROOT / "ci/test_guard.py"), "--name",
            "codex-runtime", "--tmux", tmux, "--timeout", str(timeout)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.root = Path(self.reply())

    def reply(self):
        if not select.select([self.process.stdout], [], [], 10)[0]:
            raise TimeoutError("fixture supervisor did not reply")
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("fixture supervisor exited")
        value = json.loads(line)
        if "error" in value:
            raise RuntimeError(value["error"])
        return value["ok"]

    def request(self, **value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()
        return self.reply()

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.write('{"op":"finish"}\n')
            self.process.stdin.flush()
            self.process.stdin.close()
        return self.process.wait(timeout=15) == 0


class WebSocket:
    """Minimal RFC 6455 client over an owned AF_UNIX socket; no dependencies."""
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(5)
        self.sock.connect(str(path))
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(("GET / HTTP/1.1\r\nHost: localhost\r\n"
                           "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                           f"Sec-WebSocket-Key: {key}\r\n"
                           "Sec-WebSocket-Version: 13\r\n\r\n").encode())
        self.buffer = bytearray()
        while b"\r\n\r\n" not in self.buffer:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("server closed during websocket handshake")
            self.buffer.extend(chunk)
            if len(self.buffer) > 65536:
                raise RuntimeError("oversized websocket handshake")
        header, rest = bytes(self.buffer).split(b"\r\n\r\n", 1)
        self.buffer = bytearray(rest)
        accept = base64.b64encode(hashlib.sha1(
            (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
        headers = dict(line.lower().split(b":", 1) for line in header.split(b"\r\n")[1:] if b":" in line)
        if b" 101 " not in header.split(b"\r\n")[0] or headers.get(b"sec-websocket-accept", b"").strip() != accept.lower():
            raise RuntimeError("invalid websocket upgrade: " + header.decode(errors="replace"))

    def send(self, payload, opcode=1):
        if isinstance(payload, str):
            payload = payload.encode()
        mask = os.urandom(4)
        length = len(payload)
        head = bytes([0x80 | opcode])
        if length < 126:
            head += bytes([0x80 | length])
        elif length <= 65535:
            head += bytes([0x80 | 126]) + struct.pack("!H", length)
        else:
            head += bytes([0x80 | 127]) + struct.pack("!Q", length)
        self.sock.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def exact(self, n, deadline):
        while len(self.buffer) < n:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("websocket receive deadline")
            self.sock.settimeout(remaining)
            chunk = self.sock.recv(min(65536, max(1, n - len(self.buffer))))
            if not chunk:
                raise RuntimeError("websocket closed")
            self.buffer.extend(chunk)
        result = bytes(self.buffer[:n])
        del self.buffer[:n]
        return result

    def receive(self, timeout):
        if not self.buffer and not select.select([self.sock], [], [], timeout)[0]:
            return None
        deadline = time.monotonic() + 5
        fragments = bytearray()
        while True:
            head = self.exact(2, deadline)
            final, opcode, masked = head[0] & 128, head[0] & 15, head[1] & 128
            length = head[1] & 127
            if length == 126:
                length = struct.unpack("!H", self.exact(2, deadline))[0]
            elif length == 127:
                length = struct.unpack("!Q", self.exact(8, deadline))[0]
            if masked or length + len(fragments) > MAX_FRAME:
                raise RuntimeError("invalid or oversized websocket server frame")
            payload = self.exact(length, deadline)
            if opcode == 8:
                raise RuntimeError("websocket close: " + repr(payload))
            if opcode == 9:
                self.send(payload, 10)
                continue
            if opcode == 10:
                continue
            if opcode not in (0, 1):
                raise RuntimeError(f"unsupported websocket opcode {opcode}")
            fragments.extend(payload)
            if final:
                return json.loads(fragments)

    def close(self):
        self.sock.close()


class Rpc:
    def __init__(self, path, out, label):
        self.ws = WebSocket(path)
        self.log = (out / f"rpc-{label}.jsonl").open("w")
        self.serial = 0
        self.frames = []

    def record(self, direction, frame):
        self.log.write(json.dumps(dict(at=time.monotonic(), direction=direction, frame=frame)) + "\n")
        self.log.flush()

    def receive(self, timeout=0.1):
        frame = self.ws.receive(timeout)
        if frame is not None:
            self.record("receive", frame)
            self.frames.append(frame)
        return frame

    def send(self, method, params, ident=None):
        frame = dict(method=method, params=params)
        if ident is not None:
            frame["id"] = ident
        self.record("send", frame)
        self.ws.send(json.dumps(frame))

    def call(self, method, params):
        self.serial += 1
        self.send(method, params, self.serial)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            frame = self.receive()
            if frame is not None and frame.get("id") == self.serial and "method" not in frame:
                return frame
        raise TimeoutError(f"RPC timed out: {method}")

    def initialize(self):
        value = self.call("initialize", {"clientInfo": {"name": "mesimon_runtime_probe", "version": "0.1.0"},
                                        "capabilities": {"experimentalApi": True}})
        if "error" in value:
            raise RuntimeError(f"initialize rejected: {value['error']}")
        self.send("initialized", {})
        return value

    def close(self):
        self.ws.close()
        self.log.close()


def digest(path):
    with path.open("rb") as source:
        result = hashlib.sha256()
        while chunk := source.read(1024 * 1024):
            result.update(chunk)
    return result.hexdigest()


def identities(codex):
    paths = {Path(codex).resolve()}
    # npm installs can wrap the native executable; record both, without running
    # package-manager resolution or examining any user conversation directory.
    package = Path(codex).resolve().parent.parent
    for vendor in (package / "vendor", package / "node_modules", package.parent):
        if vendor.is_dir():
            for candidate in vendor.glob("**/vendor/*/bin/codex"):
                paths.add(candidate.resolve())
    return [{"path": str(path), "sha256": digest(path)} for path in sorted(paths)]


class FrameCapture:
    """Decode a copy of a byte stream; forwarding never reconstructs frames."""
    def __init__(self, direction, log):
        self.direction, self.log = direction, log
        self.buffer = bytearray()
        self.fragments = bytearray()
        self.handshake = True

    def feed(self, chunk):
        self.buffer.extend(chunk)
        if self.handshake:
            if b"\r\n\r\n" not in self.buffer:
                if len(self.buffer) > 65536:
                    raise RuntimeError("relay handshake exceeds limit")
                return
            _, rest = self.buffer.split(b"\r\n\r\n", 1)
            self.buffer = bytearray(rest)
            self.handshake = False
        while len(self.buffer) >= 2:
            first, second = self.buffer[:2]
            final, opcode, masked = first & 128, first & 15, second & 128
            length, offset = second & 127, 2
            if length == 126:
                if len(self.buffer) < 4:
                    return
                length, offset = struct.unpack("!H", self.buffer[2:4])[0], 4
            elif length == 127:
                if len(self.buffer) < 10:
                    return
                length, offset = struct.unpack("!Q", self.buffer[2:10])[0], 10
            if length > MAX_FRAME:
                raise RuntimeError("relay frame exceeds capture limit")
            if masked:
                offset += 4
            if len(self.buffer) < offset + length:
                return
            payload = bytes(self.buffer[offset:offset + length])
            if masked:
                mask = self.buffer[offset - 4:offset]
                payload = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
            del self.buffer[:offset + length]
            if opcode not in (0, 1):
                continue
            self.fragments.extend(payload)
            if len(self.fragments) > MAX_FRAME:
                raise RuntimeError("relay message exceeds capture limit")
            if final:
                try:
                    frame = json.loads(self.fragments)
                except ValueError:
                    frame = {"captureDecodeError": True, "bytes": len(self.fragments)}
                self.log.write(json.dumps(dict(at=time.monotonic(), direction=self.direction, frame=frame)) + "\n")
                self.log.flush()
                self.fragments.clear()


def relay(listen_path, upstream_path, log_path, timeout):
    """Copy raw native WebSocket bytes both ways; no RPC calls or responses."""
    deadline = time.monotonic() + float(timeout)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(listen_path)
        listener.listen(4)
        with Path(log_path).open("w") as log:
            while time.monotonic() < deadline:
                if not select.select([listener], [], [], 0.2)[0]:
                    continue
                native, _ = listener.accept()
                with native, socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as upstream:
                    upstream.connect(upstream_path)
                    captures = {native: FrameCapture("native_to_server", log),
                                upstream: FrameCapture("server_to_native", log)}
                    peers = {native: upstream, upstream: native}
                    connected = True
                    while connected and time.monotonic() < deadline:
                        ready, _, _ = select.select(list(peers), [], [], 0.2)
                        for source in ready:
                            chunk = source.recv(65536)
                            if not chunk:
                                connected = False
                                break
                            # Relay the exact bytes before looking at the copy.
                            peers[source].sendall(chunk)
                            captures[source].feed(chunk)


def fixture_hook():
    payload = json.load(sys.stdin)
    event = payload.get("hook_event_name")
    output = {}
    if event == "SessionStart" and Path(os.environ["MESIMON_PROBE_BRIEF_FLAG"]).exists():
        output = {"hookSpecificOutput": {"hookEventName": event,
                  "additionalContext": "MESIMON_BRIEF_SENTINEL"}}
    elif event == "PreToolUse" and payload.get("tool_name") == "apply_patch":
        command = payload.get("tool_input", {}).get("command", "")
        if "protected.txt" in command:
            output = {"hookSpecificOutput": {"hookEventName": event,
                      "permissionDecision": "deny", "permissionDecisionReason": "Synthetic protected fixture path"}}
    with Path(os.environ["MESIMON_PROBE_HOOK_LOG"]).open("a") as log:
        log.write(json.dumps(dict(input=payload, output=output)) + "\n")
    print(json.dumps(output))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true", help="one economical real model turn; explicitly opt in")
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--history-mode", choices=("legacy", "paginated"), default="legacy")
    parser.add_argument("--launch-mode", choices=("native", "thread-first"), default="native")
    parser.add_argument("--relay", action="store_true", help="also capture unchanged native RPC bytes through an owned relay")
    parser.add_argument("--case", choices=("complete", "approval-cancel", "hooks-config", "hooks-trust", "gate-patch",
                                          "active-enter", "active-tab"), default="complete")
    parser.add_argument("--mesimon-mcp", action="store_true", help="connect real scoped Mesimon MCP on a generated board")
    parser.add_argument("--mesimon-binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--cold-resume", action="store_true", help="after hooks-trust, restart app-server and inspect exact-ID resume with brief off")
    parser.add_argument("--codex-binary", default=shutil.which("codex"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json")
    args = parser.parse_args()
    with_hooks = args.case in ("hooks-trust", "gate-patch")
    if not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run through python3 -B ci/test-run.py -- python3 -B ci/codex-runtime-probe.py")
    if args.timeout < 20 or args.timeout > 300:
        parser.error("timeout must be 20..300 seconds")
    if (args.case in ("approval-cancel", "active-enter", "active-tab") or with_hooks) and not (args.live and args.relay):
        parser.error("approval experiment requires --live --relay")
    if args.cold_resume and args.case != "hooks-trust":
        parser.error("--cold-resume requires --case hooks-trust")
    if args.mesimon_mcp and args.case != "complete":
        parser.error("--mesimon-mcp is a separate complete-turn experiment")
    tmux = shutil.which(os.environ.get("MESIMON_TMUX_BIN", "tmux"))
    if not args.codex_binary or not tmux:
        parser.error("codex and tmux are required")
    os.umask(0o077)
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-runtime-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="codex_native_tui_runtime_probe", live=args.live, model=args.model, case=args.case,
                    outcome="inconclusive", cleanup=False, limitation="Observation experiment, not Mesimon parity verification",
                    cost_usd=None, cost_note="No billing estimate; token usage notifications retained if supplied")
    guard = rpc = bootstrap = None
    started = time.monotonic()
    try:
        guard = Guard(args.timeout + 20, tmux)
        home, repo = guard.root / "codex-home", guard.root / "repo"
        home.mkdir(mode=0o700)
        repo.mkdir(mode=0o700)
        subprocess.run(["git", "init", "-q", str(repo)], check=True, timeout=5)
        (repo / "README.md").write_text("Disposable synthetic runtime verification repository.\n")
        (home / "config.toml").write_text('cli_auth_credentials_store = "file"\ncheck_for_update_on_startup = false\n')
        if args.case == "hooks-config":
            with (home / "config.toml").open("a") as config:
                config.write('[[hooks.PreToolUse]]\nmatcher = "^Bash$"\n[[hooks.PreToolUse.hooks]]\n'
                             'type = "command"\ncommand = "/bin/echo user-toml"\n')
            write(home / "hooks.json", {"hooks": {"PreToolUse": [{"matcher": "^Bash$", "hooks": [
                {"type": "command", "command": "/bin/echo user-json"}]}]}})
        if with_hooks:
            with (home / "config.toml").open("a") as config:
                config.write('developer_instructions = "The fixture developer sentinel is MESIMON_DEVELOPER_SENTINEL."\n')
        if args.case == "gate-patch":
            for name, content in {"safe-a.txt": "a\n", "safe-b.txt": "b\n", "protected.txt": "protected\n",
                                  "rename-source.txt": "old\n"}.items():
                (repo / name).write_text(content)
        if args.live:
            if not args.auth_file.is_file():
                raise RuntimeError("existing auth.json unavailable; no login or keychain changes attempted")
            shutil.copyfile(args.auth_file, home / "auth.json")
            (home / "auth.json").chmod(0o600)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("CODEX_", "OPENAI_", "MESIMON_")) and key not in ("TMUX", "TMUX_PANE")}
        env.update(CODEX_HOME=str(home), TERM="xterm-256color")
        mcp_flags = []
        if args.mesimon_mcp:
            binary = args.mesimon_binary.resolve(strict=True)
            spec = importlib.util.spec_from_file_location("state_lab", ROOT / "ci/state-lab.py")
            lab = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(lab)
            runtime, state = lab.paths(repo)
            guard.request(op="register", repo=str(repo), state=str(state), runtime=str(runtime), sock=str(runtime / "tmux.sock"))
            daemon_env = dict(env, MESIMON_TMUX_BIN=tmux, MESIMON_NO_DAEMON_AUTORESTART="1")
            guard.request(op="spawn", argv=[str(binary), "daemon", "--repo", str(repo)], env=daemon_env)
            lab.wait_for(lambda: (runtime / "orch.sock").exists())
            client = lab.Client(repo)
            try:
                client.request("set_mcp_tools", on=True)
                ticket = client.request("create_ticket", column="TODO", title="MESIMON_SCOPED_TICKET", workspace=None)["id"]
                client.request("create_ticket", column="TODO", title="MESIMON_DECOY_TICKET", workspace=None)
                sid = client.request("spawn_session", ticket=ticket, kind="bash", submit_prompt=False)["id"]
                manifest["mesimon_mcp"] = dict(binary=str(binary), sha256=digest(binary), ticket=ticket, session=sid,
                                              limitation="Real scoped MCP on a shell seat; not production Codex-session launch")
            finally:
                client.close()
            mcp_flags = ["-c", 'mcp_servers.mesimon={command=' + json.dumps(str(binary)) + ',args=['
                         + ','.join(json.dumps(arg) for arg in ["mcp", "--sock", str(runtime / "orch.sock"), "--session", sid, "--tools", "read"])
                         + ']}']
        hook_flags = []
        if with_hooks:
            env.update(MESIMON_PROBE_PYTHON=sys.executable, MESIMON_PROBE_SCRIPT=str(Path(__file__).resolve()),
                       MESIMON_PROBE_HOOK_LOG=str(out / "hook-inputs.jsonl"),
                       MESIMON_PROBE_BRIEF_FLAG=str(guard.root / "brief-enabled"))
            (guard.root / "brief-enabled").touch()
            command = '"$MESIMON_PROBE_PYTHON" -B "$MESIMON_PROBE_SCRIPT" hook'
            for event in ("SessionStart", "PreToolUse"):
                matcher = "startup|resume" if event == "SessionStart" else "apply_patch"
                hook_flags.extend(["-c", "hooks." + event + '=[{matcher=' + json.dumps(matcher)
                                   + ',hooks=[{type="command",command=' + json.dumps(command) + ',timeout=5}]}]'])
        manifest.update(codex_version=subprocess.check_output([args.codex_binary, "--version"], env=env, text=True, timeout=5).strip(),
                        binaries=identities(args.codex_binary), tmux=tmux)
        endpoint, sock = guard.root / "app.sock", guard.root / "tmux.sock"
        guard.request(op="register", repo=str(repo), sock=str(sock))
        server = [args.codex_binary, "app-server", "--listen", "unix://" + str(endpoint)]
        server.extend(hook_flags)
        server.extend(mcp_flags)
        if args.case == "hooks-config":
            server.extend(["-c", 'hooks.PreToolUse=[{matcher="^Bash$",hooks=[{type="command",command="/bin/echo session-flag"}]}]'])
        manifest["server_argv"] = server
        server_pid = guard.request(op="spawn", argv=server, env=env)
        deadline = started + args.timeout
        while not endpoint.exists() and time.monotonic() < deadline:
            if guard.request(op="poll", pid=server_pid) is not None:
                raise RuntimeError("app-server exited before creating its endpoint; inspect child-0.log")
            time.sleep(0.1)
        native_endpoint = endpoint
        if args.relay:
            native_endpoint = guard.root / "relay.sock"
            relay_pid = guard.request(op="spawn", argv=[sys.executable, "-B", str(Path(__file__).resolve()),
                "relay", str(native_endpoint), str(endpoint), str(out / "native-rpc.jsonl"), str(args.timeout)], env=env)
            while not native_endpoint.exists() and time.monotonic() < deadline:
                if guard.request(op="poll", pid=relay_pid) is not None:
                    raise RuntimeError("native relay exited before creating endpoint")
                time.sleep(0.1)
            manifest["native_relay"] = True
        rpc = Rpc(endpoint, out, "observer")
        manifest["initialize"] = rpc.initialize()
        if args.case == "hooks-config":
            manifest["hooks_list"] = rpc.call("hooks/list", {"cwds": [str(repo)]})
            manifest["effective_config"] = rpc.call("config/read", {"includeLayers": True})
            manifest["outcome"] = "hook_config_sources_observed"
            raise ProbeComplete()
        manifest["launch_mode"] = args.launch_mode
        manifest["available_models"] = rpc.call("model/list", {})
        available = [model.get("model") for model in manifest["available_models"].get("result", {}).get("data", [])]
        if args.live and available and args.model not in available:
            raise RuntimeError(f"requested model {args.model!r} not in authenticated model/list: {available}")
        thread = None
        if args.launch_mode == "thread-first":
            manifest["history_mode"] = args.history_mode
            created = rpc.call("thread/start", {"cwd": str(repo), "model": args.model, "historyMode": args.history_mode})
            if "error" in created:
                raise RuntimeError(f"thread/start failed: {created['error']}")
            thread = created["result"]["thread"]["id"]
            manifest["thread_id"] = thread
            manifest["thread_start"] = created
            bootstrap = rpc
            rpc = None

        def tm(*arguments):
            result = subprocess.run([tmux, "-S", str(sock), *arguments], env=env,
                                    capture_output=True, text=True, timeout=5)
            if result.returncode:
                raise RuntimeError("tmux: " + result.stderr.strip())
            return result.stdout

        native = [args.codex_binary, "--remote", "unix://" + str(native_endpoint),
                  "--no-alt-screen", "-C", str(repo), "-m", args.model]
        native.extend(hook_flags)
        native.extend(mcp_flags)
        if args.case == "approval-cancel":
            native.extend(["--sandbox", "read-only", "--ask-for-approval", "on-request"])
        if args.case == "gate-patch":
            native.extend(["--sandbox", "workspace-write", "--ask-for-approval", "on-request"])
        if thread:
            native.extend(["resume", thread])
        manifest["native_argv"] = native
        launch = guard.root / "launch.sh"
        launch.write_text("#!/bin/sh\nexec " + shlex.join(native) + "\n")
        launch.chmod(0o700)
        tmux_pid = guard.request(op="spawn", argv=[tmux, "-S", str(sock), "-f", "/dev/null", "new-session",
                                "-d", "-s", "probe", "-x", "120", "-y", "40", "-c", str(repo), str(launch)], env=env)
        while time.monotonic() < deadline:
            code = guard.request(op="poll", pid=tmux_pid)
            if code is not None:
                if code:
                    raise RuntimeError(f"tmux startup exited {code}; inspect child-1.log")
                break
            time.sleep(0.1)
        tm("set-option", "-t", "probe", "remain-on-exit", "on")
        if rpc is None:
            rpc = Rpc(endpoint, out, "reattached")
            rpc.initialize()
        manifest["thread_list"] = rpc.call("thread/list", {"cwd": str(repo), "limit": 10})
        submitted = False
        subscribed = False
        approval_request = None
        last_attach = 0
        previous = None
        with (out / "terminal.jsonl").open("w") as terminal:
            while time.monotonic() < deadline:
                screen = tm("capture-pane", "-p", "-t", "probe", "-S", "-200")
                if screen != previous:
                    terminal.write(json.dumps(dict(elapsed=time.monotonic() - started, screen=screen)) + "\n")
                    terminal.flush()
                    previous = screen
                rpc.receive(0.2)
                if args.case in ("active-enter", "active-tab") and submitted and not manifest.get("active_input_sent"):
                    running = [frame for frame in rpc.frames if frame.get("method") == "item/started"
                               and frame.get("params", {}).get("threadId") == thread
                               and frame.get("params", {}).get("item", {}).get("type") == "commandExecution"
                               and "sleep 8" in frame.get("params", {}).get("item", {}).get("command", "")]
                    if running:
                        tm("send-keys", "-t", "probe", "-l", "Reply exactly SECOND_DONE.")
                        time.sleep(0.4)
                        key = "Enter" if args.case == "active-enter" else "Tab"
                        tm("send-keys", "-t", "probe", key)
                        manifest["active_input_sent"] = key
                if args.mesimon_mcp and not manifest.get("native_mcp_read_approved"):
                    visible = tm("capture-pane", "-p", "-t", "probe")
                    if 'Allow the mesimon MCP server to run tool "get_ticket"?' in visible and "1. Allow" in visible:
                        tm("send-keys", "-t", "probe", "Enter")
                        manifest["native_mcp_read_approved"] = True
                        continue
                if with_hooks:
                    visible = tm("capture-pane", "-p", "-t", "probe")
                    if "Hooks need review" in visible and not manifest.get("hook_review_entered"):
                        tm("send-keys", "-t", "probe", "Enter")
                        manifest["hook_review_entered"] = True
                        continue
                    if "Press t to trust all;" in visible and manifest.get("hook_review_entered") and not manifest.get("hook_trust_requested"):
                        manifest["hooks_before_trust"] = rpc.call("hooks/list", {"cwds": [str(repo)]})
                        tm("send-keys", "-t", "probe", "t")
                        manifest["hook_trust_requested"] = True
                        continue
                    if "Press enter to view hooks; esc to close" in visible and manifest.get("hook_trust_requested") and not manifest.get("hook_new_requested"):
                        manifest["hooks_after_trust"] = rpc.call("hooks/list", {"cwds": [str(repo)]})
                        tm("send-keys", "-t", "probe", "Escape")
                        time.sleep(0.3)
                        tm("send-keys", "-t", "probe", "-l", "/new")
                        time.sleep(0.4)
                        tm("send-keys", "-t", "probe", "Enter")
                        manifest["hook_new_requested"] = True
                        manifest["pretrust_thread_id"] = thread
                        thread = None
                        subscribed = False
                        continue
                    if manifest.get("hook_new_requested") and not submitted:
                        users = [frame["params"]["thread"] for frame in rpc.frames if frame.get("method") == "thread/started"
                                 and not frame.get("params", {}).get("thread", {}).get("ephemeral", True)]
                        if users and users[-1]["id"] != manifest.get("pretrust_thread_id"):
                            thread = users[-1]["id"]
                            manifest["thread_id"] = thread
                            manifest["hook_new_ready"] = True
                if args.case == "approval-cancel" and not manifest.get("native_approval_cancelled"):
                    approvals = [row["frame"] for row in read_frames(out / "native-rpc.jsonl")
                                 if row["direction"] == "server_to_native" and
                                 row["frame"].get("method", "").endswith("/requestApproval")]
                    if approvals:
                        approval_request = approvals[-1]
                        manifest["native_approval_request"] = approval_request
                        # The native UI owns and answers its own dialog. The
                        # observer remains silent; Escape cancels this owned
                        # fixture request without allowing the command to run.
                        time.sleep(0.5)
                        tm("send-keys", "-t", "probe", "Escape")
                        manifest["native_approval_cancelled"] = True
                if time.monotonic() - last_attach > 0.5 and not subscribed:
                    last_attach = time.monotonic()
                    if thread is None:
                        loaded = rpc.call("thread/loaded/list", {})
                        manifest["thread_loaded_list"] = loaded
                        ids = loaded.get("result", {}).get("data", [])
                        if len(ids) == 1:
                            thread = ids[0]
                            manifest["thread_id"] = thread
                    if thread:
                        manifest["thread_read_before"] = rpc.call("thread/read", {"threadId": thread})
                        # No settings overrides and no history injection. An
                        # empty native thread may not have a rollout yet.
                        resumed = rpc.call("thread/resume", {"threadId": thread, "excludeTurns": True})
                        manifest["observer_resume"] = resumed
                        subscribed = "result" in resumed
                # This repo is generated by this probe and contains only its
                # synthetic README. Accept its native startup directory prompt,
                # never a model tool request or arbitrary path. No hook trust
                # is injected or bypassed and the observer never responds.
                if (str(repo) in screen and "Do you trust the contents of this directory?" in screen
                        and "1. Yes, continue" in screen and not manifest.get("fixture_directory_trust_accepted")):
                    tm("send-keys", "-t", "probe", "Enter")
                    manifest["fixture_directory_trust_accepted"] = True
                    continue
                auth_required = "Sign in with ChatGPT" in screen or "Provide your own API key" in screen
                ready = thread is not None and f"model:     {args.model}" in screen and not auth_required
                if auth_required and not args.live:
                    manifest["outcome"] = "credential_free_transport_observed_native_auth_required"
                    break
                if ready and not submitted:
                    manifest["native_ready"] = True
                    if with_hooks:
                        if not manifest.get("hook_new_ready"):
                            continue
                    if not args.live:
                        manifest["outcome"] = "credential_free_transport_observed"
                        break
                    prompt = "Reply exactly MESIMON_PROBE_OK. Do not use tools."
                    if args.case == "hooks-trust":
                        prompt = "Reply with the developer sentinel and SessionStart brief sentinel you were given, separated by one space. Do not use tools."
                    if args.case == "gate-patch":
                        prompt = ("Use apply_patch once to append NEW on a new line to both safe-a.txt and safe-b.txt, "
                                  "which contain a and b respectively. Then use a second separate apply_patch to rename "
                                  "rename-source.txt to protected.txt, changing old to RENAMED. If blocked, stop and reply "
                                  "exactly BLOCKED. Do not retry or use the shell or other tools.")
                    if args.case in ("active-enter", "active-tab"):
                        prompt = "Use the shell to run sleep 8, then reply exactly FIRST_DONE. Do not use any other tool."
                    if args.mesimon_mcp:
                        prompt = "Call Mesimon get_ticket once and reply with only its exact title. Do not use any other tool."
                    if args.case == "approval-cancel":
                        prompt = ("Use the shell tool to run exactly: printf MESIMON_PROBE_WRITE > "
                                  + str(guard.root / "outside.txt")
                                  + ". This is a disposable fixture. Request permission if required. "
                                    "If permission is declined, stop and reply DENIED; do not retry.")
                    tm("send-keys", "-t", "probe", "-l", prompt)
                    # Native Codex distinguishes pasted bursts from submission;
                    # let its paste detector settle before the explicit Enter.
                    time.sleep(0.4)
                    tm("send-keys", "-t", "probe", "Enter")
                    submitted = True
                    manifest["prompt_submitted"] = True
                completed = [frame for frame in rpc.frames if frame.get("method") == "turn/completed"
                             and frame.get("params", {}).get("threadId") == thread]
                if args.case == "active-tab" and len(completed) < 2:
                    continue
                if submitted and completed:
                    manifest["turn_completed"] = completed[-1]
                    final = completed[-1].get("params", {}).get("turn", {})
                    replies = [frame.get("params", {}).get("item", {}) for frame in rpc.frames
                               if frame.get("method") == "item/completed"
                               and frame.get("params", {}).get("threadId") == thread]
                    replies.extend(final.get("items", []))
                    expected_reply = ("MESIMON_DEVELOPER_SENTINEL MESIMON_BRIEF_SENTINEL"
                                      if args.case == "hooks-trust" else "MESIMON_PROBE_OK")
                    if args.mesimon_mcp:
                        expected_reply = "MESIMON_SCOPED_TICKET"
                    if args.case == "gate-patch":
                        expected_reply = "BLOCKED"
                    if args.case in ("active-enter", "active-tab"):
                        expected_reply = "SECOND_DONE"
                    exact_reply = any(item.get("type") == "agentMessage" and
                                      item.get("text", "").strip() == expected_reply for item in replies)
                    manifest["exact_agent_reply"] = exact_reply
                    manifest["outcome"] = ("live_turn_observed" if final.get("status") == "completed" and exact_reply
                                           else "live_turn_did_not_complete_expected_reply")
                    if args.case == "hooks-trust" and exact_reply and final.get("status") == "completed":
                        manifest["outcome"] = "native_trusted_additive_brief_observed"
                    if args.mesimon_mcp:
                        calls = [item for item in replies if item.get("type") == "mcpToolCall" and item.get("tool") == "get_ticket"]
                        manifest["mesimon_mcp_calls"] = calls
                        manifest["outcome"] = ("scoped_mesimon_mcp_observed" if exact_reply and calls and final.get("status") == "completed"
                                               else "scoped_mesimon_mcp_inconclusive")
                    if args.case == "gate-patch":
                        manifest["gate_files"] = {name: (repo / name).read_text() if (repo / name).exists() else None
                                                  for name in ("safe-a.txt", "safe-b.txt", "protected.txt", "rename-source.txt")}
                        hooks = read_frames(out / "hook-inputs.jsonl")
                        manifest["patch_hooks"] = [row for row in hooks if row.get("input", {}).get("tool_name") == "apply_patch"]
                        denied = any(row.get("output", {}).get("hookSpecificOutput", {}).get("permissionDecision") == "deny"
                                     for row in manifest["patch_hooks"])
                        files_ok = manifest["gate_files"] == {"safe-a.txt": "a\nNEW\n", "safe-b.txt": "b\nNEW\n",
                                                              "protected.txt": "protected\n", "rename-source.txt": "old\n"}
                        manifest["outcome"] = ("native_patch_gate_observed" if denied and files_ok and final.get("status") == "completed"
                                               else "native_patch_gate_inconclusive")
                    if args.case in ("active-enter", "active-tab"):
                        input_calls = [row["frame"] for row in read_frames(out / "native-rpc.jsonl")
                                       if row["direction"] == "native_to_server"
                                       and row["frame"].get("method") in ("turn/start", "turn/steer", "thread/queue/add")
                                       and row["frame"].get("params", {}).get("threadId") == thread]
                        manifest["active_input_calls"] = input_calls
                        manifest["completed_turn_count"] = len(completed)
                        manifest["outcome"] = ("native_active_input_observed" if exact_reply and manifest.get("active_input_sent")
                                               else "native_active_input_inconclusive")
                    if args.case == "approval-cancel":
                        responses = [row["frame"] for row in read_frames(out / "native-rpc.jsonl")
                                     if row["direction"] == "native_to_server" and approval_request
                                     and row["frame"].get("id") == approval_request.get("id")
                                     and "result" in row["frame"]]
                        manifest["native_approval_responses"] = responses
                        manifest["outside_file_exists"] = (guard.root / "outside.txt").exists()
                        denied = any(any(decision in json.dumps(frame.get("result")) for decision in ("cancel", "decline"))
                                     for frame in responses)
                        manifest["outcome"] = ("native_approval_cancel_observed" if denied and not manifest["outside_file_exists"]
                                               else "native_approval_cancel_inconclusive")
                    break
                if not args.live and time.monotonic() - started > min(20, args.timeout - 10):
                    manifest["outcome"] = "transport_observed_native_startup_inconclusive"
                    break
        if thread:
            manifest["thread_read_after"] = rpc.call("thread/read", {"threadId": thread, "includeTurns": True})
        if args.cold_resume and manifest["outcome"] == "native_trusted_additive_brief_observed":
            (guard.root / "brief-enabled").unlink()
            # A native-client reconnect to an already loaded server thread is
            # attachment, not a cold resume, and does not rerun SessionStart.
            rpc.close()
            rpc = None
            tm("respawn-pane", "-k", "-t", "probe", "/bin/sleep 2")
            if guard.request(op="poll", pid=server_pid) is not None:
                raise RuntimeError("owned app-server unexpectedly exited before cold resume")
            os.kill(server_pid, signal.SIGTERM)
            while guard.request(op="poll", pid=server_pid) is None and time.monotonic() < deadline:
                time.sleep(0.1)
            server_pid = guard.request(op="spawn", argv=server, env=env)
            while time.monotonic() < deadline:
                try:
                    rpc = Rpc(endpoint, out, "wake-observer")
                    rpc.initialize()
                    break
                except (OSError, RuntimeError):
                    if rpc:
                        rpc.close()
                        rpc = None
                    time.sleep(0.1)
            if rpc is None:
                raise TimeoutError("restarted app-server unavailable")
            resumed_native = [*native, "resume", thread]
            launch.write_text("#!/bin/sh\nexec " + shlex.join(resumed_native) + "\n")
            manifest["wake_argv"] = resumed_native
            tm("respawn-pane", "-k", "-t", "probe", "-c", str(repo), str(launch))
            while time.monotonic() < min(deadline, started + args.timeout):
                rpc.receive(0.2)
                hooks = read_frames(out / "hook-inputs.jsonl")
                resumed_hooks = [row for row in hooks if row.get("input", {}).get("source") == "resume"
                                 and row.get("input", {}).get("session_id") == thread]
                if resumed_hooks:
                    manifest["wake_hook"] = resumed_hooks[-1]
                    manifest["wake_brief_absent"] = not resumed_hooks[-1].get("output")
                    manifest["wake_thread"] = rpc.call("thread/read", {"threadId": thread})
                    manifest["outcome"] = ("native_trusted_brief_and_wake_off_observed" if manifest["wake_brief_absent"]
                                           else "wake_brief_off_failed")
                    break
            else:
                manifest["outcome"] = "wake_hook_inconclusive"
        if with_hooks:
            manifest["hooks_list_after"] = rpc.call("hooks/list", {"cwds": [str(repo)]})
        manifest["notifications"] = sorted({frame["method"] for frame in rpc.frames if "method" in frame})
        manifest["token_usage"] = [frame["params"] for frame in rpc.frames
                                   if frame.get("method") == "thread/tokenUsage/updated"
                                   and frame.get("params", {}).get("threadId") == thread]
        manifest["unanswered_server_requests"] = [frame for frame in rpc.frames if "method" in frame and "id" in frame]
        manifest["elapsed_seconds"] = round(time.monotonic() - started, 3)
        manifest["deadline_reached"] = time.monotonic() >= deadline
    except ProbeComplete:
        pass
    except Exception as exc:
        manifest["error"] = f"{type(exc).__name__}: {exc}"
    finally:
        if rpc:
            rpc.close()
        if bootstrap:
            bootstrap.close()
        if guard:
            # Logs may contain synthetic prompt responses, but never copy auth
            # or config state into the durable capture directory.
            try:
                for source in guard.root.glob("child-*.log"):
                    shutil.copyfile(source, out / source.name)
                # Preserve bounded native history schema from this generated
                # private home only. Never follow a rollout symlink or inspect
                # the caller's normal CODEX_HOME/conversations.
                private_home = (guard.root / "codex-home").resolve()
                rollouts = []
                for index, source in enumerate(private_home.glob("sessions/*/*/*/*.jsonl")):
                    if index >= 16:
                        break
                    if source.is_symlink() or not source.resolve().is_relative_to(private_home):
                        raise RuntimeError("generated rollout escaped private fixture home")
                    with source.open("rb") as history:
                        head = history.read(128 * 1024)
                        history.seek(0, os.SEEK_END)
                        size = history.tell()
                        history.seek(max(0, size - 256 * 1024))
                        tail = history.read(256 * 1024)
                    prefix = f"owned-rollout-{index}"
                    (out / (prefix + "-head.jsonl")).write_bytes(head)
                    (out / (prefix + "-tail.jsonl")).write_bytes(tail)
                    rollouts.append(dict(relative_path=str(source.relative_to(private_home)),
                                         bytes=size, capture_prefix=prefix))
                manifest["owned_rollouts"] = rollouts
            except (OSError, RuntimeError) as exc:
                manifest["capture_error"] = str(exc)
            try:
                manifest["cleanup"] = guard.close()
            except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
                manifest["cleanup_error"] = str(exc)
        write(out / "manifest.json", manifest)
        print(json.dumps({"capture": str(out), "outcome": manifest["outcome"],
                          "cleanup": manifest["cleanup"], "error": manifest.get("error")}))
    return 0 if manifest["cleanup"] and manifest["outcome"] in (
        "credential_free_transport_observed", "live_turn_observed", "native_approval_cancel_observed",
        "hook_config_sources_observed", "native_trusted_additive_brief_observed",
        "native_trusted_brief_and_wake_off_observed", "scoped_mesimon_mcp_observed", "native_patch_gate_observed",
        "native_active_input_observed") else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "relay":
        relay(*sys.argv[2:])
    elif len(sys.argv) > 1 and sys.argv[1] == "hook":
        fixture_hook()
    else:
        sys.exit(main())
