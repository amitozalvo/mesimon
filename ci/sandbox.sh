#!/usr/bin/env bash
# A local sandbox for trying unreleased work by hand, Remote Control above
# all: a relay and its PostgreSQL, and a mesimon host signed in to that relay,
# in Docker, built from this working tree. The board is the README demo's.
# This machine's own boards, its sign-ins and the author's long-lived local
# relay (project `mesimon-teams`) are never touched.
#
# The agents are the real claude, on a default tier of Sonnet at medium
# effort, once it has a sign-in: a token from `claude setup-token` (your
# subscription), alone in ~/.config/mesimon/sandbox-claude-token, mode 0600
# (MESIMON_SANDBOX_TOKEN_FILE names another file). The token reaches the
# sandbox over stdin, never a command line. Without one they are the README
# clips' scripted stand-in, which spends nothing, and `up` starts one at work;
# with one, `up` starts no agent.
#
#   ci/sandbox.sh [up]   build what is missing, start, print a pairing link
#   ci/sandbox.sh tui    the sandbox's board in this terminal (q leaves)
#   ci/sandbox.sh pair   a fresh pairing link (one use, ten minutes)
#   ci/sandbox.sh build  rebuild mesimon from this tree, restart its daemon
#   ci/sandbox.sh relay  rebuild the relay image from both trees, restart it
#   ci/sandbox.sh wasm   rebuild Mesophon's Wasm (crates/mesimon-web)
#   ci/sandbox.sh down   stop it; the board and the pairing are kept
#   ci/sandbox.sh reset  stop it and delete its board, sign-in and relay data
#   ci/sandbox.sh compose <args>   any docker compose command on its project
#
# What a change needs before it shows:
#   web/mesophon/*      nothing: the relay serves the checkout. Reload.
#   crates/mesimon-web  `wasm`, then reload.
#   any other crate     `build` (a TUI that is open reconnects by itself).
#   the relay, or a wire change it must classify   `relay`.
#
# The page is http://localhost:8454 (MESIMON_SANDBOX_PORT moves it), a
# browser on this machine only: a phone needs HTTPS. The relay comes from the
# mesimon-relay checkout beside the main one (MESIMON_RELAY_DIR overrides).
# One sandbox per machine: `up` from another checkout or ticket worktree
# moves it there.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
COMPOSE_FILE=$ROOT/ci/sandbox/compose.yaml

die() { echo "sandbox: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }

command -v docker >/dev/null 2>&1 || die "docker is not on PATH" "install Docker Desktop"
docker info >/dev/null 2>&1 || die "the docker daemon is not running" "open -a Docker"

# The git directory the checkout's `.git` names: a ticket worktree's `.git` is
# a pointer into the main checkout's, which the host mounts at the same path.
MESIMON_SANDBOX_GIT=$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)
export MESIMON_SANDBOX_GIT
MAIN=$(dirname "$MESIMON_SANDBOX_GIT")
RELAY_DIR=${MESIMON_RELAY_DIR:-$(dirname "$MAIN")/mesimon-relay}
# The compiler the release is cut with, as ci/test-linux.sh picks it.
MESIMON_SANDBOX_RUST=$(rustc -V 2>/dev/null | awk '{print $2}')
export MESIMON_SANDBOX_RUST=${MESIMON_SANDBOX_RUST:-1}
RELAY_IMAGE=${MESIMON_SANDBOX_RELAY_IMAGE:-mesimon-teams-relay:sandbox}
export MESIMON_SANDBOX_RELAY_IMAGE=$RELAY_IMAGE

TOKEN_FILE=${MESIMON_SANDBOX_TOKEN_FILE:-$HOME/.config/mesimon/sandbox-claude-token}
# compose.yaml passes this through, or leaves it unset for the real claude.
# Worked out on every command, so none of them recreates the host by
# disagreeing with the last.
unset MESIMON_CLAUDE_BIN
[ -r "$TOKEN_FILE" ] || export MESIMON_CLAUDE_BIN=/work/assets/demo/stub-claude.py

compose() { docker compose -f "$COMPOSE_FILE" "$@"; }
host() { compose exec -T host "$@"; }

relay_image() {
    [ -x "$RELAY_DIR/ci/build-local-image.sh" ] ||
        die "no relay checkout at $RELAY_DIR" "set MESIMON_RELAY_DIR"
    echo "sandbox: building $RELAY_IMAGE from $RELAY_DIR and $ROOT (minutes when cold)" >&2
    MESIMON_CORE_DIR=$ROOT "$RELAY_DIR/ci/build-local-image.sh" "$RELAY_IMAGE"
}

wasm() { "$ROOT/ci/build-mesophon.sh"; }

mesimon() {
    echo "sandbox: building mesimon for Linux (minutes the first time)" >&2
    host sh -c 'cd /work && cargo build --locked -p mesimon'
}

# The pin the relay's certificate hashes to; the certificate lives in the
# `relaytls` volume, so it holds across restarts.
pin() {
    local n=0
    until compose exec -T relay test -f /var/lib/relay/tls/relay-cert.pem 2>/dev/null; do
        n=$((n + 1))
        [ "$n" -lt 60 ] || die "the relay did not start" "ci/sandbox.sh compose logs relay"
        sleep 1
    done
    compose exec -T relay mesimon-relay certificate-pin --tls-cert /var/lib/relay/tls/relay-cert.pem
}

# The token goes in through stdin, so it is on no command line, here or there.
token() {
    if [ -r "$TOKEN_FILE" ]; then
        host sh -c 'umask 077 && cat > /root/.claude-token' <"$TOKEN_FILE"
        echo "sandbox: agents are claude, signed in from $TOKEN_FILE" >&2
    else
        host rm -f /root/.claude-token
        echo "sandbox: agents are the scripted stand-in; for claude, save a" \
            "\`claude setup-token\` token to $TOKEN_FILE and run up again" >&2
    fi
}

board() { host python3 -B /work/ci/sandbox/host.py up "$(pin)"; }

pair_link() {
    local link
    link=$(host python3 -B /work/ci/sandbox/host.py pair)
    echo
    echo "  Remote Control: $link"
    echo "  The board:      ci/sandbox.sh tui"
    echo
}

case "${1:-up}" in
    up)
        [ -f "$ROOT/web/mesophon/pkg/mesimon_web_bg.wasm" ] || wasm
        docker image inspect "$RELAY_IMAGE" >/dev/null 2>&1 || relay_image
        compose up -d --build
        mesimon
        token
        board
        pair_link
        ;;
    tui)
        # The terminal's own identity, so the board draws as it does natively.
        envs=()
        for var in TERM COLORTERM TERM_PROGRAM TERM_PROGRAM_VERSION LC_TERMINAL LC_TERMINAL_VERSION; do
            [ -n "${!var:-}" ] && envs+=(-e "$var=${!var}")
        done
        exec docker compose -f "$COMPOSE_FILE" exec "${envs[@]}" -w /root/shortlink host mesimon
        ;;
    pair) pair_link ;;
    build)
        mesimon
        board
        echo "sandbox: the daemon runs the new build" >&2
        ;;
    relay)
        relay_image
        compose up -d relay
        ;;
    wasm) wasm ;;
    down) compose stop ;;
    reset)
        # The board, the sign-in, the relay's database and its certificate;
        # cargo's registry and target stay, so the next `up` is no cold build.
        compose down
        for volume in home pgdata pgsocket relaytls; do
            docker volume rm -f "mesimon-sandbox_$volume" >/dev/null
        done
        ;;
    compose) shift; compose "$@" ;;
    *) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' >&2; exit 2 ;;
esac
