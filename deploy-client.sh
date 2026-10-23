#!/usr/bin/env bash
# Build client and deploy to static/ for the Rust server to serve
set -e
cd "$(dirname "$0")"

echo "🏗  Building client..."
cd client
bun run build

echo "📦 Deploying to static/..."
# Sync dist → static (delete old hashed files, add new ones)
rsync -a --delete dist/ ../static/

echo "✅ Done. Restart the server to pick up changes."
echo "   Tip: pkill voltrus; ./target/release/voltrus config.yaml"
