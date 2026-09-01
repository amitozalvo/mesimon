#!/usr/bin/env sh
# mesimon installer.
#
#   curl -fsSL https://raw.githubusercontent.com/amitozalvo/mesimon-releases/main/install.sh | sh
#
# Re-running this IS the update: it replaces the binary at the same path, and a
# running board notices the new mtime and offers `update ready (U reloads)`.
# If no board is open, the next one restarts the stale daemon by itself.
#
#   sh install.sh                          install or update
#   sh install.sh --version v0.1.0-alpha.1 pin a version
#   PREFIX=~/bin sh install.sh             install somewhere else
#
# Binaries are published from a separate public repo, so there is no GitHub
# account, login, or invite involved. The source lives in a private repo and
# is not needed to run mesimon.

set -eu

REPO="amitozalvo/mesimon-releases"
PREFIX="${PREFIX:-$HOME/.local/bin}"
VERSION=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --prefix)  PREFIX="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }

# --- preconditions, each with the exact line that fixes it -------------------

# One asset per platform. The Linux builds are static musl, so one binary
# runs on every distro — and under WSL2, which is the way in on Windows.
os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64)               target="aarch64-apple-darwin" ;;
  Linux/x86_64)               target="x86_64-unknown-linux-musl" ;;
  Linux/aarch64|Linux/arm64)  target="aarch64-unknown-linux-musl" ;;
  *) die "no published build for $os $arch (published: macOS arm64, Linux x86_64 and aarch64)" \
       "build from source instead — ask for access to the repo" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required" \
  "$([ "$os" = Darwin ] && echo 'it ships with macOS — check your PATH' || echo 'sudo apt install curl')"

command -v git >/dev/null 2>&1 || die "git is required" \
  "$([ "$os" = Darwin ] && echo 'xcode-select --install' || echo 'sudo apt install git')"

# macOS gets a bundled tmux (below). The Linux build does not carry one: every
# distro packages a tmux mesimon runs on, and `mesimon doctor` names the
# version floor. Not fatal here — you can browse a board without an agent.
if [ "$os" = Linux ] && ! command -v tmux >/dev/null 2>&1; then
  echo "note: tmux is not on your PATH. mesimon needs it to run agents:"
  echo "      sudo apt install tmux      (3.3 or newer; mesimon doctor checks)"
fi

# Not fatal: you can browse a board without ever spawning an agent.
command -v claude >/dev/null 2>&1 || {
  echo "note: 'claude' is not on your PATH. mesimon will run, but spawning a"
  echo "      Claude session will fail until Claude Code is installed."
}

# --- resolve the version ------------------------------------------------------

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -z "$VERSION" ]; then
  # NOT /releases/latest — that endpoint skips prereleases, and every alpha is
  # one, so it would 404 until the first stable build. The list endpoint is
  # newest-first and includes them.
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases" 2>/dev/null \
    | grep -m1 '"tag_name"' \
    | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
  [ -n "$VERSION" ] || die "could not find a published release on $REPO" \
    "check https://github.com/$REPO/releases"
fi

asset="mesimon-$VERSION-$target.tar.gz"
base="https://github.com/$REPO/releases/download/$VERSION"

echo "downloading $asset"
curl -fsSL -o "$tmp/$asset" "$base/$asset" \
  || die "could not download $asset" "check https://github.com/$REPO/releases"
curl -fsSL -o "$tmp/$asset.sha256" "$base/$asset.sha256" 2>/dev/null || true

# --- verify -------------------------------------------------------------------

if [ -s "$tmp/$asset.sha256" ]; then
  # The file is `shasum`'s `<hex>  <name>` line; coreutils' sha256sum reads it
  # unchanged, and every Linux has one of the two.
  if command -v sha256sum >/dev/null 2>&1; then
    sum="sha256sum -c"
  elif command -v shasum >/dev/null 2>&1; then
    sum="shasum -a 256 -c"
  else
    die "no sha256 tool to verify the download (need sha256sum or shasum)"
  fi
  ( cd "$tmp" && $sum "$asset.sha256" >/dev/null ) \
    || die "checksum mismatch on $asset — refusing to install" \
       "re-run; if it persists, the release asset is corrupt"
  echo "checksum ok"
else
  echo "note: no .sha256 published for this release; skipping verification"
fi

tar -xzf "$tmp/$asset" -C "$tmp"
unpacked="$tmp/mesimon-$VERSION-$target"
bin="$unpacked/mesimon"
[ -x "$bin" ] || die "the archive did not contain an executable" "report this with the release tag"

# Run it BEFORE installing: an arm64 Mac binary with a broken signature dies
# with "Killed: 9", and finding that out now beats finding out from a wedged
# board. On Linux this is the static binary proving it is one.
"$bin" --version >/dev/null || die "the downloaded binary would not run" \
  "report this with the output of: $bin --version"

# --- install ------------------------------------------------------------------

mkdir -p "$PREFIX"
# Same path every time, replaced atomically. Load-bearing: hook settings embed
# this absolute path per session, and the update offer watches its mtime.
mv "$bin" "$PREFIX/mesimon.new"
chmod +x "$PREFIX/mesimon.new"
mv "$PREFIX/mesimon.new" "$PREFIX/mesimon"

# tmux goes NEXT TO mesimon, under a name that will not shadow yours on PATH.
# mesimon prefers this sibling over whatever `tmux` means to your shell, so
# your own tmux, its version, and its config are left completely alone.
if [ -x "$unpacked/mesimon-tmux" ]; then
  mv "$unpacked/mesimon-tmux" "$PREFIX/mesimon-tmux.new"
  chmod +x "$PREFIX/mesimon-tmux.new"
  mv "$PREFIX/mesimon-tmux.new" "$PREFIX/mesimon-tmux"
  echo "installed $("$PREFIX/mesimon-tmux" -V) -> $PREFIX/mesimon-tmux (mesimon's own; yours is untouched)"
fi

echo
echo "installed $("$PREFIX/mesimon" --version) -> $PREFIX/mesimon"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *)
    # The line for the shell you are actually in: zsh on a Mac, bash on most
    # Linux and every fresh WSL distro.
    case "${SHELL:-}" in
      */zsh) rc="~/.zshrc"; sh="zsh" ;;
      *)     rc="~/.bashrc"; sh="bash" ;;
    esac
    echo
    echo "$PREFIX is not on your PATH. Add it:"
    echo "  echo 'export PATH=\"$PREFIX:\$PATH\"' >> $rc && exec $sh"
    ;;
esac

echo
echo "next:  mesimon doctor      check your environment"
echo "       cd <a git repo> && mesimon"
