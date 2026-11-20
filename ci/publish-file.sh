#!/usr/bin/env bash
# Put one file on a GitHub repo's default branch, through the contents API.
# ci/release.sh syncs install.sh, the releases README and the Homebrew formula
# with it, and ci/site.sh the pages of mesimon.dev.
#
# Usage:  ci/publish-file.sh <owner/repo> <source> <dest> <commit message> [existing blob id]
# Pass an empty fifth argument for a path known to be new.
#
# The API names a file by its git blob id, so a file that has not changed is
# skipped rather than committed again. The content goes to gh through a file
# (`-F content=@file`), never argv: the site's demo GIF is 0.7 MB in base64.

set -euo pipefail

[ $# -eq 4 ] || [ $# -eq 5 ] || { echo "usage: ci/publish-file.sh <owner/repo> <source> <dest> <message> [existing blob id]" >&2; exit 1; }
repo="$1" src="$2" dest="$3" message="$4"
[ -f "$src" ] || { echo "error: no file at $src" >&2; exit 1; }

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
if [ $# -eq 5 ]; then
  existing=$5
else
  # Standalone release files still use a path lookup. gh puts an error body
  # on stdout, so discard it on a 404 and preserve stderr for other failures.
  if existing=$(gh api "repos/$repo/contents/$dest" --jq .sha 2>"$tmp"); then
    :
  elif grep -q ' (HTTP 404)$' "$tmp"; then
    existing=""
  else
    cat "$tmp" >&2
    exit 1
  fi
fi
local_sha=$(git hash-object "$src")
if [ "$existing" = "$local_sha" ]; then
  echo "unchanged  $repo/$dest"
  exit 0
fi

base64 < "$src" | tr -d '\n' > "$tmp"

set -- -X PUT "repos/$repo/contents/$dest" -f message="$message" -F content="@$tmp"
[ -n "$existing" ] && set -- "$@" -f sha="$existing"
gh api "$@" --silent
echo "published  $repo/$dest"
