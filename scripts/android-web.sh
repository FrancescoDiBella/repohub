#!/usr/bin/env bash
# Builds the page the Android app shows (RepoHub compiled to WebAssembly) into
# android/app/src/main/assets/web. Needs the wasm32-unknown-unknown target and wasm-bindgen-cli
# at the version in Cargo.lock (see CONTRIBUTING.md, "Web build").
set -euo pipefail
cd "$(dirname "$0")/.."
out=android/app/src/main/assets/web
cargo build --release --lib --target wasm32-unknown-unknown
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" --out-name app target/wasm32-unknown-unknown/release/repohub.wasm
cp web/index.html web/icon.svg "$out/"
ls -l "$out"
