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
root="$(cd "$native/.." && pwd)"
notices="$native/packaging/licenses"

# The licence notices must describe the vt-ui revision that is built: the
# one Cargo.lock resolves, not whatever the notices were last copied from.
vt_ui_rev="$(awk '
  /^\[\[package\]\]/ { in_vt_ui = 0 }
  /^name = "vt-ui"$/ { in_vt_ui = 1 }
  in_vt_ui && /^source = "git\+https:\/\/github.com\/VoltrusID\/voltrus-explorers\?rev=/ { print; exit }
' "$root/Cargo.lock" | sed -n 's/.*?rev=\([0-9a-f]\{40\}\)#.*/\1/p')"
if [ -z "$vt_ui_rev" ]; then
  echo "error: no vt-ui git source (voltrus-explorers?rev=...) in $root/Cargo.lock" >&2
  exit 1
fi
readme_rev="$(sed -n 's/.*voltrus-explorers at rev \([0-9a-f]\{40\}\).*/\1/p' "$notices/README")"
notice_rev="$(sed -n 's#.*voltrus-explorers/tree/\([0-9a-f]\{40\}\)/.*#\1#p' "$root/NOTICE")"
for named in "native/packaging/licenses/README:$readme_rev" "NOTICE source offer:$notice_rev"; do
  if [ "${named##*:}" != "$vt_ui_rev" ]; then
    echo "error: ${named%:*} names vt-ui rev '${named##*:}', but Cargo.lock builds $vt_ui_rev." >&2
    echo "       Re-copy the notices from voltrus-explorers at $vt_ui_rev (see native/packaging/licenses/README) and update both revs." >&2
    exit 1
  fi
done

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/voltrus-native"
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
# Ad-hoc signature: required to launch on Apple silicon. It is not a
# Developer ID signature, so Gatekeeper asks before the first launch.
codesign --force --deep --sign - "$app"

(cd "$out" && ditto -c -k --keepParent "Voltrus SCADA.app" "$zip")
echo "Bundled $app ($version) as $out/$zip"
