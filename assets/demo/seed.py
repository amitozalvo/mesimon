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

# search.tape: a TODO column longer than the screen, so the ticket the tape
# looks for, the last one, starts below the fold. `csv` narrows the picker
# 15, 9, 4, 1 as it is typed; a new title must keep that true.
SEARCH = [
    ("TODO", "Custom slugs", "FEATURE", "Let a link ask for its own slug."),
    ("TODO", "Expire old links", "FEATURE",
     "A link nobody clicked for 30 days should 404 and free its slug."),
    ("TODO", "Document config", None,
     "Every key in shortlink.toml, its default, and an example."),
    ("TODO", "QR code per link", "FEATURE", "An SVG QR code at /<slug>.svg."),
    ("TODO", "Admin login", "FEATURE", "Protect /admin with a password."),
    ("TODO", "Double redirect", "BUG",
     "A slug that points at another slug redirects twice."),
    ("TODO", "Clicks per day", "FEATURE", "A per-day breakdown in stats."),
    ("TODO", "Trim slug spaces", "BUG", "' abc' and 'abc' are two slugs."),
    ("TODO", "Bulk import", "FEATURE", "Shorten every URL in a text file."),
    ("TODO", "API keys", "FEATURE", "One key per script, revocable."),
    ("TODO", "Webhook on click", "FEATURE", "POST to a URL on every click."),
    ("TODO", "Export as CSV", "FEATURE",
     "Every link with its clicks, as CSV, for the spreadsheet people."),
    ("IN PROGRESS", BUSY, "FEATURE",
     "At most 20 new links a minute per client IP; answer 429 past that."),
    ("REVIEW", "Fix slash 404", "BUG",
     "GET /abc/ should redirect like GET /abc."),
    ("DONE", "Set up CI", None, "Run the tests on every push."),
]

# crown.tape: the agent at work is the one the tape crowns, and
# stub-claude.py runs the board from it. "Custom slugs" sits in IN PROGRESS
# with nobody on it, for the crown to move back.
CROWN = "Plan 1.0"
CROWN_BOARD = [
    ("TODO", "Expire old links", "FEATURE",
     "A link nobody clicked for 30 days should 404 and free its slug."),
    ("TODO", "Document config", None,
     "Every key in shortlink.toml, its default, and an example."),
    ("IN PROGRESS", CROWN, "MILESTONE", "Decide what 1.0 needs, and file it."),
    ("IN PROGRESS", "Custom slugs", "FEATURE",
     "Let a user pick the slug instead of getting a random one."),
    ("REVIEW", "Fix slash 404", "BUG",
     "GET /abc/ should redirect like GET /abc."),
    ("DONE", "Set up CI", None, "Run the tests on every push."),
]

# ticket-page.tape: the demo's board, with one card whose agent has already
# left a note on it. The title is this board's own: stub-claude.py keys the
# note-writing turn off it, and the crown clip starts a working agent on
# "Expire old links". The description is one editor row and ends in a
# newline, as a file does, so the tape's Down lands on the line under it.
NOTED = "Retire old links"
PAGE_BOARD = [
    (c, NOTED, g, "A link nobody clicked in 30 days should 404.\n")
    if t == "Expire old links" else (c, t, g, d)
    for c, t, g, d in TICKETS
]

BOARDS = {"demo": TICKETS, "search": SEARCH, "crown": CROWN_BOARD,
          "ticket-page": PAGE_BOARD}
# The ticket whose agent is already at work, where it is not BUSY.
AT_WORK = {"crown": CROWN}


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
    at_work = ids[AT_WORK.get(tape, BUSY)]
    w.ask({"cmd": "spawn_session", "ticket": at_work, "kind": "claude",
           "submit_prompt": True, "plan": False})
    if tape == "ticket-page":
        noted(w, ids)


def noted(w, ids):
    """Run the ticket-page take's agent to the end of its turn: it reads the
    code, writes its note through the board's own tool and stops. The turn
    carries the card TODO → IN PROGRESS → REVIEW by the columns' rules, so
    it is put back at the top of TODO by hand, as a person who has more to
    say about the work would, where the board's cursor starts."""
    ticket = ids[NOTED]
    w.ask({"cmd": "spawn_session", "ticket": ticket, "kind": "claude",
           "submit_prompt": True, "plan": False})
    deadline = time.time() + 30
    while True:
        snap = w.ask({"cmd": "snapshot"})
        t = next(t for t in snap["board"]["tickets"] if t["id"] == ticket)
        if len(t["notes"]) == 2 and t["column"] == "REVIEW":
            break
        if time.time() > deadline:
            sys.exit(f"seed: {NOTED}'s agent did not leave its note")
        time.sleep(0.2)
    w.ask({"cmd": "move_ticket", "id": ticket, "column": "TODO", "before": ids[DEMO]})


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
