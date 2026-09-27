#!/usr/bin/env bash
# The root Cargo.toml claims "CI enforces via `cargo tree -d`" for the D22/05
# one-unicode-width invariant; this script is that enforcement. The invariant
# holds since ratatui 0.30 (T-470); until then unicode-width 0.1.14 arrived via
# ratatui 0.29 -> unicode-truncate.
#
# Other duplicates remain, so this gates on DRIFT, not on zero duplicates. The
# allowlist is the set we have accepted; anything new fails and has to be
# looked at. Regenerate with:
#   ci/check-dup-deps.sh --regen
# `--target all` so the set is the same on every host (a macOS-generated list
# must hold on the Linux runner), `--color never` because CI exports
# CARGO_TERM_COLOR=always and a coloured `(*)` never equals a plain one.
set -euo pipefail
cd "$(dirname "$0")/.."

tree() { cargo tree -d --workspace --target all --color never 2>/dev/null | grep -E '^[a-z0-9_-]+ v' | sort -u; }

if [ "${1:-}" = "--regen" ]; then
  tree > ci/dup-deps.allow
  echo "ci/dup-deps.allow regenerated ($(wc -l < ci/dup-deps.allow | tr -d ' ') entries)"
  exit 0
fi

got=$(tree)
want=$(cat ci/dup-deps.allow)

if [ "$got" != "$want" ]; then
  echo "duplicate dependencies drifted from ci/dup-deps.allow:"
  diff <(echo "$want") <(echo "$got") || true
  echo
  echo "If the new duplicate is intended, regenerate the allowlist (see the"
  echo "header of this script). If it duplicates unicode-width across OUR"
  echo "crates, it breaks D22/05 and the width math -- fix it instead."
  exit 1
fi
echo "duplicate dependencies match the allowlist ($(echo "$want" | wc -l | tr -d ' ') entries)"
