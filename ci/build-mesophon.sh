#!/bin/sh
# Apache browser client; generated assets stay out of source control.
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release -p mesimon-web --target wasm32-unknown-unknown
"${WASM_BINDGEN:-wasm-bindgen}" --target web --out-dir web/mesophon/pkg target/wasm32-unknown-unknown/release/mesimon_web.wasm
