#!/usr/bin/env bash
# Cut a release from THIS machine and upload it to GitHub.
#
# GitHub-hosted macOS runners bill at 10x minutes on a private repo, and the
# macOS target we ship is the one this laptop already is; the two Linux
# targets are cross-linked from here as well (ci/build-linux.sh), so a release
# is one machine's work. Every gate the workflow would have applied still
# applies, because a local build is only trustworthy if it refuses to cut
# corners:
#
#   * the working tree must be clean, so the artifact is the tagged commit and
#     nothing else — this is the failure mode a CI runner cannot have
#   * the tag must exist, point at HEAD, and match the workspace version
#   * the tag must already be pushed, so the release cannot describe a commit
#     nobody else can fetch
#   * the full suite and clippy must pass, with tmux REQUIRED (a skipped e2e
#     suite is a gate that certifies nothing)
#   * the suite must pass on Linux too, in Docker, against the tmux a distro
#     ships rather than the one we bundle (ci/test-linux.sh) — Docker is
#     required here, not skipped, for the same reason tmux is
#   * the macOS binary is never stripped, and its signature is verified
#   * every packaged artifact is executed before it is published, the Linux
#     ones inside a Debian container of their own architecture
#
# Usage:  ci/release.sh              build, verify, package, publish
#         ci/release.sh --dry-run    everything except the upload

set -euo pipefail
cd "$(dirname "$0")/.."

DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

TARGET="aarch64-apple-darwin"
# Cross-linked from this Mac (ci/build-linux.sh): static musl, one binary per
# architecture. The names are what install.sh derives from `uname` and what
# release.rs carries, pinned by a unit test that reads this file.
LINUX_TARGETS="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
# Source repo (private) and the public repo the binaries are published to.
# The split is what lets a tester install with a curl and no GitHub account,
# while the code stays private.
SRC_REPO="amitozalvo/mesimon"
DIST_REPO="amitozalvo/mesimon-releases"

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# --- who and what -----------------------------------------------------------

[ "$(uname -sm)" = "Darwin arm64" ] || \
  die "this script builds $TARGET and must run on Apple Silicon macOS (this is $(uname -sm))"

# Check the toolchain before anything slow, so a missing one costs a line
# rather than four steps. Diagnosed, not repaired: this project's own doctor
# rule is that we print the fix and never apply it, and a release script
# silently rewriting your PATH is the same trespass one layer down.
command -v cargo >/dev/null 2>&1 || \
  die "cargo is not on PATH" "source ~/.cargo/env"
command -v git >/dev/null 2>&1 || die "git is not on PATH"

version=$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)
tag="v$version"

step "releasing $tag"

[ -z "$(git status --porcelain)" ] || \
  die "the working tree is dirty — the artifact would not match the tag" \
      "commit or stash first: git status"

git rev-parse "$tag" >/dev/null 2>&1 || \
  die "tag $tag does not exist" "git tag $tag"

[ "$(git rev-parse "$tag^{commit}")" = "$(git rev-parse HEAD)" ] || \
  die "tag $tag does not point at HEAD" \
      "check out the tagged commit, or move the tag"

git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1 || \
  die "tag $tag is not on origin — the release would name a commit nobody can fetch" \
      "git push origin $tag"

# --- the gate ---------------------------------------------------------------

step "clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "duplicate dependency drift"
./ci/check-dup-deps.sh

step "bundled tmux"
# Shipped beside mesimon so a fresh machine needs nothing installed, and so
# every tester runs the same tmux the author does. Cached across releases;
# the script verifies a cached binary rather than trusting it.
./ci/build-tmux.sh
[ -x vendor/tmux/tmux ] || die "vendor/tmux/tmux missing after build"

step "tests (driven by the bundled tmux)"
# Test what actually ships. The suite used to run against whatever tmux the
# author had on PATH, which is precisely the variable bundling exists to
# remove — so the bundled binary is the one that has to pass.
MESIMON_TMUX_BIN="$PWD/vendor/tmux/tmux" MESIMON_REQUIRE_TMUX=1 cargo test --workspace

step "tests on Linux (Docker, the distro's own tmux)"
# The same suite on the platform the Linux artifacts are for, driven by the
# tmux a `sudo apt install tmux` gives people — 3.3a on Debian 12 — which is
# where a 3.6-only assumption (a tab in `-F` output) first failed with every
# pane alive. The script dies without Docker rather than skipping: a Linux
# gate that ran nothing would certify a Linux build nobody had tested.
./ci/test-linux.sh

