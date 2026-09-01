#!/usr/bin/env bash
# Build a relocatable tmux for aarch64-apple-darwin, to ship beside mesimon.
#
# Why build rather than copy: Homebrew's tmux links libevent, ncursesw and
# utf8proc out of /opt/homebrew, so it dies with a dyld error on any machine
# without those exact paths. This links all three statically; only libSystem
# and libresolv stay dynamic, which is the most macOS permits.
#
# Why bundle at all: mesimon already treats tmux as a private implementation
# detail — its own server, own socket, own generated conf, never the user's
# tmux. Owning the binary finishes that, and removes a whole bug class where a
# tester on 3.2a hits behaviour the author cannot reproduce on 3.6a.
#
# Terminfo is NOT bundled. macOS ships a terminfo database at
# /usr/share/terminfo (xterm-256color included), and ncurses is configured to
# read it, so the result is one self-contained executable with no data files.
#
# Usage:  ci/build-tmux.sh          build if vendor/tmux/tmux is missing
#         ci/build-tmux.sh --force  rebuild from scratch

set -euo pipefail
cd "$(dirname "$0")/.."

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

VENDOR="$PWD/vendor"
CACHE="$VENDOR/cache"
BUILD="$VENDOR/build"
STAGE="$VENDOR/stage"     # static libs + headers, never shipped
OUT="$VENDOR/tmux"        # the shipped artifact

die() { echo "error: $1" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

[ "$(uname -sm)" = "Darwin arm64" ] || die "this builds for aarch64-apple-darwin"

verify() {
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

  step "smoke test (a real server on a private socket)"
  # The socket lives in a temp DIRECTORY: macOS mktemp wants its X's at the end
  # of the template, so a `-XXXXXX.sock` suffix fails outright.
  local dir sock
  dir=$(mktemp -d)
  sock="$dir/tmux.sock"
  "$OUT/tmux" -S "$sock" new-session -d -s probe 'sleep 30'
  "$OUT/tmux" -S "$sock" list-sessions | sed 's/^/  /'
  "$OUT/tmux" -S "$sock" kill-server 2>/dev/null || true
  rm -rf "$dir"
}

if [ "${1:-}" = "--force" ]; then
  rm -rf "$BUILD" "$STAGE" "$OUT"
elif [ -x "$OUT/tmux" ]; then
  echo "vendor/tmux/tmux already built; verifying rather than trusting it"
  verify
  exit 0
fi

mkdir -p "$CACHE" "$BUILD" "$STAGE" "$OUT"

fetch() { # url filename sha256
  local url="$1" file="$2" want="$3"
  if [ ! -f "$CACHE/$file" ]; then
    echo "fetching $file"
    curl -fsSL -o "$CACHE/$file" "$url" || die "could not download $file"
  fi
  local got
  got=$(shasum -a 256 "$CACHE/$file" | cut -d' ' -f1)
  [ "$got" = "$want" ] || die "checksum mismatch on $file
  want $want
  got  $got"
}

unpack() { # file  -> echoes the extracted directory
  local file="$1" dir
  dir="$BUILD/$(tar -tzf "$CACHE/$file" | head -1 | cut -d/ -f1)"
  [ -d "$dir" ] || tar -xzf "$CACHE/$file" -C "$BUILD"
  echo "$dir"
}

step "fetch"
fetch "https://github.com/tmux/tmux/releases/download/$TMUX_V/tmux-$TMUX_V.tar.gz" \
      "tmux-$TMUX_V.tar.gz" "$SHA_TMUX"
fetch "https://github.com/libevent/libevent/releases/download/release-$LIBEVENT_V/libevent-$LIBEVENT_V.tar.gz" \
      "libevent-$LIBEVENT_V.tar.gz" "$SHA_LIBEVENT"
fetch "https://ftp.gnu.org/gnu/ncurses/ncurses-$NCURSES_V.tar.gz" \
      "ncurses-$NCURSES_V.tar.gz" "$SHA_NCURSES"
fetch "https://github.com/JuliaStrings/utf8proc/archive/refs/tags/v$UTF8PROC_V.tar.gz" \
      "utf8proc-$UTF8PROC_V.tar.gz" "$SHA_UTF8PROC"

export MACOSX_DEPLOYMENT_TARGET=13.0
JOBS=$(sysctl -n hw.ncpu)

step "libevent $LIBEVENT_V (static)"
d=$(unpack "libevent-$LIBEVENT_V.tar.gz")
( cd "$d" && ./configure --prefix="$STAGE" \
    --disable-shared --enable-static --with-pic \
    --disable-openssl --disable-samples --disable-libevent-regress --disable-debug-mode \
    >/dev/null && make -j"$JOBS" >/dev/null && make install >/dev/null )

step "ncurses $NCURSES_V (static, wide, system terminfo)"
d=$(unpack "ncurses-$NCURSES_V.tar.gz")
# --with-default-terminfo-dir is the load-bearing flag: it makes the static
# ncurses read macOS's own terminfo database, so nothing has to be shipped
# alongside the binary and no TERM lookup depends on our install location.
( cd "$d" && ./configure --prefix="$STAGE" \
    --without-shared --with-normal --enable-widec --enable-pc-files \
    --with-pkg-config-libdir="$STAGE/lib/pkgconfig" \
    --with-default-terminfo-dir=/usr/share/terminfo \
    --with-terminfo-dirs="/usr/share/terminfo:/usr/local/share/terminfo" \
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
# Static only — the shared target would be an extra dylib to relocate, and
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
( cd "$d" && PKG_CONFIG_PATH="$STAGE/lib/pkgconfig" \
    CPPFLAGS="-I$STAGE/include -I$STAGE/include/ncursesw" \
    LDFLAGS="-L$STAGE/lib" \
    ./configure --prefix="$STAGE" --enable-utf8proc >/dev/null && \
  make -j"$JOBS" >/dev/null )
cp "$d/tmux" "$OUT/tmux"

# Licenses travel with the binary — ISC (tmux), MIT-ish X11 (ncurses),
# 3-clause BSD (libevent), MIT (utf8proc).
mkdir -p "$OUT/licenses"
cp "$BUILD/tmux-$TMUX_V/COPYING" "$OUT/licenses/tmux-COPYING" 2>/dev/null || true
cp "$BUILD/libevent-$LIBEVENT_V/LICENSE" "$OUT/licenses/libevent-LICENSE" 2>/dev/null || true
cp "$BUILD/ncurses-$NCURSES_V/COPYING" "$OUT/licenses/ncurses-COPYING" 2>/dev/null || true
cp "$BUILD/utf8proc-$UTF8PROC_V/LICENSE.md" "$OUT/licenses/utf8proc-LICENSE" 2>/dev/null || true

verify

echo
echo "built vendor/tmux/tmux"
