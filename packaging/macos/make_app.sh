#!/usr/bin/env bash
# Builds dist/Gibson.app (universal: Apple Silicon + Intel) and dist/Gibson-macos.zip.
# Run on a Mac after:
#   cargo build --release --target aarch64-apple-darwin
#   cargo build --release --target x86_64-apple-darwin
# The app is ad-hoc signed only (no Apple developer ID), so macOS calls it
# "unidentified": see the README for how to open it the first time.
set -euo pipefail
cd "$(dirname "$0")/../.."

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP=dist/Gibson.app
rm -rf dist
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# One binary for both kinds of Mac.
lipo -create \
    target/aarch64-apple-darwin/release/gibson \
    target/x86_64-apple-darwin/release/gibson \
    -output "$APP/Contents/MacOS/gibson"

# Icon: every size macOS wants, from the 1024 px master.
ICONSET=dist/Gibson.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s assets/icon/icon_1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z $d $d assets/icon/icon_1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Gibson.icns"
rm -rf "$ICONSET"

sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# Ad-hoc signature: required for the arm64 binary to run at all.
codesign --force --deep --sign - "$APP"

(cd dist && ditto -c -k --keepParent Gibson.app Gibson-macos.zip)
echo "built $APP ($VERSION)"
