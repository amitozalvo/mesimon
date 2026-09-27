#!/usr/bin/env python3
"""Seed the demo board, or stop its daemon, over the daemon's own socket.

Usage: seed.py board <repo> [tape]   create the tape's tickets (demo's by
                                     default) and start the background
                                     agent on one of them
       seed.py stop <repo>           shut the demo daemon down

Speaks the wire protocol the TUI speaks: newline-delimited JSON
`{"principal", "command"}` envelopes to `orch.sock`, a `hello` first.
"""

import hashlib
import json
import os
import socket
import sys
import time

# The ticket the recording drives. `stub-claude.py` keys its script off this
# title, and it is created first so the board's cursor starts on it.
DEMO = "Stats as JSON"
BUSY = "Rate limiting"

TICKETS = [
    # (column, title, tag, description)
    ("TODO", DEMO, "FEATURE",
     "Scripts scrape the stats table. Print the same numbers as JSON when "
     "--json is passed, one object per link."),
    ("TODO", "Expire old links", "FEATURE",
     "A link nobody clicked for 30 days should 404 and free its slug."),
    ("TODO", "Document config", None,
     "Every key in shortlink.toml, its default, and an example."),
    ("IN PROGRESS", BUSY, "FEATURE",
     "At most 20 new links a minute per client IP; answer 429 past that."),
    ("REVIEW", "Fix slash 404", "BUG",
     "GET /abc/ should redirect like GET /abc."),
    ("DONE", "Set up CI", None, "Run the tests on every push."),
]

BOARDS = {"demo": TICKETS}


def sock_path(repo):
    canon = os.path.realpath(repo).encode()
    proj16 = hashlib.sha256(canon).hexdigest()[:16]
    return f"/tmp/mesimon-{os.getuid()}/{proj16}/orch.sock"


class Wire:
    def __init__(self, repo, wait=15.0):
        path = sock_path(repo)
        deadline = time.time() + wait
        while True:
            try:
                self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.s.connect(path)
                break
            except OSError:
                if time.time() > deadline:
                    sys.exit(f"seed: no daemon at {path}")
                time.sleep(0.1)
        self.r = self.s.makefile("rb")
        self.ask({"cmd": "hello", "version": 2, "client": "demo-seed"})

    def ask(self, command):
        env = {"principal": {"kind": "local"}, "command": command}
        self.s.sendall(json.dumps(env).encode() + b"\n")
        reply = json.loads(self.r.readline())
        if reply.get("resp") == "err":
            sys.exit(f"seed: {command['cmd']} refused: {reply.get('message')}")
        return reply


def board(repo, tape="demo"):
    w = Wire(repo)
    ids = {}
    for column, title, tag, description in BOARDS[tape]:
        made = w.ask({"cmd": "create_ticket", "column": column, "title": title,
                      "workspace": "worktree"})
        ids[title] = made["id"]
        w.ask({"cmd": "write_note", "ticket": made["id"], "note": None,
               "text": description})
        if tag:
            w.ask({"cmd": "set_tag", "id": made["id"], "group": 1, "name": tag})
    # The first-run offer to write an agent brief would sit in the header.
    w.ask({"cmd": "ignore_brief_offer"})
    # One agent already at work, so the board opens with a spinner on it.
    w.ask({"cmd": "spawn_session", "ticket": ids[BUSY], "kind": "claude",
           "submit_prompt": True, "plan": False})


def stop(repo):
    try:
        Wire(repo, wait=0).ask({"cmd": "shutdown"})
    except SystemExit:
        pass


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["board"] and len(args) in (2, 3):
        if args[2:] and args[2] not in BOARDS:
            sys.exit(f"seed: no board for tape {args[2]!r}; add one to BOARDS")
        board(*args[1:])
    elif args[:1] == ["stop"] and len(args) == 2:
        stop(args[1])
    else:
        sys.exit(__doc__)
