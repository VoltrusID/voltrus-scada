#!/bin/bash
# Publishes the voltrus package to npm.
# Guards: npm auth, typecheck, build, explicit confirmation with a file preview.
set -euo pipefail
cd "$(dirname "$0")"

if ! npm whoami >/dev/null 2>&1; then
  echo "Not logged in to npm. Run 'npm login' first." >&2
  exit 1
fi

npm run typecheck
npm run build

name="$(node -p "require('./package.json').name")"
version="$(node -p "require('./package.json').version")"
echo
echo "Ready to publish ${name}@${version}. Files:"
npm pack --dry-run

echo
read -r -p "Publish ${name}@${version} to npm? [y/N] " reply
case "$reply" in
  y | Y) ;;
  *) echo "Aborted." ;;
esac

if [[ "$reply" == [yY] ]]; then
  npm publish
  echo "Published ${name}@${version}."
fi
