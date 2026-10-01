#!/usr/bin/env python3
"""The sandbox host's board, run inside its container by ci/sandbox.sh.

Usage: host.py up <pin>   the repo, the daemon, the demo's board, a sign-in
                          to the sandbox relay and Remote Control on; each
                          step is skipped when it is already done
       host.py pair       a fresh pairing link (one use, ten minutes)
       host.py stop       shut the daemon down

Speaks the daemon's wire protocol through assets/demo/seed.py's `Wire`, and
seeds the README demo's board with it. The agents are the real claude when
ci/sandbox.sh left a token in HOME, and the scripted stand-in
(stub-claude.py, MESIMON_CLAUDE_BIN) when it did not.
"""

import json
import os
import subprocess
import sys
import time

sys.path.insert(0, "/work/assets/demo")
import seed  # noqa: E402  (assets/demo/seed.py)

REPO = "/root/shortlink"
MESIMON = "/target/debug/mesimon"
# The relay shares the host's network (compose.yaml), so it is localhost.
RELAY = "localhost:8443"
STUB = "MESIMON_CLAUDE_BIN" in os.environ
# The default tier: Sonnet at medium effort, so an agent started here costs
# little of the subscription. A fixed id, so `up` adds it once.
TIER = {"id": "01K6SXSNTMED00000000000000", "name": "sonnet",
        "provider": "claude_code", "model": "sonnet", "effort": "medium"}

# What the daemon's `$SHELL -l` hands every pane (daemon/src/shellenv.rs).
PROFILE = """\
# Written by ci/sandbox/host.py on every `ci/sandbox.sh up`.
# claude's sign-in: a `claude setup-token` token ci/sandbox.sh copied in.
if [ -r "$HOME/.claude-token" ]; then
    read -r CLAUDE_CODE_OAUTH_TOKEN <"$HOME/.claude-token"
    export CLAUDE_CODE_OAUTH_TOKEN
fi
# The image's claude stays the version it was built with.
export DISABLE_AUTOUPDATER=1
# claude skips permissions as root only inside a sandbox, and this is one.
export IS_SANDBOX=1
"""


def answers():
    """Whether a daemon answers `hello`. One shutting down can still take
    the connection and then close it unanswered."""
    try:
        seed.Wire(REPO, wait=0)
        return True
    except (SystemExit, OSError, ValueError):
        return False


def daemon_up():
    """Start the board's daemon unless one answers. Detached: it outlives
    this `docker compose exec`, as one a TUI spawns does."""
    if answers():
        return
    log = open("/root/daemon.out", "ab")
    subprocess.Popen([MESIMON, "daemon", "--repo", REPO], stdin=subprocess.DEVNULL,
                     stdout=log, stderr=log, start_new_session=True)


def repo():
    if os.path.isdir(REPO):
        return
    os.makedirs(REPO)
    subprocess.run(["cp", "-R", "/work/assets/demo/project/.", REPO], check=True)
    for argv in (["init", "-q"], ["add", "-A"],
                 ["commit", "-q", "-m", "shortlink: a tiny URL shortener"]):
        subprocess.run(["git", "-C", REPO, *argv], check=True)


def claude_home():
    """The profile, and claude's first-run screens answered in its config:
    onboarding done, a theme, the auto-mode notice seen, and HOME trusted,
    which covers the checkout and every worktree under the state dir. A
    screen left on is a SETUP card on the board."""
    with open("/root/.profile", "w") as f:
        f.write(PROFILE)
    path = "/root/.claude.json"
    try:
        with open(path) as f:
            config = json.load(f)
    except (OSError, ValueError):
        config = {}
    wanted = {"hasCompletedOnboarding": True, "theme": "dark",
              "hasSeenAutoDefaultNotice": True}
    projects = config.setdefault("projects", {})
    trusted = all(projects.get(p, {}).get("hasTrustDialogAccepted") for p in ("/root", REPO))
    if trusted and all(config.get(k) == v for k, v in wanted.items()):
        return
    config.update(wanted)
    for p in ("/root", REPO):
        projects.setdefault(p, {})["hasTrustDialogAccepted"] = True
    fd = os.open(path + ".new", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(config, f, indent=2)
    os.replace(path + ".new", path)


def until(w, what, ready, failed=lambda s: None, wait=30.0):
    """Poll the snapshot until `ready(snapshot)`; exit with `failed`'s words."""
    deadline = time.time() + wait
    while True:
        snap = w.ask({"cmd": "snapshot"})
        if ready(snap):
            return snap
        why = failed(snap)
        if why or time.time() > deadline:
            sys.exit(f"sandbox: {what}: {why or 'timed out'}")
        time.sleep(0.25)


def remote(w, action):
    return w.ask({"cmd": "mesophon", "action": {"action": action}})["info"]


def stop():
    """Shut the daemon down and wait until it has let go of its socket, so
    a daemon started next is not refused as a second one."""
    seed.stop(REPO)
    deadline = time.time() + 10
    while time.time() < deadline:
        if not answers():
            return
        time.sleep(0.1)
    sys.exit("sandbox: the daemon did not stop")


def pair(w):
    info = remote(w, "pair")
    if not info.get("code"):
        sys.exit(f"sandbox: no pairing code: {info.get('error') or info}")
    print(f"{info['origin'].rstrip('/')}/#pair={info['code']}")


def up(pin):
    repo()
    claude_home()
    # Always a fresh daemon: the newest build, and the shell environment the
    # profile gives now (a token added since reaches every new pane).
    if answers():
        stop()
    daemon_up()
    w = seed.Wire(REPO)
    if not w.ask({"cmd": "snapshot"})["board"]["tickets"]:
        seed.board(REPO, "demo", start=STUB)
        print("sandbox: seeded the demo board", file=sys.stderr)
    tiers = w.ask({"cmd": "snapshot"})["machine_tiers"]
    if not any(t["id"] == TIER["id"] for t in tiers.get("tiers", [])):
        w.ask({"cmd": "save_tier", "scope": "machine", "tier": TIER})
        w.ask({"cmd": "set_default_tier", "scope": "machine", "id": TIER["id"]})
        print("sandbox: the default tier is sonnet, medium effort", file=sys.stderr)
    team = w.ask({"cmd": "snapshot"})["team"]
    if not (team.get("device") or {}).get("registered"):
        w.ask({"cmd": "team_sign_in", "relay": f"{RELAY} {pin}",
               "display_name": "sandbox"})
        until(w, "sign-in",
              lambda s: (s["team"].get("device") or {}).get("registered"),
              lambda s: not s["team"].get("busy") and s["team"].get("error"))
        print("sandbox: signed in to the sandbox relay", file=sys.stderr)
    if not remote(w, "status")["enabled"]:
        remote(w, "enable")
    until(w, "Remote Control", lambda s: s["mesophon"]["connected"],
          lambda s: s["mesophon"].get("error"))


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["up"] and len(args) == 2:
        up(args[1])
    elif args == ["pair"]:
        pair(seed.Wire(REPO))
    elif args == ["stop"]:
        stop()
    else:
        sys.exit(__doc__)
