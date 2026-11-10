#!/usr/bin/env bash
# Builds dist/RepoHub.app (Apple silicon and Intel in one binary) and dist/RepoHub-<version>-macos.dmg.
# Run on macOS from the repository root:  packaging/macos/bundle.sh 0.1.0
set -euo pipefail
VERSION="${1:?usage: packaging/macos/bundle.sh <version>}"
export MACOSX_DEPLOYMENT_TARGET=11.0

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --bin repohub --target aarch64-apple-darwin
cargo build --release --bin repohub --target x86_64-apple-darwin

APP=dist/RepoHub.app
rm -rf dist
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/RepoHub" \
    target/aarch64-apple-darwin/release/repohub target/x86_64-apple-darwin/release/repohub
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# The icon, in every size macOS asks for.
ICONSET=dist/RepoHub.iconset
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" packaging/icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" packaging/icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/RepoHub.icns"
rm -rf "$ICONSET"

# An ad-hoc signature: enough to run on Apple silicon, but not notarized (see the README).
codesign --force --deep --sign - "$APP"

# The disk image: the app next to a shortcut to Applications.
STAGE=dist/dmg
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname RepoHub -srcfolder "$STAGE" -ov -format UDZO "dist/RepoHub-$VERSION-macos.dmg"
rm -rf "$STAGE"
echo "Built dist/RepoHub-$VERSION-macos.dmg"
