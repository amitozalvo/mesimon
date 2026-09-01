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
  "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"

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
