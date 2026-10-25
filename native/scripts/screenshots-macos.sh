#!/usr/bin/env bash
# Capture the console's window on each page, signed in to a live server,
# with the palette, the shortcut help and the HUD open, and in stress mode.
# Usage: native/scripts/screenshots-macos.sh <app bundle> <output dir>
# Needs VOLTRUS_NATIVE_URL, VOLTRUS_NATIVE_USER and VOLTRUS_NATIVE_PASSWORD.
set -euo pipefail

app="$1"
out="$2"
here="$(cd "$(dirname "$0")" && pwd)"
binary="$app/Contents/MacOS/voltrus-native"
mkdir -p "$out"

helper="$(mktemp -d)/window-id"
swiftc -O -o "$helper" "$here/window-id.swift"

# name, then the console's arguments.
shots=(
  "overview|--sign-in --page overview --appearance dark"
  "tags|--sign-in --page tags --tag ac_current_l1 --appearance dark"
  "alarms|--sign-in --page alarms --appearance dark"
  "trends|--sign-in --page trends --range 5m --appearance dark"
  "overview-light|--sign-in --page overview --appearance light"
  "sign-in|--appearance dark"
  "palette|--sign-in --page overview --overlay palette --appearance dark"
  "help|--sign-in --page tags --overlay help --appearance dark"
  "trends-hud|--sign-in --page trends --range 5m --hud --appearance dark"
  "stress-performance|--stress --stress-switch-ms 0 --page performance --appearance dark"
  "stress-overview|--stress --stress-switch-ms 0 --page overview --hud --appearance dark"
  "shell-settings|--sign-in --page overview --overlay ShellOpenSettings --appearance dark"
  "shell-settings-light|--sign-in --page alarms --overlay ShellOpenSettings --appearance light"
  "shell-notifications|--sign-in --page overview --overlay ShellToggleNotifications --appearance dark"
  "shell-shortcuts|--sign-in --page tags --overlay ShellEditShortcuts --appearance dark"
  "shell-themes|--sign-in --page trends --range 5m --overlay ShellPickTheme --appearance dark"
  "tagtree|--sign-in --page tagtree --appearance dark"
  "tagtree-light|--sign-in --page tagtree --appearance light"
  "tag-detail-wide|--sign-in --page tags --tag ac_power --overlay tag-expand --appearance dark"
  "tag-write|--sign-in --page tags --tag ac_current_l1 --overlay tag-write --appearance dark"
  "tag-policy|--sign-in --page tags --tag ac_current_l1 --overlay tag-policy --appearance dark"
  "tag-note|--sign-in --page tags --tag ac_power --overlay tag-note --appearance dark"
  "tag-compare|--sign-in --page tags --tag ac_power --overlay tag-compare --appearance dark"
  "tag-range|--sign-in --page tags --tag ac_power --overlay tag-range --appearance dark"
  "ops-commands|--sign-in --page commands --appearance dark"
  "ops-shift|--sign-in --page shift --appearance dark"
  "ops-shift-new|--sign-in --page shift --overlay OpsNewHandover --appearance dark"
  "ops-audit|--sign-in --page audit --appearance dark"
  "ops-audit-light|--sign-in --page audit --appearance light"
  "ops-export|--sign-in --page overview --overlay OpsExportData --appearance dark"
  "alarms-history|--sign-in --alarms history --appearance dark"
  "alarms-analytics|--sign-in --alarms analytics --appearance dark"
  "alarms-audit|--sign-in --alarms audit --appearance dark"
  "alarms-detail|--sign-in --alarms detail --appearance dark"
  "alarms-shelve|--sign-in --alarms shelve --appearance dark"
  "alarms-confirm|--sign-in --alarms confirm --appearance dark"
  "alarms-light|--sign-in --page alarms --appearance light"
  "dashboard|--sign-in --page dashboard --appearance dark"
  "dashboard-screen|--sign-in --page dashboard --screen 2 --appearance dark"
  "dashboard-edit|--sign-in --page dashboard --screen 2 --edit --appearance dark"
  "dashboard-add-tags|--sign-in --page dashboard --screen 2 --edit --overlay add-tags --appearance dark"
  "dashboard-light|--sign-in --page dashboard --screen 2 --appearance light"
  "stress-dashboard|--stress --stress-switch-ms 0 --page dashboard --hud --appearance dark"
)

status=0
for shot in "${shots[@]}"; do
  name="${shot%%|*}"
  read -r -a args <<< "${shot#*|}"
  "$binary" "${args[@]}" &
  pid=$!
  id=""
  for _ in $(seq 1 60); do
    if id="$("$helper" "$pid" 2>/dev/null)"; then break; fi
    sleep 0.5
  done
  if [ -z "$id" ]; then
    echo "::warning::$name: no window appeared"
    status=1
  else
    # Sign in, load history, and let live values and alarms arrive.
    sleep 10
    screencapture -o -x -l "$id" "$out/$name.png"
    echo "captured $name ($(sips -g pixelWidth -g pixelHeight "$out/$name.png" | tail -2 | awk '{print $2}' | paste -sd x -))"
  fi
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
done
exit "$status"
