#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
python -m build
twine upload dist/*
