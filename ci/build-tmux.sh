#!/usr/bin/env bash
# Build a relocatable tmux to ship beside mesimon, for one of the three
# published targets.
#
# Why build rather than copy: Homebrew's tmux links libevent, ncursesw and
# utf8proc out of /opt/homebrew, so it dies with a dyld error on any machine
# without those exact paths. This links all three statically; on macOS only
# libSystem and libresolv stay dynamic, which is the most macOS permits, and
# the Linux builds are fully static against musl, so one file per
# architecture runs on every distro and under WSL2.
#
# Why bundle at all: mesimon already treats tmux as a private implementation
# detail — its own server, own socket, own generated conf, never the user's
# tmux. Owning the binary finishes that, and removes a whole bug class where a
# tester on 3.2a hits behaviour the author cannot reproduce on 3.6a. On Linux
# it is also what makes a pane's links reach the terminal (T-1095: tmux keeps
# OSC 8 only from 3.4, and Ubuntu 22.04 ships 3.2a) and what makes T-10's
# passthrough containment a line tmux honours (`allow-passthrough` is 3.3+).
#
# Terminfo is NOT bundled. macOS ships a terminfo database at
# /usr/share/terminfo (xterm-256color included), and ncurses is configured to
# read it; on Linux the static ncurses searches the directories every distro
# family uses (and honours TERMINFO/TERMINFO_DIRS), with the common entries
# compiled in as fallbacks for a box with no database at all. Either way the
# result is one self-contained executable with no data files.
#
# The Linux targets build inside an Alpine container (musl, static-friendly)
# and are verified inside a Debian container of their architecture — the
# glibc distro a `sudo apt install` box actually is. Docker Desktop must be
# running for them; the macOS target needs no Docker.
#
# Usage:  ci/build-tmux.sh [target]          build if the artifact is missing
#         ci/build-tmux.sh --force [target]  rebuild from scratch
#
# Targets: aarch64-apple-darwin (the default), x86_64-unknown-linux-musl,
# aarch64-unknown-linux-musl. The macOS artifact is vendor/tmux/tmux; a Linux
# one is vendor/tmux-<target>/tmux. Licenses sit in `licenses/` beside each.

set -euo pipefail

TMUX_V=3.6a
LIBEVENT_V=2.1.12-stable
NCURSES_V=6.5
UTF8PROC_V=2.9.0

# Pinned so a later build cannot silently pick up different upstream bytes.
# Established by downloading once, on 2026-09-01, from the canonical release
# URLs below — that is provenance by first use, not by an independent chain of
# trust, and is worth knowing before this ships to anyone outside the project.
SHA_TMUX=b6d8d9c76585db8ef5fa00d4931902fa4b8cbe8166f528f44fc403961a3f3759
SHA_LIBEVENT=92e6de1be9ec176428fd2367677e61ceffc2ee1cb119035037a27d346b0403bb
SHA_NCURSES=136d91bc269a9a5785e5f9e980bc76ab57428f604ce3e5a5a90cebc767971cc6
SHA_UTF8PROC=18c1626e9fc5a2e192311e36b3010bfc698078f692888940f1fa150547abb0c1

# The build image for the Linux targets. Pinned by tag, not digest: a new
# patch release of Alpine changes the compiler's bytes, never the pinned
# sources above, and the verify step below is what the artifact must pass.
ALPINE_IMAGE=alpine:3.21
DEBIAN_IMAGE=debian:bookworm-slim

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# --- arguments ---------------------------------------------------------------

FORCE=0
INSIDE=0
TARGET=""
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1 ;;
    --inside) INSIDE=1 ;;   # running inside the Alpine container (below)
    -*) die "unknown flag: $1" ;;
    *) [ -z "$TARGET" ] || die "one target at a time (got $TARGET and $1)"; TARGET="$1" ;;
  esac
  shift
done
TARGET="${TARGET:-aarch64-apple-darwin}"

