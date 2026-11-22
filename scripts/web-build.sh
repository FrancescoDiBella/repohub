#!/usr/bin/env bash
# Builds the web app (RepoHub compiled to WebAssembly) into a folder: index.html, app.js,
# app_bg.wasm, snippets/, the manifest and the icon. The Dockerfile, scripts/android-web.sh and the
# Pages workflow all use it, so the three ship the same files.
#   scripts/web-build.sh <folder>                 build into <folder> (made if missing)
#   scripts/web-build.sh <folder> --precompress   also write .br and .gz copies, which
#                                                 repohub-server sends to browsers that accept them
# Needs the wasm32-unknown-unknown target, wasm-bindgen-cli at the version in Cargo.lock (0.2.129)
# and wasm-opt (scripts/install-wasm-opt.sh); --precompress also needs brotli. Without wasm-opt it
# warns and ships the unoptimized .wasm, except in CI (CI=true), where it fails.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/web-build.sh <folder> [--precompress]}
precompress=${2:-}
mkdir -p "$out"
if [ "$(cd "$out" && pwd -P)" = "$(cd web && pwd -P)" ]; then
    echo "web-build.sh: build into another folder than web/, whose index.html is the source" >&2
    exit 1
fi

# The `web` profile (Cargo.toml) is the release profile tuned for the download's size.
cargo build --profile web --lib --target wasm32-unknown-unknown
wasm=target/wasm32-unknown-unknown/web/repohub.wasm

mkdir -p "$out"
rm -rf "$out/snippets"
rm -f "$out"/app.js "$out"/app_bg.wasm "$out"/*.br "$out"/*.gz
wasm-bindgen --target web --no-typescript --out-dir "$out" --out-name app "$wasm"

# wasm-opt, light: -O1 and merging identical functions make the file 3% smaller and its gzip and
# brotli 1% and 0.4% smaller. -Oz and -O3 make the file 10% smaller but what the browser
# downloads (compressed) 2-4% bigger, and memory-packing alone makes it 1% bigger: measured in
# CONTRIBUTING, "Size". The features are the ones Rust's wasm32 target turns on (every browser
# with WebGPU has them).
if command -v wasm-opt > /dev/null; then
    wasm-opt -O1 --duplicate-function-elimination --skip-pass=memory-packing \
        --strip-debug --strip-producers \
        --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
        --enable-mutable-globals --enable-reference-types --enable-multivalue \
        "$out/app_bg.wasm" -o "$out/app_bg.wasm"
elif [ "${CI:-}" = true ]; then
    echo "web-build.sh: wasm-opt is missing (scripts/install-wasm-opt.sh)" >&2
    exit 1
else
    echo "web-build.sh: wasm-opt not found, so app_bg.wasm is not optimized (scripts/install-wasm-opt.sh)" >&2
fi

# The page names the two files with a hash of the build (`app.js?v=…`), so a browser may keep them
# for a long time and still fetch the new ones after an update (repohub-server's Cache-Control).
# (`shasum` where there is no `sha256sum`: a Mac.)
hash() { if command -v sha256sum > /dev/null; then sha256sum; else shasum -a 256; fi; }
version=$(cat "$out/app.js" "$out/app_bg.wasm" | hash | cut -c1-16)
sed -e "s|'./app.js'|'./app.js?v=$version'|" -e "s|'./app_bg.wasm'|'./app_bg.wasm?v=$version'|" web/index.html > "$out/index.html"
if [ "$(grep -c "?v=$version" "$out/index.html")" != 2 ]; then
    echo "web-build.sh: web/index.html no longer names './app.js' and './app_bg.wasm' once each" >&2
    exit 1
fi
cp web/manifest.webmanifest web/icon.svg "$out/"

if [ "$precompress" = --precompress ]; then
    command -v brotli > /dev/null || { echo "web-build.sh: --precompress needs brotli" >&2; exit 1; }
    find "$out" -type f \( -name '*.js' -o -name '*.wasm' -o -name '*.html' -o -name '*.svg' -o -name '*.webmanifest' \) |
        while read -r f; do
            gzip -9 -n -k -f "$f"
            brotli -q 11 -f -o "$f.br" "$f"
        done
fi
ls -l "$out"
