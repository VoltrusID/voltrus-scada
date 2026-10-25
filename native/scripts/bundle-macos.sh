#!/usr/bin/env bash
# Wrap the release binary in an app bundle and zip it.
# Usage: [VERSION=x.y.z] native/scripts/bundle-macos.sh <binary> <output dir> [zip name]
# VERSION overrides the bundle version (default: native/Cargo.toml's version).
set -euo pipefail

binary="$1"
out="$2"
zip="${3:-Voltrus-SCADA-Console-macos-arm64.zip}"
native="$(cd "$(dirname "$0")/.." && pwd)"
version="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$native/Cargo.toml" | head -1)}"
app="$out/Voltrus SCADA.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/voltrus-native"
# Licence texts for everything the bundle ships (also inside the zip).
root="$(cd "$native/.." && pwd)"
licenses="$app/Contents/Resources/Licenses"
mkdir -p "$licenses"
cp "$root/LICENSE" "$root/NOTICE" "$root/THIRD-PARTY-LICENSES.md" "$licenses/"
cp "$native/packaging/licenses/OFL-JetBrainsMono.txt" \
  "$native/packaging/licenses/LICENSE-lucide.txt" \
  "$native/packaging/licenses/NOTICE-voltrus-tools.md" "$licenses/"
sed "s/@VERSION@/$version/g" "$native/packaging/Info.plist" > "$app/Contents/Info.plist"

# The icon: AppKit renders the SVG to 1024 px, sips scales the iconset.
work="$(mktemp -d)"
swift "$native/scripts/rasterize-icon.swift" "$native/packaging/AppIcon.svg" "$work/AppIcon.png" 1024
iconset="$work/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$work/AppIcon.png" --out "$iconset/icon_${size}x${size}.png" > /dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$work/AppIcon.png" --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
# Ad-hoc signature: required to launch on Apple silicon. It is not a
# Developer ID signature, so Gatekeeper asks before the first launch.
codesign --force --deep --sign - "$app"

(cd "$out" && ditto -c -k --keepParent "Voltrus SCADA.app" "$zip")
echo "Bundled $app ($version) as $out/$zip"
