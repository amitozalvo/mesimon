#!/usr/bin/env bash
# Cut a release from THIS machine and upload it to GitHub.
#
# GitHub-hosted macOS runners bill at 10x minutes on a private repo, and the
# only target we ship is the one this laptop already is. So the build happens
# here — but every gate the workflow would have applied still applies, because
# a local build is only trustworthy if it refuses to cut corners:
#
#   * the working tree must be clean, so the artifact is the tagged commit and
#     nothing else — this is the failure mode a CI runner cannot have
#   * the tag must exist, point at HEAD, and match the workspace version
#   * the tag must already be pushed, so the release cannot describe a commit
#     nobody else can fetch
#   * the full suite and clippy must pass, with tmux REQUIRED (a skipped e2e
#     suite is a gate that certifies nothing)
#   * the binary is never stripped, and its signature is verified
#   * the packaged artifact is executed before it is published
#
# Usage:  ci/release.sh              build, verify, package, publish
#         ci/release.sh --dry-run    everything except the upload

set -euo pipefail
cd "$(dirname "$0")/.."

DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

TARGET="aarch64-apple-darwin"
REPO="amitozalvo/mesimon"

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# --- who and what -----------------------------------------------------------

[ "$(uname -sm)" = "Darwin arm64" ] || \
  die "this script builds $TARGET and must run on Apple Silicon macOS (this is $(uname -sm))"

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

step "tests (tmux required)"
command -v tmux >/dev/null || die "tmux is required to run the e2e suite" "brew install tmux"
MESIMON_REQUIRE_TMUX=1 cargo test --workspace

# --- build ------------------------------------------------------------------

step "build --release --target $TARGET"
cargo build --release --locked --target "$TARGET"
bin="target/$TARGET/release/mesimon"

# NOT stripped: the linker gives every arm64 binary an ad-hoc signature, strip
# invalidates it, and the symptom on someone else's Mac is SIGKILL with no
# explanation. Verify rather than assume.
step "code signature"
codesign -v --verbose=2 "$bin"

step "smoke test"
"$bin" --version
"$bin" --help >/dev/null

# --- package ----------------------------------------------------------------

step "package"
name="mesimon-$tag-$TARGET"
rm -rf dist
mkdir -p "dist/$name"
cp "$bin" "dist/$name/"
cp README.md LICENSE NOTICE TRADEMARK.md "dist/$name/"
tar -czf "dist/$name.tar.gz" -C dist "$name"
( cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256" )
cat "dist/$name.tar.gz.sha256"
ls -lh "dist/$name.tar.gz"

# Prove the tarball is the thing install.sh will unpack, from the archive
# rather than from the build directory.
step "verify the packaged artifact"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xzf "dist/$name.tar.gz" -C "$tmp"
"$tmp/$name/mesimon" --version >/dev/null || die "the packaged binary would not run"
echo "unpacked and ran: $("$tmp/$name/mesimon" --version)"

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

step "publish to $REPO"
command -v gh >/dev/null || die "the GitHub CLI is required to publish" "brew install gh"
gh auth status >/dev/null 2>&1 || die "gh is not logged in" "gh auth login"

gh release create "$tag" \
  --repo "$REPO" \
  --title "$tag" \
  --notes-file dist/notes.md \
  --prerelease \
  "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"

echo
echo "published $tag"
echo "install anywhere with:  sh install.sh"