# --- build ------------------------------------------------------------------

step "build --release --target $TARGET"
# MESIMON_RELEASE is what stamps this build as a RELEASE, and it is set here
# and nowhere else. The stamp is what arms the update checker (it asks the
# dist repo for a newer tag, and can replace the binary at its own path), so
# every other build — a `cargo run`, a plain `cargo build --release`, every
# test binary — comes out `dev` and never checks. That gate cannot be a
# heuristic: a wrong answer would point a download at somebody's build tree.
MESIMON_RELEASE=1 cargo build --release --locked --target "$TARGET"
bin="target/$TARGET/release/mesimon"

# NOT stripped: the linker gives every arm64 binary an ad-hoc signature, strip
# invalidates it, and the symptom on someone else's Mac is SIGKILL with no
# explanation. Verify rather than assume.
step "code signature"
codesign -v --verbose=2 "$bin"

step "smoke test"
"$bin" --version
"$bin" --help >/dev/null

step "build for Linux ($LINUX_TARGETS)"
# Cross-linked here with the toolchain's own rust-lld: static musl, so one
# binary per architecture runs on every distro and under WSL2. The release
# stamp is PASSED THROUGH, not set a second time — the note above still
# holds; this is the same build step reaching two more targets.
MESIMON_RELEASE=1 ./ci/build-linux.sh $LINUX_TARGETS

# --- package ----------------------------------------------------------------

step "package"
rm -rf dist
assets=()
name="mesimon-$tag-$TARGET"
mkdir -p "dist/$name"
cp "$bin" "dist/$name/"
# tmux sits BESIDE mesimon under a name that will not shadow the user's own
# on PATH: that adjacency is the resolution ladder's middle rung (tmux_bin()),
# so both the location and the name are load-bearing.
cp vendor/tmux/tmux "dist/$name/mesimon-tmux"
cp README.md LICENSE NOTICE TRADEMARK.md "dist/$name/"
cp -R vendor/tmux/licenses "dist/$name/licenses-bundled"
tar -czf "dist/$name.tar.gz" -C dist "$name"
( cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256" )
cat "dist/$name.tar.gz.sha256"
ls -lh "dist/$name.tar.gz"
assets+=("dist/$name.tar.gz" "dist/$name.tar.gz.sha256")

# The Linux packages carry no tmux (ci/build-linux.sh says why) and so no
# bundled licenses; everything else is the layout install.sh unpacks. The
# checksum file is `shasum`'s `<hex>  <name>` line, which Linux's `sha256sum
# -c` reads unchanged.
for t in $LINUX_TARGETS; do
  lname="mesimon-$tag-$t"
  mkdir -p "dist/$lname"
  cp "target/$t/release/mesimon" "dist/$lname/"
  cp README.md LICENSE NOTICE TRADEMARK.md "dist/$lname/"
  # bsdtar records every file's `com.apple.provenance` xattr, and GNU tar on
  # the other end prints a warning per file while unpacking it. Measured on
  # Debian 12: five warnings before "installed". Strip the Mac metadata here;
  # nothing in the package needs it.
  COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -czf "dist/$lname.tar.gz" -C dist "$lname"
  ( cd dist && shasum -a 256 "$lname.tar.gz" > "$lname.tar.gz.sha256" )
  cat "dist/$lname.tar.gz.sha256"
  ls -lh "dist/$lname.tar.gz"
  assets+=("dist/$lname.tar.gz" "dist/$lname.tar.gz.sha256")
done

# Prove the tarball is the thing install.sh will unpack, from the archive
# rather than from the build directory.
step "verify the packaged artifact"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xzf "dist/$name.tar.gz" -C "$tmp"
"$tmp/$name/mesimon" --version >/dev/null || die "the packaged binary would not run"
echo "unpacked and ran: $("$tmp/$name/mesimon" --version)"
# Prove the bundled tmux is the one mesimon will pick from that layout, and
# that it runs from a path it was never built in.
"$tmp/$name/mesimon-tmux" -V >/dev/null || die "the packaged tmux would not run"
picked=$("$tmp/$name/mesimon" doctor multiplexer --verbose | grep 'tmux binary' || true)
case "$picked" in
  *"$tmp/$name/mesimon-tmux"*) echo "mesimon picks its bundled tmux: $("$tmp/$name/mesimon-tmux" -V)" ;;
  *) die "the packaged mesimon did not resolve its bundled tmux:
