#!/usr/bin/env bash
# The root Cargo.toml claims "CI enforces via `cargo tree -d`" for the D22/05
# one-unicode-width invariant. It never did — and the invariant is already
# violated: unicode-width 0.1.14 arrives via ratatui 0.29 -> unicode-truncate.
#
# So this gates on DRIFT, not on zero duplicates. The allowlist is the set we
# have accepted; anything new fails and has to be looked at. Regenerate with:
#   cargo tree -d --workspace | grep -E '^[a-z0-9_-]+ v' | sort -u > ci/dup-deps.allow
set -euo pipefail
cd "$(dirname "$0")/.."

got=$(cargo tree -d --workspace 2>/dev/null | grep -E '^[a-z0-9_-]+ v' | sort -u)
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
