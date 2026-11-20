#!/bin/sh
# Runs once in each new ticket worktree, before its agent starts (see docs/USING.md).
# Seeds Mesophon's dependencies and, for build tickets, the checkout's Cargo cache.
# macOS: APFS clones, so the copy is metadata only and takes no space until a file
# diverges. Linux: a reflink where the filesystem allows one, a plain copy otherwise.
set -e
# After a read-only ticket gets the go, run this script again from its worktree
# before building. Only the daemon's init environment supplies the checkout path.
if [ -z "${MESIMON_CHECKOUT:-}" ]; then
  MESIMON_CHECKOUT=$(git worktree list --porcelain | sed -n '1s/^worktree //p')
  [ -n "$MESIMON_CHECKOUT" ] || exit 1
fi
# A checkout on a msmn/ branch is itself a ticket worktree, so this worktree is a
# rig test's (ci/rig.py): its worker never builds, and the clone would only pin
# build output the checkout has since replaced (T-765).
case "$(git -C "$MESIMON_CHECKOUT" symbolic-ref --short -q HEAD)" in
  msmn/*) exit 0 ;;
esac

seed() (
  src="$MESIMON_CHECKOUT/$1"
  [ -d "$src" ] && [ ! -e "$1" ] && [ ! -L "$1" ] || return 0
  # Publish only a complete copy, so a failed init can be retried.
  parent=$(dirname "$1")
  stage=$(mktemp -d "$parent/.mesimon-seed.XXXXXXXX")
  trap 'rm -rf "$stage"' EXIT
  dest="$stage/${1##*/}"
  case "$(uname)" in
    Darwin) cp -c -R "$src/." "$dest" ;;
    *) cp -R --reflink=auto "$src/." "$dest" ;;
  esac
  mv "$dest" "$parent/"
  echo "seeded $1 from $src"
)

# The init environment carries the key. Only an explicit group-2 tag in this
# checkout's ticket file defers the Rust cache. An unavailable TOML reader or
# ticket record takes the ordinary warm-build path.
read_only=false
ticket="$MESIMON_CHECKOUT/.mesimon/board/tickets/${MESIMON_TICKET:-}/ticket.toml"
# A child leg of a multi-repository board can have its own unrelated T-1.
# Only the root worktree's directory matches the board's branch suffix.
worktree=${MESIMON_WORKTREE:-$PWD}
if [ "${worktree##*/}" = "${MESIMON_WORKTREE_BRANCH#msmn/}" ] && [ -f "$ticket" ] && python3 - "$ticket" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as f:
    tags = tomllib.load(f).get("tags", [])
read_only = any(tag.get("group") == 2 and tag.get("name") in ("RESEARCH", "READY") for tag in tags)
sys.exit(0 if read_only else 1)
PY
then
  read_only=true
fi

# How every ticket worktree builds, in the one directory they share: cargo reads each
# `.cargo/config.toml` up the tree, so this reaches the worktrees already cut too.
# Measured on 2026-11-10 (T-1193), four worktrees rebuilding the unit tests at once
# after a one-line core edit: 98 s each and load 57 as cargo ships; 18 to 21 s each
# and load 14 with these three.
#   incremental: an edit recompiles what it touched (core 31 s -> 8 s alone). The cache
#     is 4 to 6 GB a worktree, which is why Cargo.toml keeps it off for the checkout.
#   jobs: with incremental on, four jobs build as fast as eleven.
#   rustc-wrapper: six compilers at once across all of them (ci/rustc-slot.py), copied
#     beside the config so no worktree's build depends on what the checkout has out.
# Only under mesimon's own state directory: a worktree kept anywhere else shares its
# parent with projects this has no business configuring.
shared="$(dirname "$worktree")"
case "$shared" in
  */mesimon/*/worktrees)
    mkdir -p "$shared/.cargo"
    wrapper=""
    if [ -f "$MESIMON_CHECKOUT/ci/rustc-slot.py" ] && command -v python3 >/dev/null 2>&1; then
      cp "$MESIMON_CHECKOUT/ci/rustc-slot.py" "$shared/.cargo/rustc-slot.py.$$"
      chmod +x "$shared/.cargo/rustc-slot.py.$$"
      mv -f "$shared/.cargo/rustc-slot.py.$$" "$shared/.cargo/rustc-slot.py"
      wrapper="rustc-wrapper = '$shared/.cargo/rustc-slot.py'"
    fi
    cat > "$shared/.cargo/config.toml.$$" <<EOF
# Written by .mesimon-worktree-init.sh each time a ticket worktree is cut; an edit
# here lasts until the next one. The script says why each line is here.
[build]
incremental = true
jobs = 4
$wrapper
EOF
    mv -f "$shared/.cargo/config.toml.$$" "$shared/.cargo/config.toml"
    echo "wrote $shared/.cargo/config.toml"
    # Only the first build ticket each UTC hour scans siblings. Concurrent inits
    # compete for one directory, and a killed sweep can be retried next hour.
    if [ "$read_only" = false ] && mkdir "$shared/.cargo/reclaim-$(date -u +%Y%m%d%H)" 2>/dev/null; then
      find "$shared/.cargo" -maxdepth 1 -type d -name 'reclaim-*' -mtime +1 -empty -delete
      # Reclaim only landed worktrees whose incremental cache has been idle an hour.
      for other in "$shared"/*/; do
        cache="${other}target/debug/incremental"
        [ -d "$cache" ] || continue
        head="$(git -C "$other" rev-parse -q --verify HEAD 2>/dev/null)" || continue
        git -C "$MESIMON_CHECKOUT" merge-base --is-ancestor "$head" HEAD 2>/dev/null || continue
        [ -z "$(find "$cache" -maxdepth 1 -mmin -60 2>/dev/null | head -1)" ] || continue
        rm -rf "$cache"
        echo "reclaimed the idle incremental cache of $(basename "$other")"
      done
    fi
    ;;
esac

if [ "$read_only" = true ]; then
  echo "deferred target/ for a read-only ticket; after the go, run ./.mesimon-worktree-init.sh before building"
else
  seed target
fi

if [ -d web/mesophon ]; then
  seed web/mesophon/node_modules
fi
