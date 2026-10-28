#!/usr/bin/env bash
# Checks that a release binary carries the web UI: starts it in an empty directory
# (no static/ folder) and requests the page, a hashed asset, a client-side route
# and a missing file; then that a ./static/index.html on disk takes over.
#
# Usage: scripts/smoke-embedded-ui.sh target/release/voltrus
set -euo pipefail

bin="$(cd "$(dirname "${1:?usage: smoke-embedded-ui.sh path/to/voltrus}")" && pwd)/$(basename "$1")"
port="${PORT:-3999}"
dir="$(mktemp -d)"
pid=""
cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; rm -rf "$dir"; }
trap cleanup EXIT

cat > "$dir/config.yaml" <<EOF
server_host: 127.0.0.1
server_port: $port
db_path: data.db
EOF

cd "$dir"
"$bin" config.yaml > server.log 2>&1 &
pid=$!
base="http://127.0.0.1:$port"

for _ in $(seq 1 100); do
  curl -fsS "$base/api/v1/health" > /dev/null 2>&1 && break
  kill -0 "$pid" 2>/dev/null || { cat server.log; echo "server exited"; exit 1; }
  sleep 0.1
done

fail() { echo "FAIL: $*"; cat server.log | tail -20; exit 1; }
header() { grep -i "^$1:" "$2" | tr -d '\r' | sed "s/^[^:]*: *//"; }

# The page.
curl -fsS -D h.txt -o index.html "$base/"
header content-type h.txt | grep -q '^text/html' || fail "/ is not text/html"
grep -q '<div id="root">' index.html || fail "/ is not the web UI"

# A hashed asset: gzip as stored for browsers that accept it, plain for those that do not.
asset="$(grep -o '/assets/[^"]*\.js' index.html | head -1)"
[ -n "$asset" ] || fail "index.html references no script"
curl -fsS -D h.txt -o asset.gz -H 'Accept-Encoding: gzip' "$base$asset"
[ "$(header content-encoding h.txt)" = "gzip" ] || fail "$asset was not sent gzipped"
header cache-control h.txt | grep -q immutable || fail "$asset is not cached as immutable"
etag="$(header etag h.txt)"
curl -fsS -D h.txt -o asset.js -H 'Accept-Encoding: identity' "$base$asset"
[ -z "$(header content-encoding h.txt)" ] || fail "$asset was gzipped for an identity client"
[ "$(wc -c < asset.js)" = "$(header content-length h.txt)" ] || fail "$asset length mismatch"
header content-type h.txt | grep -qi 'javascript' || fail "$asset is not served as JavaScript"
gunzip -c asset.gz | cmp - asset.js || fail "the gzip and identity bodies differ"
[ "$(curl -s -o /dev/null -w '%{http_code}' -H 'Accept-Encoding: gzip' -H "If-None-Match: $etag" "$base$asset")" = "304" ] || fail "no 304 for a matching ETag"

# A client-side route gets the page; a missing file is a 404.
curl -fsS "$base/alarms" | grep -q '<div id="root">' || fail "/alarms is not the web UI"
[ "$(curl -s -o /dev/null -w '%{http_code}' "$base/assets/missing-0123.js")" = "404" ] || fail "missing asset is not 404"

# A ./static/index.html on disk overrides the embedded UI.
mkdir static && echo 'override' > static/index.html
[ "$(curl -fsS "$base/")" = "override" ] || fail "static/index.html did not override the embedded UI"

echo "embedded UI OK ($asset)"
