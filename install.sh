#!/usr/bin/env sh
# mesimon installer.
#
# Re-running this IS the update: it replaces the binary at the same path, and a
# running board notices the new mtime and offers `update ready (U reloads)`.
# If no board is open, the next one restarts the stale daemon by itself.
#
#   sh install.sh                  install or update
#   sh install.sh --version v0.1.0-alpha.1
#   PREFIX=~/bin sh install.sh     install somewhere else
#
# The repo is private, so the download goes through `gh` (which carries your
# GitHub login) rather than a bare curl.

set -eu

REPO="amitozalvo/mesimon"
PREFIX="${PREFIX:-$HOME/.local/bin}"
VERSION=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --prefix)  PREFIX="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }

# --- preconditions, each with the exact line that fixes it -------------------

[ "$(uname -s)" = "Darwin" ] || die "mesimon builds are macOS-only right now (this is $(uname -s))" \
  "build from source instead: cargo install --git https://github.com/$REPO --locked mesimon"

[ "$(uname -m)" = "arm64" ] || die "the published build is Apple Silicon only (this is $(uname -m))" \
  "build from source instead: cargo install --git https://github.com/$REPO --locked mesimon"

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is required to download from a private repo" \
  "brew install gh && gh auth login"

gh auth status >/dev/null 2>&1 || die "gh is installed but not logged in" "gh auth login"

command -v tmux >/dev/null 2>&1 || die "tmux is required — mesimon runs every agent in its own private tmux server" \
  "brew install tmux"

command -v git >/dev/null 2>&1 || die "git is required" "xcode-select --install"

# Not fatal: you can browse a board without ever spawning an agent.
command -v claude >/dev/null 2>&1 || {
  echo "note: 'claude' is not on your PATH. mesimon will run, but spawning a"
  echo "      Claude session will fail until Claude Code is installed."
}

# --- download ---------------------------------------------------------------

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -z "$VERSION" ]; then
  VERSION="$(gh release list --repo "$REPO" --limit 1 --json tagName --jq '.[0].tagName' 2>/dev/null || true)"
  [ -n "$VERSION" ] || die "no releases found on $REPO" \
    "check that you have access: gh repo view $REPO"
fi

asset="mesimon-$VERSION-aarch64-apple-darwin.tar.gz"
echo "downloading $asset"
gh release download "$VERSION" --repo "$REPO" --pattern "$asset" --pattern "$asset.sha256" --dir "$tmp" \
  || die "could not download $asset from $VERSION" \
     "list what exists: gh release view $VERSION --repo $REPO"

# --- verify -----------------------------------------------------------------

if [ -f "$tmp/$asset.sha256" ]; then
  ( cd "$tmp" && shasum -a 256 -c "$asset.sha256" >/dev/null ) \
    || die "checksum mismatch on $asset — refusing to install" \
       "re-run; if it persists, the release asset is corrupt"
  echo "checksum ok"
else
  echo "note: no .sha256 published for this release; skipping verification"
fi

tar -xzf "$tmp/$asset" -C "$tmp"
bin="$tmp/mesimon-$VERSION-aarch64-apple-darwin/mesimon"
[ -x "$bin" ] || die "the archive did not contain an executable" "report this with the release tag"

# Run it BEFORE installing: an arm64 binary with a broken signature dies with
# "Killed: 9", and finding that out now beats finding out from a wedged board.
"$bin" --version >/dev/null || die "the downloaded binary would not run" \
  "report this with the output of: $bin --version"

# --- install ----------------------------------------------------------------

mkdir -p "$PREFIX"
# Same path every time, replaced atomically. Load-bearing: hook settings embed
# this absolute path per session, and the update offer watches its mtime.
mv "$bin" "$PREFIX/mesimon.new"
chmod +x "$PREFIX/mesimon.new"
mv "$PREFIX/mesimon.new" "$PREFIX/mesimon"

echo
echo "installed $("$PREFIX/mesimon" --version) -> $PREFIX/mesimon"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *)
    echo
    echo "$PREFIX is not on your PATH. Add it:"
    echo "  echo 'export PATH=\"$PREFIX:\$PATH\"' >> ~/.zshrc && exec zsh"
    ;;
esac

echo
echo "next:  mesimon doctor      check your environment"
echo "       cd <a git repo> && mesimon"
