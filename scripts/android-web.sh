#!/usr/bin/env bash
# Builds the page the Android app shows (RepoHub compiled to WebAssembly) into
# android/app/src/main/assets/web, with scripts/web-build.sh (see there for what it needs).
# No .br or .gz copies: the APK compresses its files itself, and the WebView wouldn't decode them.
set -euo pipefail
cd "$(dirname "$0")/.."
out=android/app/src/main/assets/web
rm -rf "$out"
scripts/web-build.sh "$out"
