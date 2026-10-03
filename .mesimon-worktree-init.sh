#!/bin/sh
# Runs once in each new ticket worktree, before its agent starts (see docs/USING.md).
# Seeds this worktree's build cache from the checkout's, so cargo rebuilds only the
# crates that changed instead of the whole workspace from nothing (10 to 27 minutes).
# macOS: APFS clones, so the copy is metadata only and takes no space until a file
# diverges. Linux: a reflink where the filesystem allows one, a plain copy otherwise.
set -e
src="$MESIMON_CHECKOUT/target"
[ -d "$src" ] || exit 0
[ -e target ] && exit 0
case "$(uname)" in
  Darwin) cp -c -R "$src" target ;;
  *) cp -R --reflink=auto "$src" target ;;
esac
echo "seeded target/ from $src"
