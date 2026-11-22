#!/usr/bin/env bash
# Installs wasm-opt (from Binaryen's release, at a pinned version whose checksum is checked) into
# a folder, for scripts/web-build.sh. Linux on x86_64 or arm64.
#   scripts/install-wasm-opt.sh ~/.local/bin      then make sure that folder is on PATH
# The Dockerfile, the Pages workflow, CI and the Android build use it; on a Mac,
# `brew install binaryen` gives the same tool.
set -euo pipefail
dir=${1:?usage: scripts/install-wasm-opt.sh <folder>}
version=132
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64; sum=195ddc94f9bc89f45abdabb0b9eea86023d727ba90eac8b35b80f2544fc30572 ;;
    aarch64 | arm64) arch=aarch64; sum=c58562417836c5d0493d89bdefc434933bdc097db641b483df86bcfa557a107f ;;
    *) echo "install-wasm-opt.sh: no Binaryen build for $(uname -m)" >&2; exit 1 ;;
esac
if [ -x "$dir/wasm-opt" ] && "$dir/wasm-opt" --version 2>/dev/null | grep -q "version $version\b"; then
    exit 0
fi
name=binaryen-version_$version-$arch-linux.tar.gz
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL --retry 3 -o "$tmp/$name" "https://github.com/WebAssembly/binaryen/releases/download/version_$version/$name"
echo "$sum  $tmp/$name" | sha256sum -c --quiet
# The binary is statically linked: nothing else from the archive is needed.
tar -xzf "$tmp/$name" -C "$tmp" "binaryen-version_$version/bin/wasm-opt"
mkdir -p "$dir"
install -m 755 "$tmp/binaryen-version_$version/bin/wasm-opt" "$dir/wasm-opt"
"$dir/wasm-opt" --version
