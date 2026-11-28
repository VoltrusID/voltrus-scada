#!/usr/bin/env bash
# The OPC UA benchmark matrix of docs/footprint.md: one bench.py run per line, results appended to <results.jsonl>.
#   matrix.sh <dir with voltrus and opcua-sim> <label> <results.jsonl> [extra bench.py args...]
# MATRIX_TAGS (default "100 1000 10000"), MATRIX_RATES ("1000 100"), MATRIX_PCTS ("10 100") and MATRIX_DEVICES10
# (tag counts also run as 10 devices, default "10000") pick the cells. WARMUP/WINDOW in seconds (20/60).
set -euo pipefail
bin=$1 label=$2 out=$3; shift 3
here=$(cd "$(dirname "$0")" && pwd)
for n in ${MATRIX_TAGS:-100 1000 10000}; do
  devs="1"; [[ " ${MATRIX_DEVICES10:-10000} " == *" $n "* ]] && devs="1 10"
  for d in $devs; do
    for m in ${MATRIX_RATES:-1000 100}; do
      for p in ${MATRIX_PCTS:-10 100}; do
        python3 -I "$here/bench.py" --bin "$bin" --tags "$n" --rate-ms "$m" --change-pct "$p" --devices "$d" \
          --warmup "${WARMUP:-20}" --window "${WINDOW:-60}" --label "$label" --out "$out" "$@" \
          || echo "{\"label\": \"$label\", \"tags\": $n, \"rate_ms\": $m, \"change_pct\": $p, \"devices\": $d, \"error\": \"bench.py failed\"}" >> "$out"
      done
    done
  done
done
