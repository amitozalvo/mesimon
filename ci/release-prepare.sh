#!/usr/bin/env bash
# Before the bump: ci/release-prepare.sh --lint
# On the committed candidate: ci/release-prepare.sh [--jobs N]
# After fast-forwarding main to a worktree candidate: ci/release-prepare.sh --handoff
#
# Both roads get six test threads by default (T-1101). Once their test build
# finishes, the release builds run beside the tests. Worktrees hand off only
# the three finished binaries and the original two-road stamp.
set -euo pipefail
cd "$(dirname "$0")/.."

# The steps release.sh will run again, spelled once, so the warm build is the
# release's: cargo keys its artifacts by the exact flags.
. ci/release-lib.sh
t0=$(date +%s)
mark() { printf '%6ss  %s\n' "$(( $(date +%s) - t0 ))" "$1"; }

if [ "$#" = 1 ] && [ "$1" = --lint ]; then
  release_clippy
  exit
fi

main=$(cd "$(git rev-parse --git-common-dir)/.." && pwd -P)
candidate=$(git rev-parse HEAD)
bundle="$main/target/release-prepared/$candidate.zip"
if [ "$#" = 1 ] && [ "$1" = --handoff ]; then
  cd "$main"
  [ "$(git symbolic-ref --short -q HEAD)" = main ] && [ "$(git rev-parse HEAD)" = "$candidate" ] ||
    die "fast-forward main to the prepared candidate before activating its handoff"
  require_clean_tree "the release needs the tested commit" "git status"
  python3 -B ci/release-handoff.py activate "$bundle" "$candidate" "$main/vendor/tmux/tmux"
  exit
fi

jobs=6
while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) jobs=${2:?--jobs needs a number}; shift 2 ;;
    --jobs=*) jobs=${1#--jobs=}; shift ;;
    *) die "unknown argument: $1" "ci/release-prepare.sh [--lint | --jobs N | --handoff]" ;;
  esac
done

# --- what would stop the release later, checked now ---------------------------

require_clean_tree "the gate would not stamp (the commit is not what ran)" "commit first: git status"

step "clippy on the candidate"
release_clippy

version=$(release_version)
heading=$(grep -m1 '^## ' CHANGELOG.md || true)
printf '%s\n' "$heading" | grep -qE "^## v$version( — | - )[0-9]{4}-[0-9]{2}-[0-9]{2}$" || \
  die "CHANGELOG.md's top heading is '$heading', not v$version's" \
      "add '## v$version — $(date +%F)' above its notes, at the top of the file"

worktree=0
[ "$main" = "$(pwd -P)" ] || worktree=1
if [ -z "${MESIMON_TMUX_BIN:-}" ]; then
  if [ -x "$main/vendor/tmux/tmux" ]; then
    MESIMON_TMUX_BIN="$main/vendor/tmux/tmux"
  else
    die "no bundled tmux to drive the suite" "ci/build-tmux.sh, or MESIMON_TMUX_BIN=<path>"
  fi
fi
export MESIMON_TMUX_BIN
# The gate stamp and the release build paths below belong to this checkout.
export CARGO_TARGET_DIR="$PWD/target"

step "compile relay tests against the candidate"
release_relay_compile "${MESIMON_RELAY_DIR:-$(dirname "$main")/mesimon-relay}"

logs=target/release-prepare
rm -rf "$logs"
mkdir -p "$logs"
step "preparing v$version at $(git rev-parse --short HEAD) (logs in $logs/)"
echo "  suite driven by $MESIMON_TMUX_BIN"
uptime

# --- the gate, then the warm-up beside it ------------------------------------

step "the gate: both roads, side by side (ci/test-run.py --both-roads --exclusive --jobs $jobs)"
MESIMON_REQUIRE_TMUX=1 python3 -B ci/test-run.py --both-roads --exclusive --jobs "$jobs" \
  > "$logs/gate.log" 2>&1 &
gate=$!
trap 'kill -TERM $gate 2>/dev/null || true' EXIT
mark "gate started (pid $gate)"
# The test build is the critical path and runs alone: nothing else starts
# until nextest says the tests are starting.
until grep -qE '^ *Starting [0-9]+ tests' "$logs/gate.log"; do
  if ! kill -0 "$gate" 2>/dev/null; then
    cat "$logs/gate.log"
    die "the gate ended before its tests started" "$logs/gate.log"
  fi
  sleep 2
done
mark "the test build is done; the tests run"

# $1: the log's name; the rest: the step, a function of ci/release-lib.sh and
# its arguments, run niced in a subshell (nice cannot run a shell function).
warm() {
  local name=$1
  shift
  mark "$name: $*"
  # macOS's /bin/bash is 3.2, without BASHPID: the exec'd sh's parent is
  # the subshell.
  if ! ( renice -n 10 -p "${BASHPID:-$(exec sh -c 'echo $PPID')}" >/dev/null 2>&1
    "$@" ) > "$logs/$name.log" 2>&1; then
    mark "$name FAILED"
    tail -n 40 "$logs/$name.log"
    kill -TERM "$gate" 2>/dev/null || true
    wait "$gate" || true
    die "$name failed, so the gate was stopped: this commit will not be the release" "$logs/$name.log"
  fi
  mark "$name ok"
}
warm dup-deps release_dup_deps
warm site release_site
warm build-macos release_build_macos aarch64-apple-darwin
warm build-linux release_build_linux

# --- the gate's verdict ------------------------------------------------------

step "waiting for the gate"
said=
while kill -0 "$gate" 2>/dev/null; do
  last=$(grep -E '^(mod \| )? *(PASS|FAIL|Starting|Summary)' "$logs/gate.log" | tail -n 1 || true)
  [ "$last" = "$said" ] || { mark "${last:-...}"; said=$last; }
  sleep 10
done
trap - EXIT
set +e
wait "$gate"
code=$?
set -e
mark "the gate ended ($code)"
grep -E '^(mod \| )?( *Summary|Suite passed|Fixture audit|Check not started|Starting)' "$logs/gate.log" || true
if [ "$code" != 0 ]; then
  echo
  grep -E '^Rerun ' "$logs/gate.log" || true
  grep -E '^(mod \| )? *(FAIL|TIMEOUT|LEAK|SIGSEGV|SIGABRT) ' "$logs/gate.log" | head -n 20 || true
  die "the gate failed" "$logs/gate.log"
fi

require_clean_tree "the candidate changed during prep" "prepare the committed candidate again"
[ "$(git rev-parse HEAD)" = "$candidate" ] || die "HEAD changed during prep"

if [ "$worktree" = 1 ]; then
  python3 -B ci/release-handoff.py save "$bundle" "$candidate" "$main/vendor/tmux/tmux"
  echo "After the fast-forward, run ci/release-prepare.sh --handoff in $main."
fi
step "ready: v$version at $(git rev-parse --short HEAD)"
echo "Follow the release skill for the relay lock update, dry run and ship."