case "$TARGET" in
  aarch64-apple-darwin)        OS=darwin; PLATFORM="" ;;
  x86_64-unknown-linux-musl)   OS=linux;  PLATFORM=linux/amd64 ;;
  aarch64-unknown-linux-musl)  OS=linux;  PLATFORM=linux/arm64 ;;
  *) die "no recipe for $TARGET" \
       "one of: aarch64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl" ;;
esac

# --- layout ------------------------------------------------------------------

if [ "$INSIDE" = 1 ]; then
  # Inside the container: the host mounted its tarball cache read-only at
  # /cache and the artifact directory at /out; everything else is scratch.
  CACHE=/cache
  BUILD=/build
  STAGE=/stage
  OUT=/out
else
  cd "$(dirname "$0")/.."
  VENDOR="$PWD/vendor"
  CACHE="$VENDOR/cache"
  if [ "$OS" = darwin ]; then
    BUILD="$VENDOR/build"
    STAGE="$VENDOR/stage"     # static libs + headers, never shipped
    OUT="$VENDOR/tmux"        # the shipped artifact
  else
    OUT="$VENDOR/tmux-$TARGET"
  fi
fi
REL_OUT="${OUT#"${PWD:-}/"}"

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else sha256sum "$1" | cut -d' ' -f1; fi
}

fetch() { # url filename sha256
  local url="$1" file="$2" want="$3"
  if [ ! -f "$CACHE/$file" ]; then
    [ "$INSIDE" = 0 ] || die "$file is missing from the mounted cache"
    echo "fetching $file"
    curl -fsSL -o "$CACHE/$file" "$url" || die "could not download $file"
  fi
  local got
  got=$(sha256_of "$CACHE/$file")
  [ "$got" = "$want" ] || die "checksum mismatch on $file
  want $want
  got  $got"
}

fetch_all() {
  step "fetch"
  fetch "https://github.com/tmux/tmux/releases/download/$TMUX_V/tmux-$TMUX_V.tar.gz" \
        "tmux-$TMUX_V.tar.gz" "$SHA_TMUX"
  fetch "https://github.com/libevent/libevent/releases/download/release-$LIBEVENT_V/libevent-$LIBEVENT_V.tar.gz" \
        "libevent-$LIBEVENT_V.tar.gz" "$SHA_LIBEVENT"
  fetch "https://ftp.gnu.org/gnu/ncurses/ncurses-$NCURSES_V.tar.gz" \
        "ncurses-$NCURSES_V.tar.gz" "$SHA_NCURSES"
  fetch "https://github.com/JuliaStrings/utf8proc/archive/refs/tags/v$UTF8PROC_V.tar.gz" \
        "utf8proc-$UTF8PROC_V.tar.gz" "$SHA_UTF8PROC"
}

unpack() { # file  -> echoes the extracted directory
  local file="$1" dir
  dir="$BUILD/$(tar -tzf "$CACHE/$file" | head -1 | cut -d/ -f1)"
  [ -d "$dir" ] || tar -xzf "$CACHE/$file" -C "$BUILD"
  echo "$dir"
}

# A real server on a private socket, from the built binary.
smoke() { # tmux-binary
  # The socket lives in a temp DIRECTORY: macOS mktemp wants its X's at the end
  # of the template, so a `-XXXXXX.sock` suffix fails outright.
  local dir sock
  dir=$(mktemp -d)
  sock="$dir/tmux.sock"
  "$1" -S "$sock" new-session -d -s probe 'sleep 30'
  "$1" -S "$sock" list-sessions | sed 's/^/  /'
  "$1" -S "$sock" kill-server 2>/dev/null || true
  rm -rf "$dir"
}

# OSC 8 (`Hls`) is compiled in only when tmux sees an ncurses newer than 5.8
# (tty-features.c); the whole Linux bundle exists for that capability, so a
# build without it is a failed build.
check_hyperlinks() { # tmux-binary
  grep -aq 'Hls=' "$1" || die "the built tmux has no Hls (OSC 8) capability: ncurses was not seen at configure time"
  echo "  OSC 8 hyperlinks: compiled in"
}

