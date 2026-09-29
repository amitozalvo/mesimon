#!/usr/bin/env bash
# Put one file on a GitHub repo's default branch, through the contents API.
# ci/release.sh syncs install.sh, the releases README and the Homebrew formula
# with it, and ci/site.sh the pages of mesimon.dev.
#
# Usage:  ci/publish-file.sh <owner/repo> <source> <dest> <commit message>
#
# The API names a file by its git blob id, so a file that has not changed is
# skipped rather than committed again. The content goes to gh through a file
# (`-F content=@file`), never argv: the site's demo GIF is 0.7 MB in base64.

set -euo pipefail

[ $# -eq 4 ] || { echo "usage: ci/publish-file.sh <owner/repo> <source> <dest> <message>" >&2; exit 1; }
repo="$1" src="$2" dest="$3" message="$4"
[ -f "$src" ] || { echo "error: no file at $src" >&2; exit 1; }

existing=$(gh api "repos/$repo/contents/$dest" --jq .sha 2>/dev/null || true)
if [ -n "$existing" ] && [ "$existing" = "$(git hash-object "$src")" ]; then
  echo "unchanged  $repo/$dest"
  exit 0
fi

b64=$(mktemp)
trap 'rm -f "$b64"' EXIT
base64 < "$src" | tr -d '\n' > "$b64"

set -- -X PUT "repos/$repo/contents/$dest" -f message="$message" -F content="@$b64"
[ -n "$existing" ] && set -- "$@" -f sha="$existing"
gh api "$@" --silent
echo "published  $repo/$dest"
