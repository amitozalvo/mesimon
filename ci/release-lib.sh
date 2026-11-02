# Sourced by ci/release.sh and ci/release-prepare.sh, after `cd` to the
# repository root: what both run on the release commit, spelled once. Cargo
# keys its artifacts by the exact flag set and environment, so a warm-up that
# drifted from the release by one flag would build everything twice and the
# "takes seconds" promise of ci/release-prepare.sh would be false. The release
# script is the caller of record; the warm-up only runs these earlier.

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

release_version() { grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2; }

# $1: why a dirty tree stops this script; $2: the fix line.
require_clean_tree() { [ -z "$(git status --porcelain)" ] || die "the working tree is dirty — $1" "$2"; }

release_clippy() { cargo clippy --workspace --all-targets -- -D warnings; }
release_dup_deps() { ./ci/check-dup-deps.sh; }
release_site() { ci/site.sh; }
# The target names stay in ci/release.sh (release.rs's unit test pins them
# there) and in ci/build-linux.sh's default. MESIMON_RELEASE is what stamps a
# build as a RELEASE (the note in ci/release.sh); it is set here and nowhere
# else, and passed through to the Linux builds, not set a second time.
release_build_macos() { MESIMON_RELEASE=1 cargo build --release --locked --target "$1"; }
release_build_linux() { MESIMON_RELEASE=1 ./ci/build-linux.sh "$@"; }
