#!/bin/sh
# Runs once in each new ticket worktree, before its agent starts (see docs/USING.md).
# Seeds this worktree's build cache from the checkout's, so cargo rebuilds only the
# crates that changed instead of the whole workspace from nothing (10 to 27 minutes).
# macOS: APFS clones, so the copy is metadata only and takes no space until a file
# diverges. Linux: a reflink where the filesystem allows one, a plain copy otherwise.
set -e
# A checkout on a msmn/ branch is itself a ticket worktree, so this worktree is a
# rig test's (ci/rig.py): its worker never builds, and the clone would only pin
# build output the checkout has since replaced (T-765).
case "$(git -C "$MESIMON_CHECKOUT" symbolic-ref --short -q HEAD)" in
  msmn/*) exit 0 ;;
esac

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
shared="$(dirname "${MESIMON_WORKTREE:-$PWD}")"
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
    # The incremental cache of a worktree whose branch is in the checkout's history
    # (landed, or never committed to) and that has not built for an hour is 4 to 6 GB
    # nobody is using. Its next build costs one full compile of the workspace crates.
    for other in "$shared"/*/; do
      cache="${other}target/debug/incremental"
      [ -d "$cache" ] || continue
      head="$(git -C "$other" rev-parse -q --verify HEAD 2>/dev/null)" || continue
      git -C "$MESIMON_CHECKOUT" merge-base --is-ancestor "$head" HEAD 2>/dev/null || continue
      [ -z "$(find "$cache" -maxdepth 1 -mmin -60 2>/dev/null | head -1)" ] || continue
      rm -rf "$cache"
      echo "reclaimed the idle incremental cache of $(basename "$other")"
    done
    echo "wrote $shared/.cargo/config.toml"
    ;;
esac

src="$MESIMON_CHECKOUT/target"
[ -d "$src" ] || exit 0
[ -e target ] && exit 0
case "$(uname)" in
  Darwin) cp -c -R "$src" target ;;
  *) cp -R --reflink=auto "$src" target ;;
esac
echo "seeded target/ from $src"
