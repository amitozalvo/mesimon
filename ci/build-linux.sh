#!/usr/bin/env bash
# Build the Linux release binaries — from this Mac, with no Linux in the room.
#
# The workspace's one C dependency is `ring` (rustls' crypto provider, via
# mesimon-team since T-332), so a Linux target needs a musl cross C toolchain
# for that crate's C and assembly — `<arch>-linux-musl-gcc`, the name the `cc`
# crate looks for on its own (the check below names the brew formula). Nothing
# else needs it: the Rust toolchain's own `rust-lld` links, and the musl
# `rust-std` ships its own crt objects and libc. The result is a STATIC
# binary, which is the point of picking musl over glibc — a glibc build is
# only portable to distros with a glibc at least as new as the builder's,
# and a WSL box is as likely to be Ubuntu 22.04 (glibc 2.35) as anything.
# One binary per architecture runs on every distro, and under WSL2.
#
# tmux is NOT bundled for Linux (it is for macOS, `ci/build-tmux.sh`): every
# distro packages a tmux that mesimon now runs on (3.2a up — see the tab note
# on `SEP` in crates/mesimon-backend-tmux/src/lib.rs), and `mesimon doctor`
# names the version floor and the apt line.
#
# Usage:  ci/build-linux.sh [target ...]     default: both targets below
#
# `ci/release.sh` calls this with MESIMON_RELEASE=1 in the environment; run
# by hand it produces a dev build, which is the right default (build.rs).

set -euo pipefail
cd "$(dirname "$0")/.."

# The names are load-bearing: install.sh derives them from `uname`, and
# release.rs carries the same list, pinned by a unit test that reads this file.
DEFAULT_TARGETS="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
targets="${*:-$DEFAULT_TARGETS}"

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH" "source ~/.cargo/env"
command -v rustup >/dev/null 2>&1 || die "rustup is not on PATH"

# The linker ships inside the toolchain, next to the host's rust-std, and is
# not on PATH — so it is addressed by its absolute path.
sysroot=$(rustc --print sysroot)
host=$(rustc -vV | sed -n 's/^host: //p')
lld="$sysroot/lib/rustlib/$host/bin/rust-lld"
[ -x "$lld" ] || die "rust-lld is missing from the toolchain at $lld" \
  "rustup component add rust-std (it ships with every stable toolchain)"

for t in $targets; do
  rustup target list --installed | grep -qx "$t" || \
    die "the $t target is not installed" "rustup target add $t"
done

# `ring` is built by the `cc` crate with the target's own gcc, found by name.
# Apple's clang cannot stand in: its headers `#include_next` a libc this Mac
# does not have (measured on both targets, hosted and -ffreestanding). The
# prebuilt toolchains are Homebrew formulae named after the target, in a
# third-party tap that brew must be told to trust first.
for t in $targets; do
  cc="${t%%-*}-linux-musl-gcc"
  command -v "$cc" >/dev/null 2>&1 || \
    die "$cc is not on PATH; ring needs a C compiler for $t" \
      "brew tap messense/macos-cross-toolchains; brew trust messense/macos-cross-toolchains; brew install messense/macos-cross-toolchains/$t"
done

for t in $targets; do
  # The release gate's clippy compiles the Darwin libc and nothing under
  # `cfg(target_os = "linux")`, and the build below denies no warning — so a
  # warning only this target can raise (the libc crate deprecates its
  # `time_t` alias on musl, T-484) would scroll past. This is that clippy
  # over the target that ships, and over the default members, which are what
  # the binary is built from (the wasm crate is not, and is linted by the
  # gate's own clippy).
  step "clippy --target $t"
  cargo clippy --locked --all-targets --target "$t" -- -D warnings
  step "build --release --target $t"
  upper=$(printf '%s' "$t" | tr 'a-z-' 'A-Z_')
  # Per-target so the host build is untouched; `link-self-contained=yes` is
  # what makes rust-lld use the bundled crt objects rather than look for a
  # Linux sysroot this machine does not have.
  env "CARGO_TARGET_${upper}_LINKER=$lld" \
      "CARGO_TARGET_${upper}_RUSTFLAGS=-C link-self-contained=yes" \
      cargo build --release --locked --target "$t"
  bin="target/$t/release/mesimon"
  [ -x "$bin" ] || die "$bin missing after build"
  # Static, or it is not the artifact described above. `file` says
  # "statically linked" for a plain executable and "static-pie linked" for
  # the x86_64 default; either is what we want, "dynamically" is not.
  desc=$(file "$bin")
  case "$desc" in
    *"static"*) echo "$desc" ;;
    *) die "$bin is not statically linked: $desc" ;;
  esac
done
