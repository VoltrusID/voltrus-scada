#!/usr/bin/env bash
# Wrap the release binary in an app bundle and zip it.
# Usage: [VERSION=x.y.z] [SERVER=<voltrus binary>] native/scripts/bundle-macos.sh <binary> <output dir> [zip name]
# VERSION overrides the bundle version (default: native/Cargo.toml's version).
# SERVER bundles the server as Contents/MacOS/voltrus: the console then offers
# to set this Mac up as a local station (`voltrus station init`, `voltrus
# service install` as a LaunchAgent).
set -euo pipefail

binary="$1"
out="$2"
zip="${3:-Voltrus-SCADA-Console-macos-arm64.zip}"
native="$(cd "$(dirname "$0")/.." && pwd)"
version="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$native/Cargo.toml" | head -1)}"
app="$out/Voltrus SCADA.app"
root="$(cd "$native/.." && pwd)"
notices="$native/packaging/licenses"

# The licence notices must describe the vt-ui revision that is built.
"$native/scripts/check-notices.sh"
if [ -n "${SERVER:-}" ] && [ ! -f "$SERVER" ]; then
  echo "error: SERVER=$SERVER is not a file" >&2
  exit 1
fi

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/voltrus-native"
if [ -n "${SERVER:-}" ]; then
  cp "$SERVER" "$app/Contents/MacOS/voltrus"
fi
# Licence texts for everything the bundle ships (also inside the zip).
licenses="$app/Contents/Resources/Licenses"
mkdir -p "$licenses"
cp "$root/LICENSE" "$root/NOTICE" "$root/THIRD-PARTY-LICENSES.md" "$licenses/"
cp "$notices/OFL-JetBrainsMono.txt" \
  "$notices/LICENSE-lucide.txt" \
  "$notices/NOTICE-voltrus-tools.md" \
  "$notices/gpui-LICENSE-APACHE.txt" \
  "$notices/iTerm2-Color-Schemes-LICENSE.txt" \
  "$notices/muxy-LICENSE.txt" "$licenses/"
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
# Ad-hoc signatures: required to launch on Apple silicon. They are not
# Developer ID signatures, so Gatekeeper asks before the first launch. The
# server, which launchd runs on its own, is signed first under its own
# identifier, then the bundle around it.
if [ -n "${SERVER:-}" ]; then
  codesign --force --sign - --identifier id.voltrus.scada.server "$app/Contents/MacOS/voltrus"
fi
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

(cd "$out" && ditto -c -k --keepParent "Voltrus SCADA.app" "$zip")
echo "Bundled $app ($version) as $out/$zip"