$picked" ;;
esac
# And that the release stamp survived into the artifact. An unstamped build
# is a silent failure of exactly the wrong shape: it installs, runs, and then
# never tells anyone a newer version exists. Checked here rather than on
# `$bin`, because `$bin` is inside target/ and the checker refuses a build
# tree whatever its stamp says — the unpacked artifact is the real answer.
checks=$("$tmp/$name/mesimon" doctor install --verbose | grep 'update checks' || true)
case "$checks" in
  *"off ∙"*) die "the packaged mesimon will not check for updates:
$checks" "MESIMON_RELEASE=1 must be set on the cargo build above" ;;
  *"update checks"*) echo "release stamp present:${checks#*update checks}" ;;
  *) die "the packaged mesimon printed no 'update checks' line at all" \
      "mesimon doctor install must report it — see crates/mesimon-tui/src/release.rs" ;;
esac

# The same two proofs for each Linux artifact — it runs, and it is stamped —
# from inside a Debian container of its own architecture (the x86_64 one
# under Docker's emulation, which is exactly enough for `--version`). The
# checker's build-tree guard is exercised for real here too: /pkg has no
# `target` component, so `off` can only mean a missing stamp or a target the
# checker does not know.
step "verify the Linux artifacts (Docker)"
for t in $LINUX_TARGETS; do
  lname="mesimon-$tag-$t"
  tar -xzf "dist/$lname.tar.gz" -C "$tmp"
  case "$t" in
    x86_64-*)  plat=linux/amd64 ;;
    aarch64-*) plat=linux/arm64 ;;
    *) die "no docker platform for $t" ;;
  esac
  said=$(docker run --rm --platform "$plat" -v "$tmp/$lname:/pkg:ro" -e HOME=/root \
      debian:bookworm-slim \
      sh -c '/pkg/mesimon --version && /pkg/mesimon doctor install --verbose | grep "update checks"') \
    || die "the packaged $t binary would not run in a $plat container, or printed no 'update checks' line"
  echo "$said"
  case "$said" in
    *"off ∙"*) die "the packaged $t mesimon will not check for updates:
$said" "MESIMON_RELEASE=1 must reach ci/build-linux.sh, and release.rs must name $t" ;;
  esac
done

# --- notes ------------------------------------------------------------------

awk -v want="## $tag" '
  $0 == want {found=1; next}
  found && /^## / {exit}
  found {print}
' CHANGELOG.md > dist/notes.md || true
[ -s dist/notes.md ] || echo "See CHANGELOG.md." > dist/notes.md

if [ "$DRY_RUN" = "1" ]; then
  step "dry run — nothing published"
  echo "artifacts in dist/, notes in dist/notes.md"
  echo "publish with: ci/release.sh"
  exit 0
fi

# --- publish ----------------------------------------------------------------

step "publish to $DIST_REPO"
command -v gh >/dev/null || die "the GitHub CLI is required to publish" "brew install gh"
gh auth status >/dev/null 2>&1 || die "gh is not logged in" "gh auth login"
gh repo view "$DIST_REPO" >/dev/null 2>&1 || \
  die "the public releases repo does not exist yet" \
      "gh repo create $DIST_REPO --public --add-readme"

# The dist repo carries no source, so its tag would otherwise name nothing.
# Record the commit this artifact was actually built from.
sha=$(git rev-parse HEAD)
{
  echo
  echo "---"
  echo "Built from \`$SRC_REPO\` at \`${sha:0:7}\`."
} >> dist/notes.md

gh release create "$tag" \
  --repo "$DIST_REPO" \
  --title "$tag" \
  --notes-file dist/notes.md \
  --prerelease \
  "${assets[@]}"

# Keep the public repo's install.sh and README in step with what was just
# released — the curl one-liner reads them straight off its main branch.
publish_file() {
  local src="$1" dest="$2" existing
  existing=$(gh api "repos/$DIST_REPO/contents/$dest" --jq .sha 2>/dev/null || true)
  set -- -X PUT "repos/$DIST_REPO/contents/$dest" \
    -f message="sync $dest ($tag)" \
    -f content="$(base64 < "$src" | tr -d '\n')"
  [ -n "$existing" ] && set -- "$@" -f sha="$existing"
  gh api "$@" --silent
}
step "sync install.sh + README to $DIST_REPO"
publish_file install.sh install.sh
publish_file ci/releases-readme.md README.md

echo
echo "published $tag"
echo
echo "share this line:"
echo "  curl -fsSL https://raw.githubusercontent.com/$DIST_REPO/main/install.sh | sh"
