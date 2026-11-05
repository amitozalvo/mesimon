#!/usr/bin/env python3
"""Seed the demo board, or stop its daemon, over the daemon's own socket.

Usage: seed.py board <repo> [tape]   create the tape's tickets (demo's by
                                     default) and start the background
                                     agent on one of them
       seed.py prs <repo>            print where each ticket and its pull
                                     requests stand (a failed prs take)
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

# reddit.tape: the crown board with the demo ticket in TODO. The person
# starts DEMO from its card (Shift+Enter), crowns the idle agent and tells
# it what to do; the crown files two tickets and starts "Custom slugs". The
# demo's agent raises its hand to the person, who answers from the card;
# the crown's worker asks the crown, which answers it; the merge train lands
# both. "Expire old links" is absent, so the crown starts "Custom slugs",
# and nothing waits in IN PROGRESS with no agent on it: that is a staged
# thing (the author, T-1125). The crown's agent has read the board and
# ended its turn before the take opens (`settled`).
REDDIT_BOARD = [
    ("TODO", DEMO, "FEATURE",
     "Scripts scrape the stats table. Print the same numbers as JSON when "
     "--json is passed, one object per link."),
    ("TODO", "Custom slugs", "FEATURE",
     "Let a user pick the slug instead of getting a random one."),
    ("TODO", "Document config", None,
     "Every key in shortlink.toml, its default, and an example."),
    ("IN PROGRESS", CROWN, "MILESTONE", "Decide what 1.0 needs, and file it."),
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

# prs.tape: a board that merges through pull requests, read off a forge
# that is forge.py (a stand-in `gh`), with an environment column for each
# place a merge goes. IN PROGRESS holds a card until its pipeline passes,
# REVIEW until a teammate merges it on the forge, STAGING and PRODUCTION
# take it as the merge reaches them. The crown ("Ship 1.1") starts the two
# TODO tickets with a worker each before the take opens; one worker's PR
# fails its pipeline, which wakes the crown, which sends the worker back.
# The cards already past REVIEW carry pull requests of their own, named
# under a Summary in a note, as an agent names a PR it opened: PRS says
# where each one's clock stands when the board opens.
PRS_CROWN = "Ship 1.1"
PRS_BOARD = [
    ("TODO", "Rate limiting", "FEATURE",
     "At most 20 new links a minute per client IP; answer 429 past that."),
    ("TODO", "Fix slash 404", "BUG",
     "GET /abc/ should redirect like GET /abc."),
    ("TODO", "Webhook on click", "FEATURE", "POST to a URL on every click."),
    ("TODO", "Clicks per day", "FEATURE", "A per-day breakdown in stats."),
    ("TODO", "Document config", None,
     "Every key in shortlink.toml, its default, and an example."),
    ("IN PROGRESS", PRS_CROWN, "MILESTONE",
     "Run the 1.1 sprint: start what is open, hear the forge, keep it moving."),
    ("IN PROGRESS", "Admin login", "FEATURE", "Protect /admin with a password."),
    ("REVIEW", "Bulk import", "FEATURE", "Shorten every URL in a text file."),
    ("STAGING", "Export as CSV", "FEATURE",
     "Every link with its clicks, as CSV, for the spreadsheet people."),
    ("PRODUCTION", "QR code per link", "FEATURE", "An SVG QR code at /<slug>.svg."),
    ("PRODUCTION", "Set up CI", None, "Run the tests on every push."),
]
# Each card's pull request on the stand-in forge (forge.py): `ago` is the
# seconds since its last push when the take opens, `run_s` how long
# its pipeline runs, `merge_after` the seconds from green to a teammate's
# merge (None: nobody merges it in the take), `deploy` the seconds from the
# merge to each environment. Admin login goes green, merges and deploys
# during the take; Export as CSV reaches production during it.
PRS = {
    "Admin login": dict(number=40, ago=2, run_s=16, review="APPROVED", merge_after=6,
                        deploy={"staging": 6, "production": 12}),
    "Bulk import": dict(number=39, ago=60, run_s=5, review="REVIEW_REQUIRED",
                        merge_after=None, deploy={}),
    "Export as CSV": dict(number=38, ago=120, run_s=5, review="APPROVED", merge_after=1,
                          deploy={"staging": 2, "production": 150}),
    "QR code per link": dict(number=37, ago=600, run_s=5, review="APPROVED", merge_after=1,
                             deploy={"staging": 2, "production": 5}),
    "Set up CI": dict(number=35, ago=900, run_s=5, review="APPROVED", merge_after=1,
                      deploy={"staging": 2, "production": 5}),
}
PR_URL = "https://github.com/demo/shortlink/pull/{}"

# agent.tape steps into BUSY's agent, so it needs no board of its own.
BOARDS = {"demo": TICKETS, "agent": TICKETS, "search": SEARCH,
          "crown": CROWN_BOARD, "ticket-page": PAGE_BOARD, "reddit": REDDIT_BOARD,
          "prs": PRS_BOARD}
# The ticket whose agent is already at work, where it is not BUSY.
AT_WORK = {"crown": CROWN, "reddit": CROWN, "prs": PRS_CROWN}

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


def board(repo, tape="demo", start=True):
    key_file = os.environ.get("DEMO_KEY_FILE")
    tickets = BOARDS[tape]
    if key_file:
        tickets = [(c, t, g, REAL_BUSY if t == BUSY else d) for c, t, g, d in tickets]
    w = Wire(repo)
    # A new board opens on Setup (T-816); the person on the tape has been
    # through it, with its answers for one person on one repository, or for
    # a team that merges through pull requests.
    prs = tape == "prs"
    w.ask({"cmd": "apply_setup", "template": "kanban", "noun": "PR",
           "merge_into": "pull_requests" if prs else "main",
           "workspace": "worktree", "agent_tools": "full"})
    if prs:
        columns(w)
    if tape in ("reddit", "demo"):
        # DONE reclaims nothing: a person's worker stays in its worktree
        # after the merge, and the card would say the worktree is kept
        # (the peek shows the card's words, not the ticket page's).
        done = column_settings(w)["DONE"]
        done.update(reclaim=False, free_worktree=False)
        w.ask({"cmd": "set_column_settings", "name": "DONE", "settings": done})
    ids = {}
    for column, title, tag, description in tickets:
        # Only TODO takes a new ticket (T-764); a card further on is filed
        # there and moved.
        made = w.ask({"cmd": "create_ticket", "column": "TODO", "title": title,
                      "workspace": "worktree"})
        ids[title] = made["id"]
        # An agent may not undo a move made in the last minute
        # (daemon/src/movegate.rs), and the crown take moves an IN PROGRESS
        # card back to TODO: that card comes by way of REVIEW, so its last
        # move is not the one the crown reverses.
        if column == "IN PROGRESS":
            w.ask({"cmd": "move_ticket", "id": made["id"], "column": "REVIEW"})
        if column != "TODO":
            w.ask({"cmd": "move_ticket", "id": made["id"], "column": column})
        w.ask({"cmd": "write_note", "ticket": made["id"], "note": None,
               "text": description})
        if tag:
            w.ask({"cmd": "set_tag", "id": made["id"], "group": 1, "name": tag})
        if title in PRS and prs:
            # The PR the card already carries, named as an agent names one
            # it opened: under a Summary heading in a note.
            w.ask({"cmd": "write_note", "ticket": made["id"], "note": None,
                   "text": f"## Summary\n{PR_URL.format(PRS[title]['number'])}\n"})
    if prs:
        forge(repo, tickets)
    # The first-run offer to write an agent brief would sit in the header.
    w.ask({"cmd": "ignore_brief_offer"})
    # The first step into an agent practises the way back first; the person
    # on the tape has done that once already (agent.tape steps in).
    w.ask({"cmd": "gate_passed"})
    # ci/sandbox.sh with the real claude starts none: an agent spends usage.
    if not start:
        return
    # One agent already at work, so the board opens with a spinner on it.
    at_work = ids[AT_WORK.get(tape, BUSY)]
    # The pull-request take's crown was crowned before the take, as the
    # person's ^o does; its first turn runs the board.
    if tape == "prs":
        w.ask({"cmd": "crown_ticket", "id": at_work})
    tree = sign_in(w, repo, key_file, at_work) if key_file else None
    w.ask({"cmd": "spawn_session", "ticket": at_work, "kind": "claude",
           "submit_prompt": True, "plan": False})
    if tree:
        at_work_started(w, repo, at_work, tree)
    if tape == "ticket-page":
        noted(w, ids)
    if tape == "reddit":
        settled(w, ids[CROWN])
    if tape == "prs":
        sprinting(w, repo, ids)


def columns(w):
    """The pull-request board's columns: DONE becomes PRODUCTION, STAGING
    goes in after REVIEW, and each column takes a card when the forge says
    so (`arrives_on`, T-739): REVIEW once the pipeline passes, STAGING and
    PRODUCTION once the merge is deployed there. A turn's end moves nothing
    out of IN PROGRESS and a working agent nothing out of REVIEW: the
    forge places the cards from there."""
    w.ask({"cmd": "rename_column", "name": "DONE", "to": "PRODUCTION"})
    w.ask({"cmd": "add_column", "name": "STAGING", "after": "REVIEW"})
    settings = column_settings(w)
    for name, change in [
        # TODO collapsed to its count: four columns fit the take's 120
        # cells, and the sprint is what the clip is about.
        ("TODO", {"collapsed": True}),
        ("IN PROGRESS", {"on_done": None}),
        ("REVIEW", {"on_working": None, "arrives_on": "pipeline_passes"}),
        ("STAGING", {"arrives_on": "deployed", "arrives_env": "staging"}),
        ("PRODUCTION", {"arrives_on": "deployed", "arrives_env": "production"}),
    ]:
        s = {**settings[name], **change}
        s = {k: v for k, v in s.items() if v is not None}
        w.ask({"cmd": "set_column_settings", "name": name, "settings": s})


def column_settings(w):
    """Each column's settings, which sit beside its name and order in the
    snapshot, whole: set_column_settings takes the whole struct."""
    return {c["name"]: {k: v for k, v in c.items() if k not in ("name", "order")}
            for c in w.ask({"cmd": "snapshot"})["board"]["columns"]}


def forge(repo, tickets):
    """Seed the stand-in forge (forge.py, on PATH as `gh`) with the pull
    requests the cards carry, each at its own point in time."""
    spec = {"repo": repo, "prs": []}
    for _, title, _, _ in tickets:
        if title in PRS:
            slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
            spec["prs"].append({**PRS[title], "title": title, "branch": f"feature/{slug}"})
    subprocess.run(["gh", "__seed"], input=json.dumps(spec), text=True, check=True)


def sprinting(w, repo, ids):
    """Wait for the pull-request take's crown to end its first turn, with
    a worker at work on each of the two tickets it starts."""
    workers = {ids["Rate limiting"], ids["Fix slash 404"]}
    deadline = time.time() + 40
    while True:
        snap = w.ask({"cmd": "snapshot"})["board"]
        running = {s["ticket"] for s in snap["sessions"] if s["state"]["state"] == "running"}
        crown = next((s for s in snap["sessions"] if s["ticket"] == ids[PRS_CROWN]), None)
        if workers <= running and crown and crown["state"]["state"] == "idle":
            return
        if time.time() > deadline:
            sys.exit("seed: the crown did not start both workers:\n"
                     + (pane(repo, crown) if crown else "(no crown session)"))
        time.sleep(0.2)


def settled(w, ticket):
    """Wait for the reddit take's crown agent to end its first turn: it
    read the board and said what 1.0 is missing, and the column's rule
    carried its card to REVIEW, where the take crowns it and asks."""
    deadline = time.time() + 30
    while True:
        snap = w.ask({"cmd": "snapshot"})
        t = next(t for t in snap["board"]["tickets"] if t["id"] == ticket)
        if t["column"] == "REVIEW":
            return
        if time.time() > deadline:
            sys.exit("seed: the crown's agent did not end its first turn")
        time.sleep(0.2)


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
    # The daemon probes the claude on PATH for its road as it starts
    # (daemon/src/modroad.rs), and each run of claude there writes this same
    # file: wait for the probe's verdict, then put the answers on top of
    # what the file holds.
    verdict = os.path.join(os.environ["HOME"], ".local/state/mesimon", proj16(repo),
                           "mod", "probe.json")
    deadline = time.time() + 60
    while not os.path.exists(verdict):
        if time.time() > deadline:
            sys.exit("seed: the daemon never finished probing claude's road")
        time.sleep(0.2)
    path = os.path.join(home, ".claude.json")
    try:
        with open(path) as f:
            config = {**json.load(f), **config}
    except FileNotFoundError:
        pass
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
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


def prs(repo):
    """What the board knows of its tickets' pull requests, for tuning the
    pull-request take: the column each card is in, and each PR's state,
    pipeline, review and the environments its merge reached."""
    snap = Wire(repo, wait=0).ask({"cmd": "snapshot"})
    cols = {t["id"]: (t["short_key"], t["title"], t["column"]) for t in snap["board"]["tickets"]}
    for key, title, column in sorted(cols.values()):
        print(f"{key:5} {column:12} {title}")
    for p in snap.get("prs", []):
        key, title, column = cols.get(p["ticket"], ("?", "?", "?"))
        print(f"{key:5} #{p['number']} {p['state']} pipeline={p['pipeline']} "
              f"review={p['review']} deployed={p.get('deployed', [])} "
              f"merged={p.get('merged_oid', '')[:7]} reviewed={p.get('reviewed', False)}")


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
    elif args[:1] == ["prs"] and len(args) == 2:
        prs(args[1])
    elif args[:1] == ["stop"] and len(args) == 2:
        stop(args[1])
    else:
        sys.exit(__doc__)
