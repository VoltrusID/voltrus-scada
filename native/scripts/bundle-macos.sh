#!/usr/bin/env bash
# Wrap the release binary in an app bundle and zip it.
# Usage: native/scripts/bundle-macos.sh <binary> <output dir> [zip name]
set -euo pipefail

binary="$1"
out="$2"
zip="${3:-Voltrus-SCADA-Console-macos-arm64.zip}"
native="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$native/Cargo.toml" | head -1)"
app="$out/Voltrus SCADA.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/voltrus-native"
sed "s/@VERSION@/$version/g" "$native/packaging/Info.plist" > "$app/Contents/Info.plist"

# Ad-hoc signature: required to launch on Apple silicon. It is not a
# Developer ID signature, so Gatekeeper asks before the first launch.
codesign --force --deep --sign - "$app"

(cd "$out" && ditto -c -k --keepParent "Voltrus SCADA.app" "$zip")
echo "Bundled $app ($version) as $out/$zip"
