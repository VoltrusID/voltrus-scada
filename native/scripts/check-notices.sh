#!/usr/bin/env bash
# Check that the licence notices the console ships describe the vt-ui revision
# that is built: the one Cargo.lock resolves, not whatever the notices were
# last copied from. Run by bundle-macos.sh and before the Windows installer.
# Usage: native/scripts/check-notices.sh
set -euo pipefail

native="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$native/.." && pwd)"
notices="$native/packaging/licenses"

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
