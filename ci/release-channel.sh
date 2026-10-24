#!/usr/bin/env bash
# The release channel of a version, and the tag it is published under (T-946).
#
#   ci/release-channel.sh 0.1.0-beta.13              -> stable v0.1.0-beta.13
#   ci/release-channel.sh 0.1.0-beta.13.unstable.2   -> unstable unstable-0.1.0-beta.13.unstable.2
#
# A version is unstable when one of its pre-release identifiers is exactly
# `unstable`, and nothing else says so: ci/release.sh reads the channel off
# Cargo.toml's version through this script, so a version and its channel
# cannot disagree. An unstable tag starts with `unstable-`, which the
# updater from beta.12 on and the install.sh served since T-877 skip. The
# Rust copy of
# this rule is `mesimon_core::channel::tag`; a unit test in
# crates/mesimon-tui/src/release.rs runs this script against it.

set -euo pipefail

version="${1:?usage: ci/release-channel.sh <version>}"
version="${version#v}"
core="${version%%+*}"
channel=stable
case "$core" in
  *-*)
    case ".${core#*-}." in
      *.unstable.*) channel=unstable ;;
    esac
    ;;
esac
if [ "$channel" = unstable ]; then
  echo "unstable unstable-$version"
else
  echo "stable v$version"
fi
