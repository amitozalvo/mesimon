#!/bin/sh
# Re-record one of the README's clips with VHS: the demo (assets/demo.gif)
# by default, or the tape named, e.g. `search` for assets/demo/search.gif.
# The `prs` tape adds a stand-in `gh` (forge.py) on the sandbox's PATH, so
# the board reads a forge that is one JSON file and calls no network.
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
# API key, or a `claude setup-token` token, kept in a file (an API key bills
# that key's credits, cents a take; a token, its subscription's usage):
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
# The intro (T-869) is a first run's, not a demo's: this HOME has seen it,
# in the words the marker has said since T-903 (an empty one plays it).
mkdir -p "$HOME/.local/state/mesimon" \
  && echo "skipped board demo" >"$HOME/.local/state/mesimon/welcomed"
# The merge train (on by default) would land a finished branch before the
# take's `m` does; the takes show the person merging. The Reddit clips
# (reddit.tape, prs.tape) keep the train, since the first one's ending is
# the train's own merge and the second's board merges on its forge, and
# show every card's latest reply (the `P` peek), so a feed viewer can read
# what each agent is doing.
case $TAPE in
    reddit | prs) echo '{"schema_version":1,"peek":"all"}' >"$HOME/.local/state/mesimon/prefs.json" ;;
    *) echo '{"schema_version":1,"merge_train":false}' >"$HOME/.local/state/mesimon/prefs.json" ;;
esac
# The reddit clip's workers raise their hands one at a time (stub-claude.py,
# `paced`): a feed viewer follows one beat at a time.
[ "$TAPE" = reddit ] && export DEMO_PACED=1 || true
if [ "$TAPE" = prs ]; then
    # A github.com remote names the forge; forge.py answers for it as `gh`,
    # first on PATH for the daemon and, through the sandbox's login shell,
    # for every agent pane. Nothing is fetched (the fetch setting is off).
    git -C "$REPO" remote add origin https://github.com/demo/shortlink.git
    mkdir -p "$SANDBOX/bin" "$SANDBOX/forge"
    ln -s "$HERE/forge.py" "$SANDBOX/bin/gh"
    export PATH=$SANDBOX/bin:$PATH
    export MSMN_FORGE=$SANDBOX/forge
    # The forge is read every 2 s, not every minute, and 1.5 s while a
    # pipeline runs: a take cannot wait a minute between beats.
    export MESIMON_FORGE_EVERY_MS=2000
    export MESIMON_FORGE_BUSY_MS=1500
    export MESIMON_FORGE_BURST_MS=1000
fi
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
        cat >>"$HOME/.profile" <<EOF
read -r DEMO_KEY <'$KEY_FILE'
case \$DEMO_KEY in
    sk-ant-oat*) export CLAUDE_CODE_OAUTH_TOKEN="\$DEMO_KEY" ;;
    *) export ANTHROPIC_API_KEY="\$DEMO_KEY" ;;
esac
unset DEMO_KEY
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
# Made by kitty-term.py as the take opens: the stand-in agents of the
# pull-request clip, and forge.py's clocks, wait for it.
export DEMO_GO=$SANDBOX/go
# Every wake the board types into a crown's pane, for tuning its answers.
export DEMO_WAKES=$SANDBOX/wakes.log
# A pane's environment is what the sandbox's login shell exports
# (daemon/src/shellenv.rs), not this script's: what the stand-ins and the
# stand-in forge read goes through its profile. The profile is written
# before the key-file case appends claude's sign-in below.
{
    echo "export DEMO_GO='$DEMO_GO' DEMO_WAKES='$DEMO_WAKES'"
    [ -n "${DEMO_PACED:-}" ] && echo "export DEMO_PACED=1"
    [ -n "${MSMN_FORGE:-}" ] && echo "export PATH='$SANDBOX/bin':\"\$PATH\" MSMN_FORGE='$MSMN_FORGE'"
    true
} >"$HOME/.profile"

"$MESIMON" daemon --repo "$REPO" >"$SANDBOX/daemon.out" 2>&1 &
DAEMON=$!
DEMO_KEY_FILE=$KEY_FILE python3 -B "$HERE/seed.py" board "$REPO" "$TAPE"

cd "$HERE"
if ! vhs "$TAPE.tape"; then
    # The stand-in leaves its traceback in its pane; the sandbox goes with
    # the take, so print every agent pane before it does.
    if [ -S "$RT/tmux.sock" ]; then
        for pane in $(tmux -S "$RT/tmux.sock" list-panes -a -F '#{pane_id}' 2>/dev/null); do
            echo "--- agent pane $pane ---" >&2
            tmux -S "$RT/tmux.sock" capture-pane -p -t "$pane" >&2 || true
        done
    fi
    [ -f "$DEMO_WAKES" ] && { echo "--- wakes the crown heard ---" >&2; cat "$DEMO_WAKES" >&2; }
    if [ "$TAPE" = prs ]; then
        echo "--- the board's pull requests ---" >&2
        python3 -B "$HERE/seed.py" prs "$REPO" >&2 || true
        echo "--- the forge's state ---" >&2; cat "$MSMN_FORGE/state.json" >&2 || true
        echo "--- git log origin/main ---" >&2; git -C "$REPO" log --oneline refs/remotes/origin/main >&2 || true
        echo "--- daemon.out (forge) ---" >&2; grep -i "forge\|deploy" "$SANDBOX/daemon.out" | tail -40 >&2 || true
    fi
    exit 1
fi
# A tape may name more than one Output (reddit.tape: an mp4 and a gif).
sed -n 's/^Output //p' "$TAPE.tape" | while read -r out; do ls -l "$out"; done
[ -f "$DEMO_WAKES" ] && { echo "--- wakes the crown heard ---"; cat "$DEMO_WAKES"; }
true