# --- the recipe, shared by both platforms ----------------------------------------

build() {
  mkdir -p "$CACHE" "$BUILD" "$STAGE" "$OUT"
  fetch_all

  local JOBS d
  if [ "$OS" = darwin ]; then
    export MACOSX_DEPLOYMENT_TARGET=13.0
    JOBS=$(sysctl -n hw.ncpu)
  else
    JOBS=$(nproc)
  fi

  step "libevent $LIBEVENT_V (static)"
  d=$(unpack "libevent-$LIBEVENT_V.tar.gz")
  ( cd "$d" && ./configure --prefix="$STAGE" \
      --disable-shared --enable-static --with-pic \
      --disable-openssl --disable-samples --disable-libevent-regress --disable-debug-mode \
      >/dev/null && make -j"$JOBS" >/dev/null && make install >/dev/null )

  step "ncurses $NCURSES_V (static, wide, system terminfo)"
  d=$(unpack "ncurses-$NCURSES_V.tar.gz")
  # --with-default-terminfo-dir is the load-bearing flag: it makes the static
  # ncurses read the system's own terminfo database, so nothing has to be
  # shipped alongside the binary and no TERM lookup depends on our install
  # location. On Linux the database moves by distro family — Debian and
  # Ubuntu keep the common entries in /lib/terminfo and /etc/terminfo, Fedora
  # and Alpine in /usr/share/terminfo — so the search list names them all,
  # and the entries a board meets are compiled in as fallbacks for a minimal
  # box with no database (the build host's `tic`/`infocmp` produce them).
  local terminfo=()
  if [ "$OS" = darwin ]; then
    terminfo=(--with-default-terminfo-dir=/usr/share/terminfo
              --with-terminfo-dirs="/usr/share/terminfo:/usr/local/share/terminfo")
  else
    terminfo=(--with-default-terminfo-dir=/usr/share/terminfo
              --with-terminfo-dirs="/etc/terminfo:/lib/terminfo:/usr/share/terminfo:/usr/lib/terminfo:/usr/local/share/terminfo"
              --with-fallbacks=xterm,xterm-256color,screen,screen-256color,tmux,tmux-256color,linux,vt100)
  fi
  ( cd "$d" && ./configure --prefix="$STAGE" \
      --without-shared --with-normal --enable-widec --enable-pc-files \
      --with-pkg-config-libdir="$STAGE/lib/pkgconfig" \
      "${terminfo[@]}" \
      --without-debug --without-ada --without-manpages --without-progs \
      --without-tests --without-cxx-binding --disable-db-install \
      >/dev/null && make -j"$JOBS" >/dev/null 2>&1 && make install >/dev/null 2>&1 )
  # tmux's configure asks pkg-config for `ncurses`, not `ncursesw`, and the macOS
  # SDK ships an ncurses.pc — which wins, and quietly links the system's ancient
  # non-wide libncurses.5.4 instead of what we just built. Answer to that name
  # ourselves; PKG_CONFIG_PATH is searched before the system directories.
  cp "$STAGE/lib/pkgconfig/ncursesw.pc" "$STAGE/lib/pkgconfig/ncurses.pc"

  step "utf8proc $UTF8PROC_V (static)"
  d=$(unpack "utf8proc-$UTF8PROC_V.tar.gz")
  # Static only — the shared target would be an extra library to relocate, and
  # tmux only needs the archive.
  ( cd "$d" && make -j"$JOBS" libutf8proc.a >/dev/null && \
    mkdir -p "$STAGE/lib" "$STAGE/include" && \
    cp libutf8proc.a "$STAGE/lib/" && cp utf8proc.h "$STAGE/include/" )
  cat > "$STAGE/lib/pkgconfig/libutf8proc.pc" <<PC
prefix=$STAGE
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: libutf8proc
Description: utf8proc
Version: $UTF8PROC_V
Libs: -L\${libdir} -lutf8proc
Cflags: -I\${includedir}
PC

  step "tmux $TMUX_V"
  d=$(unpack "tmux-$TMUX_V.tar.gz")
  # --enable-static is Linux-only: it passes -static to the link, which musl
  # honours and macOS refuses (libSystem has no static form).
  local static=()
  [ "$OS" = darwin ] || static=(--enable-static)
  ( cd "$d" && PKG_CONFIG_PATH="$STAGE/lib/pkgconfig" \
      CPPFLAGS="-I$STAGE/include -I$STAGE/include/ncursesw" \
      LDFLAGS="-L$STAGE/lib" \
      ./configure --prefix="$STAGE" --enable-utf8proc "${static[@]}" >/dev/null && \
    make -j"$JOBS" >/dev/null )
  cp "$d/tmux" "$OUT/tmux"

  # Licenses travel with the binary — ISC (tmux), MIT-ish X11 (ncurses),
  # 3-clause BSD (libevent), MIT (utf8proc).
  mkdir -p "$OUT/licenses"
  cp "$BUILD/tmux-$TMUX_V/COPYING" "$OUT/licenses/tmux-COPYING" 2>/dev/null || true
  cp "$BUILD/libevent-$LIBEVENT_V/LICENSE" "$OUT/licenses/libevent-LICENSE" 2>/dev/null || true
  cp "$BUILD/ncurses-$NCURSES_V/COPYING" "$OUT/licenses/ncurses-COPYING" 2>/dev/null || true
  cp "$BUILD/utf8proc-$UTF8PROC_V/LICENSE.md" "$OUT/licenses/utf8proc-LICENSE" 2>/dev/null || true
}

