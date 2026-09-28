#!/bin/sh
# Re-record one of the README's clips with VHS: the demo (assets/demo.gif)
# by default, or the tape named, e.g. `search` for assets/demo/search.gif.
#
#   cargo build --release -p mesimon && assets/demo/record.sh [tape]
#
# Needs vhs (brew install vhs), python3 and tmux. Everything the run touches
# lives in a throwaway sandbox: its own HOME (so its own state dir and
# prefs), its own repo, and a scripted stand-in for claude
# (stub-claude.py), so the take is the same every time and spends no tokens.
# The daemon and its private tmux are stopped and the sandbox removed on
# the way out.
#
# The agent clip can run the real claude instead, signed in with a Console
# API key kept in a file (it bills that key's credits, cents a take):
#
#   MESIMON_DEMO_KEY_FILE=~/.config/mesimon-demo.key assets/demo/record.sh agent
#
# The key is read by the sandbox's own login shell, so it rides no command
# line, and nothing of the sign-in on this machine is borrowed. The model is
# Sonnet unless MESIMON_DEMO_MODEL names another. It has to be one with
# claude's auto mode: without it (Haiku) claude asks before its first edit,
# and the take stalls on the question.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
MESIMON=${MESIMON:-$ROOT/target/release/mesimon}
[ -x "$MESIMON" ] || { echo "record: no $MESIMON; cargo build --release -p mesimon" >&2; exit 1; }
command -v vhs >/dev/null || { echo "record: vhs missing; brew install vhs" >&2; exit 1; }
TAPE=${1:-demo}
[ -f "$HERE/$TAPE.tape" ] || { echo "record: no tape $HERE/$TAPE.tape" >&2; exit 1; }

KEY_FILE=${MESIMON_DEMO_KEY_FILE:-}
MODEL=${MESIMON_DEMO_MODEL:-sonnet}
if [ -n "$KEY_FILE" ]; then
    # The other clips need the stand-in's timing: the demo's raised hand,
    # the crown's tool calls, the note on the ticket page.
    [ "$TAPE" = agent ] || { echo "record: only the agent tape runs the real claude" >&2; exit 1; }
    [ -r "$KEY_FILE" ] || { echo "record: cannot read $KEY_FILE" >&2; exit 1; }
    KEY_FILE=$(cd "$(dirname "$KEY_FILE")" && pwd)/$(basename "$KEY_FILE")
    CLAUDE=$(command -v claude) || { echo "record: claude is not on PATH" >&2; exit 1; }
    CLAUDE_DIR=$(dirname "$CLAUDE")
    # All three land in single quotes in the sandbox's profile.
    case $KEY_FILE$CLAUDE_DIR$MODEL in
        *\'*) echo "record: a ' in $KEY_FILE, $CLAUDE_DIR or $MODEL" >&2; exit 1 ;;
    esac
fi

# Spelled the way the kernel spells it (/tmp is /private/tmp on macOS), so
# HOME is a prefix of every cwd under it and claude shows its own as `~/…`.
SANDBOX=$(cd "$(mktemp -d /tmp/msmn-demo.XXXXXX)" && pwd -P)
REPO=$SANDBOX/shortlink
RT=
DAEMON=

cleanup() {
    python3 -B "$HERE/seed.py" stop "$REPO" 2>/dev/null || true
    # The daemon's last writes land in the sandbox; let it finish them.
    n=0
    while [ -n "$DAEMON" ] && kill -0 "$DAEMON" 2>/dev/null && [ "$n" -lt 50 ]; do
        sleep 0.1
        n=$((n + 1))
    done
    if [ -n "$RT" ] && [ -S "$RT/tmux.sock" ]; then
        # An agent writes into the sandbox as it dies (the real claude logs
        # its tool server's exit); wait for each pane's process to go.
        pids=$(tmux -S "$RT/tmux.sock" list-panes -a -F '#{pane_pid}' 2>/dev/null || true)
        tmux -S "$RT/tmux.sock" kill-server 2>/dev/null || true
        for pid in $pids; do
            n=0
            while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 50 ]; do
                sleep 0.1
                n=$((n + 1))
            done
        done
    fi
    [ -n "$RT" ] && rm -rf "$RT"
    # Something else on its way out (the browser VHS drove, which keeps its
    # profile under this HOME) can still write mid-removal; a second pass
    # takes what it left.
    rm -rf "$SANDBOX" 2>/dev/null || { sleep 1; rm -rf "$SANDBOX"; }
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
export MESIMON_CLAUDE_HOME=$HOME/.claude
if [ -n "$KEY_FILE" ]; then
    # A pane's environment is what `$SHELL -l` exports (the daemon asks it,
    # daemon/src/shellenv.rs), so the sandbox's login profile signs claude
    # in. `read` is a builtin: the key is on no command line. CLAUDE_CONFIG_DIR
    # gives claude the sandbox's config and a keychain entry name of its own,
    # never the one a sign-in on this machine made; it is exported here too,
    # for a pane that starts before the daemon has asked the shell.
    # CLAUDE_CODE_TMPDIR keeps its scratch files in the sandbox, not under
    # /tmp/claude-<uid>. The profile goes with the sandbox; seed.py answers
    # claude's first-run screens.
    export CLAUDE_CONFIG_DIR=$HOME/.claude
    mkdir -m 700 "$HOME/tmp"
    (
        umask 077
        cat >"$HOME/.profile" <<EOF
read -r ANTHROPIC_API_KEY <'$KEY_FILE'
export ANTHROPIC_API_KEY
export ANTHROPIC_MODEL='$MODEL'
export CLAUDE_CONFIG_DIR="\$HOME/.claude"
export CLAUDE_CODE_TMPDIR="\$HOME/tmp"
export PATH='$CLAUDE_DIR':"\$PATH"
export DISABLE_AUTOUPDATER=1
EOF
    )
else
    export MESIMON_CLAUDE_BIN=$HERE/stub-claude.py
fi
export MESIMON_NO_UPDATE_CHECK=1
# The worktree flags (`1 to merge`) refresh every 2 s, not every 10 s, so the
# take does not sit on a finished agent waiting for the merge offer.
export MESIMON_WT_REFRESH_TICKS=8
# A link a take opens goes nowhere: the board says `opening …` and no
# browser starts on the machine doing the recording.
export MESIMON_OPEN=true
export MESIMON_THEME=graphite
export MESIMON_COLOR=truecolor
export DEMO_MESIMON=$MESIMON
export DEMO_REPO=$REPO
export DEMO_TERM=$HERE/kitty-term.py

"$MESIMON" daemon --repo "$REPO" >"$SANDBOX/daemon.out" 2>&1 &
DAEMON=$!
DEMO_KEY_FILE=$KEY_FILE python3 -B "$HERE/seed.py" board "$REPO" "$TAPE"

cd "$HERE"
vhs "$TAPE.tape"
ls -l "$(sed -n 's/^Output //p' "$TAPE.tape")"
