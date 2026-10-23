#!/usr/bin/env bash
# Installs version-controlled git hooks from scripts/hooks/ into .git/hooks/.
# Idempotent: skips hooks that are already identical, backs up any that differ.
#
# Usage:
#   scripts/install-git-hooks.sh          # install all hooks
#   scripts/install-git-hooks.sh pre-push # install one hook
set -euo pipefail

if [ -z "${GIT_DIR:-}" ] && [ -d .git ]; then :; else
  echo "Run from the repo root." >&2; exit 1
fi

REPO_ROOT=$(git rev-parse --show-toplevel)
SRC="$REPO_ROOT/scripts/hooks"
DEST=$(git rev-parse --git-path hooks)   # respects core.hooksPath
mkdir -p "$DEST"

targets=("$@")
[ ${#targets[@]} -eq 0 ] && targets=("$SRC"/*)

installed=0
for src_hook in "${targets[@]}"; do
  # allow passing a bare name like "pre-push"
  [ -e "$src_hook" ] || src_hook="$SRC/$(basename "$src_hook")"
  name=$(basename "$src_hook")
  [ -f "$src_hook" ] || { echo "skip $name (not found)"; continue; }

  dst_hook="$DEST/$name"
  if [ -e "$dst_hook" ] && ! [ -L "$dst_hook" ]; then
    if cmp -s "$src_hook" "$dst_hook"; then
      echo "ok    $name (unchanged)"; installed=$((installed+1)); continue
    fi
    backup="$dst_hook.bak.$(date +%s)"
    cp "$dst_hook" "$backup"
    echo "back  $name → $(basename "$backup")"
  fi
  cp "$src_hook" "$dst_hook"
  chmod +x "$dst_hook"
  echo "inst  $name → $dst_hook"
  installed=$((installed+1))
done

echo "Done. $installed hook(s) installed in $DEST"
echo "Tip: git config core.hooksPath scripts/hooks  # run hooks straight from the repo"
