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
import re
import socket
import subprocess
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

# agent.tape steps into BUSY's agent, so it needs no board of its own.
BOARDS = {"demo": TICKETS, "agent": TICKETS, "search": SEARCH,
          "crown": CROWN_BOARD, "ticket-page": PAGE_BOARD}
# The ticket whose agent is already at work, where it is not BUSY.
AT_WORK = {"crown": CROWN}

# With the real claude (record.sh with a key file), BUSY's agent works on
# this instead: it has to still be at work when the take steps into it, and
# the stand-in's one-line task can be done, or questioned, before that.
REAL_BUSY = (
    "At most 20 new links a minute per client IP; answer 429 past that. "
    "There is no server yet: write server.py on the standard library "
    "(POST /shorten, GET /<slug>), put the limit in front of /shorten, and "
    "cover both with unittest tests. Run them.")


def proj16(repo):
    return hashlib.sha256(os.path.realpath(repo).encode()).hexdigest()[:16]


def rt_dir(repo):
    return f"/tmp/mesimon-{os.getuid()}/{proj16(repo)}"


def sock_path(repo):
    return os.path.join(rt_dir(repo), "orch.sock")


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
    key_file = os.environ.get("DEMO_KEY_FILE")
    tickets = BOARDS[tape]
    if key_file:
        tickets = [(c, t, g, REAL_BUSY if t == BUSY else d) for c, t, g, d in tickets]
    w = Wire(repo)
    ids = {}
    for column, title, tag, description in tickets:
        made = w.ask({"cmd": "create_ticket", "column": column, "title": title,
                      "workspace": "worktree"})
        ids[title] = made["id"]
        w.ask({"cmd": "write_note", "ticket": made["id"], "note": None,
               "text": description})
        if tag:
            w.ask({"cmd": "set_tag", "id": made["id"], "group": 1, "name": tag})
    # The first-run offer to write an agent brief would sit in the header.
    w.ask({"cmd": "ignore_brief_offer"})
    # The first step into an agent practises the way back first; the person
    # on the tape has done that once already (agent.tape steps in).
    w.ask({"cmd": "gate_passed"})
    # One agent already at work, so the board opens with a spinner on it.
    at_work = ids[AT_WORK.get(tape, BUSY)]
    tree = sign_in(w, repo, key_file, at_work) if key_file else None
    w.ask({"cmd": "spawn_session", "ticket": at_work, "kind": "claude",
           "submit_prompt": True, "plan": False})
    if tree:
        at_work_started(w, repo, at_work, tree)
    if tape == "ticket-page":
        noted(w, ids)


def worktree(repo, ticket):
    """Where the daemon cuts this ticket's worktree: `<state>/worktrees/
    <KEY>-<slug>`, the slug as core/src/workspace.rs::slug makes it from a
    short ASCII title. `at_work_started` checks the agent runs there."""
    slug = re.sub(r"[^a-z0-9._]+", "-", ticket["title"].lower()).strip("-.")
    state = os.path.join(os.environ["HOME"], ".local/state/mesimon", proj16(repo))
    return os.path.join(state, "worktrees", f"{ticket['short_key']}-{slug}")


def sign_in(w, repo, key_file, ticket):
    """Answer the real claude's first-run screens before it starts, in the
    sandbox's claude config: onboarding done, a theme, the key approved
    (claude keeps its last 20 characters), the auto-mode and fast-mode
    notices seen, and the agent's worktree trusted. A screen left on is a
    SETUP card on the board and a take that stalls. Returns the worktree."""
    snap = w.ask({"cmd": "snapshot"})
    tree = worktree(repo, next(t for t in snap["board"]["tickets"] if t["id"] == ticket))
    with open(key_file) as f:
        tail = f.read().strip()[-20:]
    config = {
        "hasCompletedOnboarding": True,
        "theme": "dark",
        "customApiKeyResponses": {"approved": [tail], "rejected": []},
        "hasSeenAutoDefaultNotice": True,
        "penguinModeOrgEnabled": True,
        # The path as the daemon spells it, and as the kernel does (/tmp is
        # /private/tmp on macOS): claude looks up the one its cwd gives.
        "projects": {p: {"hasTrustDialogAccepted": True}
                     for p in {tree, os.path.realpath(tree)}},
    }
    home = os.environ["CLAUDE_CONFIG_DIR"]
    os.makedirs(home, mode=0o700, exist_ok=True)
    fd = os.open(os.path.join(home, ".claude.json"), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(config, f, indent=2)
    # The key reaches a pane through the shell's answer, which the daemon
    # asks for as it starts; a claude spawned before that lands would start
    # signed out.
    env_file = os.path.join(rt_dir(repo), "shellenv.env")
    deadline = time.time() + 20
    while not os.path.exists(env_file):
        if time.time() > deadline:
            sys.exit("seed: the daemon never read the sandbox's shell environment")
        time.sleep(0.1)
    return tree


def at_work_started(w, repo, ticket, tree):
    """Wait for the real claude to take its first prompt. A screen it stops
    at fails the take here, with the screen, not as a timeout in the tape."""
    deadline = time.time() + 60
    while True:
        snap = w.ask({"cmd": "snapshot"})
        s = next((s for s in snap["board"]["sessions"] if s["ticket"] == ticket), None)
        if s is not None:
            if os.path.realpath(s["cwd"]) != os.path.realpath(tree):
                sys.exit(f"seed: claude runs in {s['cwd']}, but the trusted folder is {tree}")
            state = s["state"]["state"]
            if state == "running" and not s.get("pending_submit"):
                return
            if state not in ("spawning", "running", "idle"):
                sys.exit(f"seed: claude stopped at {s['state']}:\n{pane(repo, s)}")
        if time.time() > deadline:
            sys.exit(f"seed: claude did not start work:\n{pane(repo, s) if s else ''}")
        time.sleep(0.2)


def pane(repo, session):
    """What is on an agent's screen: the daemon's private tmux names each
    session by the first 16 hex digits of its id."""
    sid16 = session["id"].replace("-", "")[:16]
    tmux = os.environ.get("MESIMON_TMUX_BIN", "tmux")
    return subprocess.run([tmux, "-S", os.path.join(rt_dir(repo), "tmux.sock"),
                           "capture-pane", "-p", "-t", sid16],
                          capture_output=True, text=True).stdout


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