# --- macOS -------------------------------------------------------------------------

verify_darwin() {
  step "verify"
  # A strict allowlist, not just "nothing outside /usr/lib". The first build of
  # this script passed that weaker check while silently linking the system's
  # libncurses.5.4 — relocatable, but not the pinned stack the bundling exists
  # to guarantee. Anything not on this list is a linkage we did not choose.
  local allowed='^/usr/lib/(libSystem\.B|libresolv\.9)\.dylib$'
  local unexpected
  unexpected=$(otool -L "$OUT/tmux" | tail -n +2 | awk '{print $1}' | grep -Ev "$allowed" || true)
  [ -z "$unexpected" ] || die "tmux links libraries it should have static:
$unexpected"
  otool -L "$OUT/tmux" | tail -n +2 | awk '{print "  " $1}'
  codesign -v --verbose=2 "$OUT/tmux" 2>&1 | sed 's/^/  /' || \
    die "the built tmux has no valid signature"
  echo "  $("$OUT/tmux" -V)"
  check_hyperlinks "$OUT/tmux"

  step "smoke test (a real server on a private socket)"
  smoke "$OUT/tmux"
}

# The driver for every target: a cached binary is verified rather than
# trusted, --force removes what the target's build left ($2…), and a fresh
# build ($1, a function) is verified by `verify_$OS`.
run() {
  local build_step=$1
  shift
  if [ "$FORCE" = 1 ]; then
    rm -rf "$@"
  elif [ -x "$OUT/tmux" ]; then
    echo "$REL_OUT/tmux already built; verifying rather than trusting it"
    "verify_$OS"
    exit 0
  fi
  "$build_step"
  "verify_$OS"
  echo
  echo "built $REL_OUT/tmux"
}

run_darwin() {
  [ "$(uname -sm)" = "Darwin arm64" ] || die "$TARGET builds on Apple Silicon macOS only (this is $(uname -sm))"
  run build "$BUILD" "$STAGE" "$OUT"
}

# --- Linux ---------------------------------------------------------------------------

docker_ready() {
  command -v docker >/dev/null 2>&1 || die "docker is not on PATH; the $TARGET tmux builds in a container" "install Docker Desktop"
  docker info >/dev/null 2>&1 || die "the docker daemon is not running" "open -a Docker"
}

