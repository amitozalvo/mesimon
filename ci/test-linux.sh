#!/usr/bin/env bash
# The whole suite on Linux, from this Mac — Debian 12 in Docker, driven by the
# tmux the Linux release bundles (ci/build-tmux.sh aarch64-unknown-linux-musl,
# built first when it is missing), because that is the tmux a Linux board
# runs since T-1097.
#
# It ran on the DISTRO's tmux before that, and MESIMON_TEST_LINUX_TMUX=distro
# still does: the suite had passed for months on a tmux no Linux user had,
# while apt on Debian 12 gave 3.3a, Ubuntu 22.04 3.2a, and every tmux before
# 3.6 rewrote a tab in `-F` output as `_`, which emptied every snapshot the
# daemon took (see `SEP` in crates/mesimon-backend-tmux/src/lib.rs). That leg
# is what a user who sets MESIMON_TMUX_BIN to their own tmux gets.
#
# The container is native arm64 (this is Apple Silicon; the x86_64 artifact is
# smoke-tested under emulation by ci/release.sh instead — a full suite under
# Rosetta would be slow and would test Rosetta). The repo is mounted read-only
# and the build goes to a named volume, so a run can never write into the
# checkout, and a second run is incremental.
#
# Usage:  ci/test-linux.sh            the full suite
#         ci/test-linux.sh <args...>  passed to `cargo test` inside (e.g. -p mesimon-core)
#         MESIMON_TEST_LINUX_TMUX=distro ci/test-linux.sh   on Debian's apt tmux instead

set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }

command -v docker >/dev/null 2>&1 || die "docker is not on PATH" "install Docker Desktop"
docker info >/dev/null 2>&1 || die "the docker daemon is not running" "open -a Docker"

# The same compiler the release is cut with, so the Linux build is not also a
# different-rustc build. The official image carries every patch version.
rust_version=$(rustc -V | awk '{print $2}')
image="rust:${rust_version}-bookworm"
test_git_common=$(git rev-parse --path-format=absolute --git-common-dir)

# The container is arm64, so the bundled tmux it runs is the aarch64 one; the
# repo mount makes it /work/vendor/tmux-<target>/tmux inside. Built here when
# missing (~1 min native), verified when present.
tmux_bin=""
if [ "${MESIMON_TEST_LINUX_TMUX:-bundled}" != distro ]; then
  ci/build-tmux.sh aarch64-unknown-linux-musl
  tmux_bin=/work/vendor/tmux-aarch64-unknown-linux-musl/tmux
fi

exec docker run --rm --init --cpus 2 --memory 4g --platform linux/arm64 \
  -v "$PWD:/work:ro" \
  -v "$test_git_common:$test_git_common:ro" \
  -v msmn-linux-registry:/usr/local/cargo/registry \
  -v msmn-linux-target:/target \
  -w /work \
  -e CARGO_TARGET_DIR=/target \
  -e MESIMON_TEST_AUDIT_ROOT=/target/test-audits \
  -e CARGO_TERM_COLOR=always \
  -e MESIMON_REQUIRE_TMUX=1 \
  -e MESIMON_TMUX_BIN="$tmux_bin" \
  -e MESIMON_CI=1 \
  -e SHELL=/bin/bash \
  "$image" timeout --signal=TERM --kill-after=10s 900s bash -c '
    set -euo pipefail
    apt-get update -qq >/dev/null
    # gh, signed out, in /usr/bin as on the GitHub Ubuntu runner (T-917): a
    # forge e2e whose stand-in loses to it fails here as it fails there.
    apt-get install -y -qq tmux git procps python3 gh >/dev/null
    # An empty MESIMON_TMUX_BIN would be an override naming nothing: unset
    # it, and the distro leg resolves `tmux` from PATH as before.
    [ -n "${MESIMON_TMUX_BIN:-}" ] || unset MESIMON_TMUX_BIN
    # The worktree e2e commits; a container has no identity.
    git config --global user.email "ci@mesimon.invalid"
    git config --global user.name "mesimon ci"
    git config --global --add safe.directory /work
    . /etc/os-release
    echo "$PRETTY_NAME, $("${MESIMON_TMUX_BIN:-tmux}" -V) at ${MESIMON_TMUX_BIN:-$(command -v tmux)}, $(rustc -V)"
    # The no-argument release check covers both roads; a targeted developer
    # selection stays targeted, with no hidden second full integration pass.
    roads=()
    if [ "$#" -eq 0 ]; then roads=(--both-roads); fi
    python3 -B ci/test-run.py "${roads[@]}" -- cargo test --workspace --locked "$@"
  ' -- "$@"
