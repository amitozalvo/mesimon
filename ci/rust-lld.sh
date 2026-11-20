#!/bin/sh
# The Linux cross-build's linker (ci/build-linux.sh). The toolchain's rust-lld
# loads libLLVM.dylib through dyld's fallback path, which rustup sets for
# cargo but which did not reach the linker when cargo ran rustc directly
# (rustc 1.98, 2026-11-20: "Library not loaded: @rpath/libLLVM.dylib").
# Naming the toolchain's lib here makes the link independent of what survives
# the way down; a variable set in this script reaches rust-lld, which is not
# a protected binary.
: "${MESIMON_RUST_SYSROOT:?ci/build-linux.sh sets it}" "${MESIMON_RUST_LLD:?ci/build-linux.sh sets it}"
DYLD_FALLBACK_LIBRARY_PATH="$MESIMON_RUST_SYSROOT/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" \
  exec "$MESIMON_RUST_LLD" "$@"
