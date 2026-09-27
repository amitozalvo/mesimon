#!/bin/sh
# Re-record the README demo, assets/demo.gif, with VHS.
#
#   cargo build --release -p mesimon && assets/demo/record.sh
#
# Needs vhs (brew install vhs), python3 and tmux. Everything the run touches
# lives in a throwaway sandbox: its own HOME (so its own state dir and
# prefs), its own repo, and a scripted stand-in for claude
# (stub-claude.py), so the take is the same every time and spends no tokens.
# The daemon and its private tmux are stopped and the sandbox removed on
# the way out.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
MESIMON=${MESIMON:-$ROOT/target/release/mesimon}
[ -x "$MESIMON" ] || { echo "record: no $MESIMON; cargo build --release -p mesimon" >&2; exit 1; }
command -v vhs >/dev/null || { echo "record: vhs missing; brew install vhs" >&2; exit 1; }

SANDBOX=$(mktemp -d /tmp/msmn-demo.XXXXXX)
REPO=$SANDBOX/shortlink
RT=

cleanup() {
    python3 -B "$HERE/seed.py" stop "$REPO" 2>/dev/null || true
    sleep 1
    [ -n "$RT" ] && [ -S "$RT/tmux.sock" ] && tmux -S "$RT/tmux.sock" kill-server 2>/dev/null || true
    [ -n "$RT" ] && rm -rf "$RT"
    rm -rf "$SANDBOX"
}
trap cleanup EXIT INT TERM

# The project on the board: small, committed, on `main`.
mkdir -p "$SANDBOX/home" "$REPO"
git -C "$REPO" init -q -b main
git -C "$REPO" config user.name "Demo"
git -C "$REPO" config user.email "demo@example.invalid"
cp -R "$HERE/project/." "$REPO/"
git -C "$REPO" add -A
git -C "$REPO" commit -q -m "shortlink: a tiny URL shortener"
REPO=$(cd "$REPO" && pwd -P)
RT=/tmp/mesimon-$(id -u)/$(printf %s "$REPO" | shasum -a 256 | cut -c1-16)

# Nothing of the calling shell's own board may reach the demo's: run from
# inside a mesimon pane, the ticket's variables and tmux's are all set.
for var in $(env | sed -n 's/^\(MESIMON_[A-Z0-9_]*\)=.*/\1/p'); do
    [ "$var" = MESIMON_TMUX_BIN ] || unset "$var"
done
unset TMUX TMUX_PANE

export HOME=$SANDBOX/home
export SHELL=/bin/sh
export MESIMON_CLAUDE_BIN=$HERE/stub-claude.py
export MESIMON_CLAUDE_HOME=$HOME/.claude
export MESIMON_NO_UPDATE_CHECK=1
# The worktree flags (`1 to merge`) refresh every 2 s, not every 10 s, so the
# take does not sit on a finished agent waiting for the merge offer.
export MESIMON_WT_REFRESH_TICKS=8
export MESIMON_THEME=graphite
export MESIMON_COLOR=truecolor
export DEMO_MESIMON=$MESIMON
export DEMO_REPO=$REPO
export DEMO_TERM=$HERE/kitty-term.py

"$MESIMON" daemon --repo "$REPO" >"$SANDBOX/daemon.out" 2>&1 &
python3 -B "$HERE/seed.py" board "$REPO"

cd "$HERE"
vhs demo.tape
ls -l "$ROOT/assets/demo.gif"
