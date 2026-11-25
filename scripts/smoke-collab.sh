#!/usr/bin/env bash
# Browser smoke run of live collaboration in the Designer (client/e2e/collab.smoke.mjs)
# against a server binary with the WTP showcase: starts plant-sim.py and the
# server in a temporary directory, seeds the screens, installs playwright-core
# there and runs the smoke headless with the system Chrome or Chromium.
#
# Usage: scripts/smoke-collab.sh target/release/voltrus [screenshot-dir]
#   CHROMIUM=/path/to/chrome   browser to drive (default: google-chrome, chromium)
#   PORT, PLC_PORT             server and plant-sim ports (default 3998, 5930)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$(cd "$(dirname "${1:?usage: smoke-collab.sh path/to/voltrus [shots]}")" && pwd)/$(basename "$1")"
shots="${2:-}"
port="${PORT:-3997}"
plc="${PLC_PORT:-5940}"
dir="$(mktemp -d)"
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  rm -rf "$dir"
}
trap cleanup EXIT

browser="${CHROMIUM:-$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)}"
[ -n "$browser" ] || { echo "no Chrome or Chromium found; set CHROMIUM"; exit 1; }

scenario="$root/scenarios/wtp-hmi"
sed -e "s/^server_port: .*/server_port: $port/" -e "s/port: 5030/port: $plc/" "$scenario/voltrus.yaml" > "$dir/voltrus.yaml"

cd "$dir"
# A build without the embedded UI (debug) serves ./static; a release binary embeds it.
[ -f "$root/static/index.html" ] && ln -s "$root/static" static
python3 "$scenario/plant-sim.py" --port "$plc" --s7-filler $((plc + 1)) --s7-packer $((plc + 2)) --enip $((plc + 3)) > sim.log 2>&1 &
pids+=($!)
"$bin" voltrus.yaml > server.log 2>&1 &
pids+=($!)
base="http://127.0.0.1:$port"

for _ in $(seq 1 300); do
  curl -fsS "$base/api/v1/health" > /dev/null 2>&1 && break
  kill -0 "${pids[1]}" 2>/dev/null || { tail -40 server.log; echo "server exited"; exit 1; }
  sleep 0.1
done
[ -f initial-admin-password ] || { tail -40 server.log; echo "no initial-admin-password"; exit 1; }

# Names the tags and builds the screens; prints the admin password it sets.
python3 "$scenario/seed.py" --url "$base" --password "$(cat initial-admin-password)" \
  --plc-port "$plc" --s7-filler $((plc + 1)) --s7-packer $((plc + 2)) --enip $((plc + 3)) > seed.log 2>&1 \
  || { tail -40 seed.log; tail -40 server.log; exit 1; }
password="$(grep -oP 'admin password changed to \K\S+' seed.log)"
[ -n "$password" ] || { echo "seed.py printed no admin password"; tail -20 seed.log; exit 1; }

npm install --prefix "$dir/pw" --no-save --no-audit --no-fund --silent playwright-core@1 > /dev/null

cd "$root/client"
VOLTRUS_URL="$base" VOLTRUS_PASSWORD="$password" CHROMIUM="$browser" \
  PLAYWRIGHT_CORE="$dir/pw/node_modules/playwright-core/index.js" \
  node e2e/collab.smoke.mjs ${shots:+"$shots"} \
  || { echo "--- server.log (tail)"; tail -40 "$dir/server.log"; exit 1; }
