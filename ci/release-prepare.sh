#!/usr/bin/env bash
# Everything a release needs before its tag, at once (T-1101).
#
# From "release" to the release on mesimon.dev took ~45 minutes at
# beta.14.unstable.2 with nothing wasted: the suite's two roads ran one after
# the other (~16 min each under load), and the author's ship then built three
# release binaries cold (~7 min). This is the one command after the release
# commit. It starts the stamped gate (ci/test-run.py, both roads side by side),
# waits for the test build, and then warms, beside the tests, what ci/release.sh
# and the relay's ship.sh will run again on this same commit: the gate's clippy,
# the dependency drift check, the site build, the macOS release build and the
# two Linux ones (ci/build-linux.sh, with their clippy), with MESIMON_RELEASE
# set exactly as release.sh sets it. Each of those steps then finds its work
# done and takes seconds. The e2es wait on their own timers and barely use the
# CPU (T-1023), so the builds ride beside them, niced, and the test build —
# the critical path — runs alone first.
#
# A red clippy ends the gate at once: the commit will change, and the run would
# prove nothing. The gate's own result is the stamp (target/suite-passed.json),
# which release.sh and the relay's dry run honour.
#
# Usage:  ci/release-prepare.sh [--jobs N]
#   --jobs N          each road's test threads and cargo's build jobs
#                     (ci/test-run.py --jobs; default: nextest's own)
#   MESIMON_TMUX_BIN  the tmux that drives the suite; default the bundled
#                     vendor/tmux/tmux, in a worktree the main checkout's
#
# Run it where ci/release.sh will run — the main checkout — for the warm builds
# to count: cargo keys its artifacts by source path, so a worktree's builds
# never serve the checkout the release is cut from. In a worktree the release
# builds are skipped and the stamp is the deliverable (copied to the main
# checkout's target/ after the fast-forward, T-654).
set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
t0=$(date +%s)
mark() { printf '%6ss  %s\n' "$(( $(date +%s) - t0 ))" "$1"; }

jobs=
while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) jobs=${2:?--jobs needs a number}; shift 2 ;;
    --jobs=*) jobs=${1#--jobs=}; shift ;;
    *) die "unknown argument: $1" "ci/release-prepare.sh [--jobs N]" ;;
  esac
done

# --- what would stop the release later, checked now ---------------------------

[ -z "$(git status --porcelain)" ] || \
  die "the working tree is dirty — the gate would not stamp (the commit is not what ran)" \
      "commit first: git status"

version=$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)
heading=$(grep -m1 '^## ' CHANGELOG.md || true)
printf '%s\n' "$heading" | grep -qE "^## v$version( — | - )[0-9]{4}-[0-9]{2}-[0-9]{2}$" || \
  die "CHANGELOG.md's top heading is '$heading', not v$version's" \
      "add '## v$version — $(date +%F)' above its notes, at the top of the file"

main=$(cd "$(git rev-parse --git-common-dir)/.." && pwd -P)
worktree=0
[ "$main" = "$(pwd -P)" ] || worktree=1
if [ -z "${MESIMON_TMUX_BIN:-}" ]; then
  if [ -x vendor/tmux/tmux ]; then
    MESIMON_TMUX_BIN="$PWD/vendor/tmux/tmux"
  elif [ -x "$main/vendor/tmux/tmux" ]; then
    MESIMON_TMUX_BIN="$main/vendor/tmux/tmux"
  else
    die "no bundled tmux to drive the suite" "ci/build-tmux.sh, or MESIMON_TMUX_BIN=<path>"
  fi
fi
export MESIMON_TMUX_BIN

logs=target/release-prepare
rm -rf "$logs"
mkdir -p "$logs"
step "preparing v$version at $(git rev-parse --short HEAD) (logs in $logs/)"
echo "  suite driven by $MESIMON_TMUX_BIN"
uptime

# --- the gate, then the warm-up beside it ------------------------------------

step "the gate: both roads, side by side (ci/test-run.py --both-roads${jobs:+ --jobs $jobs})"
MESIMON_REQUIRE_TMUX=1 python3 -B ci/test-run.py --both-roads ${jobs:+--jobs "$jobs"} \
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

warm() {
  local name=$1
  shift
  mark "$name: $*"
  if ! nice -n 10 "$@" > "$logs/$name.log" 2>&1; then
    mark "$name FAILED"
    tail -n 40 "$logs/$name.log"
    kill -TERM "$gate" 2>/dev/null || true
    wait "$gate" || true
    die "$name failed, so the gate was stopped: this commit will not be the release" "$logs/$name.log"
  fi
  mark "$name ok"
}
warm clippy cargo clippy --workspace --all-targets -- -D warnings
warm dup-deps ./ci/check-dup-deps.sh
warm site ci/site.sh
if [ "$worktree" = 0 ]; then
  warm build-macos env MESIMON_RELEASE=1 cargo build --release --locked --target aarch64-apple-darwin
  warm build-linux env MESIMON_RELEASE=1 ./ci/build-linux.sh
else
  mark "release builds skipped: a worktree's artifacts never serve $main, where ci/release.sh builds"
fi

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
  grep -E '^(mod \| )? *(FAIL|TIMEOUT|LEAK|SIGSEGV|SIGABRT) ' "$logs/gate.log" | head -n 20 || true
  die "the gate failed" "$logs/gate.log"
fi

step "ready: v$version at $(git rev-parse --short HEAD) is stamped and warm"
echo "next, in the relay checkout:"
echo "  cargo update --workspace --offline && git commit -m 'chore: lock the core crates at v$version' Cargo.lock"
echo "  deploy/ship.sh all --dry-run"
echo "then hand the author the one command:"
echo "  deploy/ship.sh all"
