"""A client for one board's daemon, over `orch.sock` (T-588).

Standard library only. It speaks the protocol `crates/mesimon/src/state.rs`
and the e2e harness's `TestClient` speak: newline-delimited JSON, a `Hello`
first, then one `Envelope { principal, command }` per line, each answered by
one `Response` line. A connection that sent `Subscribe` also receives
`{"event": "board_changed"}` lines between replies; `request` skips them.

Every shape here is read from `crates/mesimon-core/src/command.rs`:
`Command` is `#[serde(tag = "cmd", rename_all = "snake_case")]`, `Response`
is `tag = "resp"`, `Principal` is `tag = "kind"`. The principal is always
`local`: the rig acts as the person at the desk, and the crown under test
acts as itself through its own MCP tools.

    from wire import Wire, board_paths
    paths = board_paths("/path/to/checkout")
    with Wire(paths.orch_sock) as w:
        board = w.board()
"""

import hashlib
import json
import os
import secrets
import socket
import time

PROTOCOL_VERSION = 2  # core/src/command.rs::PROTOCOL_VERSION
LOCAL = {"kind": "local"}


class WireError(Exception):
    """The daemon answered `Response::Err`; the message is its words."""


class BoardPaths:
    """The per-repo layout of `daemon/src/paths.rs` (D33b)."""

    def __init__(self, repo):
        canon = os.path.realpath(repo)
        self.repo = canon
        self.proj16 = hashlib.sha256(canon.encode()).hexdigest()[:16]
        self.rt_dir = f"/tmp/mesimon-{os.getuid()}/{self.proj16}"
        self.state_dir = os.path.join(
            os.environ["HOME"], ".local/state/mesimon", self.proj16
        )
        self.board_dir = os.path.join(canon, ".mesimon")
        self.orch_sock = os.path.join(self.rt_dir, "orch.sock")
        self.tmux_sock = os.path.join(self.rt_dir, "tmux.sock")
        self.feed = os.path.join(self.state_dir, "activity.jsonl")
        self.daemon_log = os.path.join(self.state_dir, "daemon.log")
        self.mod_root = os.path.join(self.state_dir, "mod")


def board_paths(repo):
    return BoardPaths(repo)


def ulid():
    """A ULID string, as a tier a person makes carries for its id."""
    alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    n = (int(time.time() * 1000) << 80) | secrets.randbits(80)
    return "".join(alphabet[(n >> (5 * i)) & 31] for i in reversed(range(26)))


