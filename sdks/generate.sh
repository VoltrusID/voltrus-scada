#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SPEC_FILE="$SCRIPT_DIR/openapi.json"
VOLTRUS_BIN="${VOLTRUS_BIN:-cargo run --bin voltrus -- --dump-openapi}"

echo "Generating OpenAPI spec..."
$VOLTRUS_BIN > "$SPEC_FILE"
echo "Spec written to $SPEC_FILE ($(wc -c < "$SPEC_FILE") bytes)"

# Generate Python SDK
echo ""
echo "Generating Python SDK..."
docker run --rm -v "$SCRIPT_DIR:/sdks" \
    openapitools/openapi-generator-cli:v7.12.0 \
    generate \
    -i /sdks/openapi.json \
    -g python \
    -o /sdks/python \
    --package-name voltrus \
    --additional-properties=generateSourceCodeOnly=true,library=urllib3

echo "Python SDK generated in sdks/python/"

# Generate TypeScript SDK
echo ""
echo "Generating TypeScript SDK..."
docker run --rm -v "$SCRIPT_DIR:/sdks" \
    openapitools/openapi-generator-cli:v7.12.0 \
    generate \
    -i /sdks/openapi.json \
    -g typescript-axios \
    -o /sdks/typescript \
    --additional-properties=supportsES6=true,npmName=voltrus,withInterfaces=true

echo "TypeScript SDK generated in sdks/typescript/"
echo ""
echo "Done! Run 'cd sdks/python && python -m pip install -e .' to install Python SDK."
echo "Run 'cd sdks/typescript && npm install' to install TypeScript SDK."
