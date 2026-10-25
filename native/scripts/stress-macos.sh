#!/usr/bin/env bash
# Run the console's stress test in a real window: the synthetic 10k-tag
# plant (no server), pages switching every 2 s, for STRESS_SECONDS
# (default 20). The console prints its report and quits; this prints it
# again in the log and fails if no report came.
# Usage: native/scripts/stress-macos.sh <app bundle> <report file>
set -euo pipefail

app="$1"
report="$2"
seconds="${STRESS_SECONDS:-20}"
binary="$app/Contents/MacOS/voltrus-native"
rm -f "$report"

"$binary" --stress --stress-seconds "$seconds" --stress-report "$report" \
  --appearance dark &
pid=$!
# The run, plus time to open the window and quit.
limit=$((seconds + 60))
for _ in $(seq 1 "$limit"); do
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  sleep 1
done
if kill -0 "$pid" 2>/dev/null; then
  echo "::error::the stress run did not end within ${limit} s"
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  exit 1
fi
wait "$pid" || true

if [ ! -s "$report" ]; then
  echo "::error::no stress report at $report"
  exit 1
fi
echo "── stress report ──"
cat "$report"
{
  echo "### Native console stress report"
  echo '```'
  cat "$report"
  echo '```'
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