class Wire:
    """One connection. `timeout` bounds every read, so a dead daemon is an
    exception, never a hang."""

    def __init__(self, sock_path, client="rig", timeout=15.0, connect_for=5.0):
        deadline = time.time() + connect_for
        while True:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.connect(sock_path)
                break
            except OSError:
                s.close()
                if time.time() >= deadline:
                    raise
                time.sleep(0.1)
        s.settimeout(timeout)
        self.sock = s
        self.reader = s.makefile("r", encoding="utf-8", newline="\n")
        self.hello = self.request({"cmd": "hello", "version": PROTOCOL_VERSION, "client": client})

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def close(self):
        try:
            self.reader.close()
            self.sock.close()
        except OSError:
            pass

    def _line(self):
        line = self.reader.readline()
        if not line:
            raise ConnectionError("the daemon closed the connection")
        return json.loads(line)

    def request(self, command, timeout=None):
        """Send one command as the person; return the response dict, or raise
        `WireError` on `Response::Err`."""
        if timeout is not None:
            self.sock.settimeout(timeout)
        envelope = {"principal": LOCAL, "command": command}
        self.sock.sendall((json.dumps(envelope) + "\n").encode())
        while True:
            msg = self._line()
            if "event" in msg and "resp" not in msg:
                continue
            if msg.get("resp") == "err":
                raise WireError(msg.get("message", "refused"))
            return msg

    # ---- Reads.

    def snapshot(self):
        """`Response::Board`: the board and everything that rides with it."""
        return self.request({"cmd": "snapshot"})

    def board(self):
        return self.snapshot()["board"]

    def read_note(self, ticket, note):
        return self.request({"cmd": "read_note", "ticket": ticket, "note": note})["text"]

    # ---- Tickets.

    def create_ticket(self, column, title, workspace=None, tier=None):
        cmd = {"cmd": "create_ticket", "column": column, "title": title, "workspace": workspace}
        if tier is not None:
            cmd["tier"] = tier
        return self.request(cmd)["id"]

    def write_note(self, ticket, text, note=None):
        """Create (`note=None`) or replace a note; the first is the
        description. Returns the note's id."""
        return self.request(
            {"cmd": "write_note", "ticket": ticket, "note": note, "text": text}
        ).get("note")

    def set_workspace(self, ticket, workspace):
        return self.request({"cmd": "set_workspace", "id": ticket, "workspace": workspace})

    def set_manual_merge(self, ticket, on):
        return self.request({"cmd": "set_manual_merge", "id": ticket, "on": on})

    def move(self, ticket, column, before=None):
        return self.request({"cmd": "move_ticket", "id": ticket, "column": column, "before": before})

    def archive(self, ticket):
        return self.request({"cmd": "archive_ticket", "id": ticket})

    def crown(self, ticket):
        return self.request({"cmd": "crown_ticket", "id": ticket})

    # ---- Board settings.

    def save_tier(self, scope, tier):
        """`scope` is `board` or `machine`; `tier` is `tier::Tier`'s shape:
        id, name, provider (`claude_code`|`codex`), model, effort,
        description."""
        return self.request({"cmd": "save_tier", "scope": scope, "tier": tier})

    def set_default_tier(self, scope, tier_id):
        return self.request({"cmd": "set_default_tier", "scope": scope, "id": tier_id})

    def set_crown_sends(self, on):
        return self.request({"cmd": "set_crown_sends", "on": on})

    def set_crown_answers(self, on):
        return self.request({"cmd": "set_crown_answers", "on": on})

    def set_crown_budget(self, budget):
        return self.request({"cmd": "set_crown_budget", "budget": budget})

    # ---- Sessions.

    def spawn(self, ticket, kind="claude", submit_prompt=True, plan=False):
        """What the composer's Shift+Enter sends: the agent starts on the
        ticket's title and description as its first prompt. `c` sends the
        same with `submit_prompt=False` (the title typed, not submitted).
        Answers `spawned` or `provisioning` (a worktree being cut)."""
        return self.request(
            {"cmd": "spawn_session", "ticket": ticket, "kind": kind,
             "submit_prompt": submit_prompt, "plan": plan}
        )

    def prompt(self, ticket, text, queued=False):
        """The person's words to the ticket's agent, as the ticket page's
        composer sends them."""
        return self.request(
            {"cmd": "prompt_session", "ticket": ticket, "text": text, "queued": queued}
        )

    def sleep(self, session):
        return self.request({"cmd": "sleep_session", "id": session})

    def mod_ping(self, session):
        """The whole mod road for one session, timed by the daemon: queued
        for the session's bridge, answered when the mod's pong comes back up
        through `mesimon hook`. Returns the milliseconds."""
        return self.request({"cmd": "mod_ping", "session": session}, timeout=15)["ms"]

    def shutdown(self):
        return self.request({"cmd": "shutdown"})


class Subscription:
    """A second connection that asked for `BoardChanged` pushes. `wait`
    returns True when one arrived inside the timeout."""

    def __init__(self, sock_path):
        self.wire = Wire(sock_path, client="rig-subscriber")
        self.wire.request({"cmd": "subscribe"})

    def wait(self, timeout):
        self.wire.sock.settimeout(timeout)
        try:
            while True:
                msg = self.wire._line()
                if msg.get("event") == "board_changed":
                    return True
        except (socket.timeout, TimeoutError):
            return False

    def close(self):
        self.wire.close()


def read_feed(paths, since=0):
    """The board's activity feed (`activity.jsonl`) from byte `since`:
    (lines as dicts, the offset to read from next). A rotation since the
    last read starts again at the top of the new file."""
    try:
        size = os.path.getsize(paths.feed)
    except FileNotFoundError:
        return [], 0
    if size < since:
        since = 0
    with open(paths.feed, "rb") as f:
        f.seek(since)
        data = f.read()
    lines = []
    end = data.rfind(b"\n") + 1
    for raw in data[:end].splitlines():
        try:
            lines.append(json.loads(raw))
        except ValueError:
            pass
    return lines, since + end


def feed_all(paths):
    """Every line the feed still holds, the rotated file's first."""
    lines = []
    for path in (paths.feed + ".1", paths.feed):
        try:
            with open(path, "rb") as f:
                for raw in f:
                    try:
                        lines.append(json.loads(raw))
                    except ValueError:
                        pass
        except FileNotFoundError:
            pass
    return lines
