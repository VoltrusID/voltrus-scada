#!/bin/bash
# Dumps the OpenAPI spec of this Core build and generates reference clients
# from it into sdks/generated/ (git-ignored).
#
# The published SDKs in sdks/python and sdks/typescript are hand-written. They
# are NOT overwritten by this script. Compare sdks/generated/ and
# sdks/openapi.json against them when the REST API changes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SPEC_FILE="$SCRIPT_DIR/openapi.json"
OUT_DIR="$SCRIPT_DIR/generated"
VOLTRUS_BIN="${VOLTRUS_BIN:-cargo run --bin voltrus -- --dump-openapi}"

echo "Generating OpenAPI spec..."
$VOLTRUS_BIN > "$SPEC_FILE"
echo "Spec written to $SPEC_FILE ($(wc -c < "$SPEC_FILE") bytes)"

mkdir -p "$OUT_DIR"

echo ""
echo "Generating reference Python client..."
docker run --rm -v "$SCRIPT_DIR:/sdks" \
    openapitools/openapi-generator-cli:v7.12.0 \
    generate \
    -i /sdks/openapi.json \
    -g python \
    -o /sdks/generated/python \
    --package-name voltrus \
    --additional-properties=generateSourceCodeOnly=true,library=urllib3

echo ""
echo "Generating reference TypeScript client..."
docker run --rm -v "$SCRIPT_DIR:/sdks" \
    openapitools/openapi-generator-cli:v7.12.0 \
    generate \
    -i /sdks/openapi.json \
    -g typescript-axios \
    -o /sdks/generated/typescript \
    --additional-properties=supportsES6=true,npmName=voltrus,withInterfaces=true

echo ""
echo "Done. Reference output is in sdks/generated/ and the spec in sdks/openapi.json."
