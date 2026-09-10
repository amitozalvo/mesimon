#!/usr/bin/env bash
# The whole suite on Linux, from this Mac — Debian 12 in Docker, driven by the
# DISTRO's tmux, not the 3.6a we bundle on macOS.
#
# That last part is the reason this exists. The suite passed for months on a
# tmux no Linux user has: apt on Debian 12 gives 3.3a, Ubuntu 22.04 gives
# 3.2a, and every tmux before 3.6 rewrites a tab in `-F` output as `_`, which
# emptied every snapshot the daemon took (see `SEP` in
# crates/mesimon-backend-tmux/src/lib.rs). A Linux gate that ran on the
# bundled tmux would have certified that bug; this one runs on the tmux a
# `sudo apt install tmux` actually installs.
#
# The container is native arm64 (this is Apple Silicon; the x86_64 artifact is
# smoke-tested under emulation by ci/release.sh instead — a full suite under
# Rosetta would be slow and would test Rosetta). The repo is mounted read-only
# and the build goes to a named volume, so a run can never write into the
# checkout, and a second run is incremental.
#
# Usage:  ci/test-linux.sh            the full suite
#         ci/test-linux.sh <args...>  passed to `cargo test` inside (e.g. -p mesimon-core)

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
  -e MESIMON_CI=1 \
  -e SHELL=/bin/bash \
  "$image" timeout --signal=TERM --kill-after=10s 900s bash -c '
    set -euo pipefail
    apt-get update -qq >/dev/null
    apt-get install -y -qq tmux git procps python3 >/dev/null
    # The worktree e2e commits; a container has no identity.
    git config --global user.email "ci@mesimon.invalid"
    git config --global user.name "mesimon ci"
    git config --global --add safe.directory /work
    . /etc/os-release
    echo "$PRETTY_NAME, $(tmux -V), $(rustc -V)"
    python3 -B ci/test-run.py -- cargo test --workspace --locked "$@"
  ' -- "$@"