# Inside the Alpine container: the recipe, then the facts only the container
# can check about its own output.
run_inside() {
  build
  # Stripped: a static ELF carries no signature to invalidate (the macOS one
  # is left whole for that reason), and the debug info is half the file.
  strip "$OUT/tmux"
  step "verify (in the build container)"
  # Static, or it is not the artifact described above: `ldd` on a static
  # binary says so on its first line (musl's wording), and `file` agrees.
  local desc
  desc=$(file "$OUT/tmux")
  case "$desc" in
    *"statically linked"*|*"static-pie linked"*) echo "  $desc" ;;
    *) die "the built tmux is not static: $desc" ;;
  esac
  echo "  $("$OUT/tmux" -V)"
  check_hyperlinks "$OUT/tmux"
}

# From the host: the built binary on the distro a Linux board really runs on.
# The Debian image is slim — it has the Essential ncurses-base (xterm-256color
# under /lib/terminfo) and bsdutils' `script`, nothing else — so this proves
# the static ncurses finds a Debian database, and that the binary needs no
# library the image lacks.
verify_linux() {
  step "verify ($DEBIAN_IMAGE, $PLATFORM)"
  local said
  said=$(docker run --rm --platform "$PLATFORM" -v "$OUT:/pkg:ro" "$DEBIAN_IMAGE" bash -c '
    set -euo pipefail
    /pkg/tmux -V
    sock=/tmp/probe.sock
    /pkg/tmux -S "$sock" new-session -d -s probe "sleep 30"
    /pkg/tmux -S "$sock" list-sessions
    # A real client attach needs the outer TERM in terminfo. `script` lends a
    # pty; tmux prints "open terminal failed" and exits 1 when the lookup
    # fails, and otherwise stays attached until the timeout ends it (124).
    # `-T hyperlinks` is the flag the board attaches with since T-1095.
    set +e
    TERM=xterm-256color timeout 3 script -qec "/pkg/tmux -T hyperlinks -S $sock attach" /dev/null >/tmp/attach.log 2>&1
    rc=$?
    set -e
    if grep -q "open terminal failed" /tmp/attach.log; then
      echo "terminfo lookup failed:"; cat /tmp/attach.log; exit 1
    fi
    [ "$rc" = 124 ] || { echo "attach ended with $rc:"; cat /tmp/attach.log; exit 1; }
    echo "attached as xterm-256color with -T hyperlinks"
    /pkg/tmux -S "$sock" kill-server
  ') || die "the built $TARGET tmux failed on $DEBIAN_IMAGE:
$said"
  echo "$said" | sed 's/^/  /'
}

build_linux() {
  mkdir -p "$CACHE" "$OUT"
  fetch_all   # on the host, so the container needs no network at all
  step "build in $ALPINE_IMAGE ($PLATFORM)"
  # The container runs THIS script with --inside: one recipe for every target.
  # The cache is mounted read-only, so the container downloads no source;
  # only /out is written. Alpine's ncurses package lends `tic` and `infocmp`
  # for the compiled-in fallbacks.
  docker run --rm --platform "$PLATFORM" \
    -v "$CACHE:/cache:ro" -v "$OUT:/out" -v "$PWD/ci/build-tmux.sh:/build-tmux.sh:ro" \
    "$ALPINE_IMAGE" sh -c '
      set -e
      apk add -q bash build-base linux-headers pkgconf bison ncurses ncurses-terminfo file
      exec bash /build-tmux.sh --inside "$1"
    ' -- "$TARGET" || die "the $TARGET build failed in $ALPINE_IMAGE"
}

run_linux() {
  docker_ready
  run build_linux "$OUT"
}

# --- dispatch --------------------------------------------------------------------------

if [ "$INSIDE" = 1 ]; then
  run_inside
elif [ "$OS" = darwin ]; then
  run_darwin
else
  run_linux
fi
