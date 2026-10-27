#!/usr/bin/env bash
# Run the console's stress test in a real window: the synthetic 10k-tag
# plant (no server), pages switching every 2 s, for STRESS_SECONDS
# (default 20). The console prints its report and quits; this prints it
# again in the log and fails if no report came.
#
# STRESS_PROFILE=1 runs the animation showcase instead (--stress-profile:
# the Overview stays put, tiles flash in ripples, a trend moves every
# frame). The figures are checked against the frame budget: a miss is a
# warning, or an error with STRESS_ENFORCE=1.
# Usage: native/scripts/stress-macos.sh <app bundle> <report file>
set -euo pipefail

app="$1"
report="$2"
seconds="${STRESS_SECONDS:-20}"
binary="$app/Contents/MacOS/voltrus-native"
rm -f "$report"

flags=(--stress --stress-seconds "$seconds" --stress-report "$report" --appearance dark)
title="Native console stress report"
if [ "${STRESS_PROFILE:-0}" = "1" ]; then
  flags+=(--stress-profile)
  title="Native console stress profile (animation showcase)"
fi

"$binary" "${flags[@]}" &
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
echo "── ${title} ──"
cat "$report"
{
  echo "### ${title}"
  echo '```'
  cat "$report"
  echo '```'
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"

# The budget: under 1% of vsyncs dropped, at least 97% of back-to-back
# frames one vsync after the previous (frame_pacing), no frame interval
# over STRESS_MAX_INTERVAL_MS (default 50 ms, three refreshes at 60 Hz), a
# page's first frame within 34 ms of the switch; in the profile, 95% of
# frames built within a 120 Hz frame. The interval p95 is printed as
# information only: on the CI runner it measures when the display link
# wakes the main thread (see late_starts), not the console; the same wake
# jitter sets the single longest interval there, so CI raises its limit.
level=warning
if [ "${STRESS_ENFORCE:-0}" = "1" ]; then level=error; fi
misses=$(awk -v max_interval="${STRESS_MAX_INTERVAL_MS:-50}" '
  /^dropped_frames:/ {
    match($0, /\(([0-9.]+)%/); pct = substr($0, RSTART + 1, RLENGTH - 2)
    if (pct + 0 >= 1.0) print "dropped " pct "% of vsyncs (budget < 1%)"
  }
  /^frame_interval_ms:/ {
    if ($5 + 0 > 17.5) info = "frame interval p95 " $5 " ms (information only: runner wake jitter, see late_starts)"
    if ($7 + 0 >= max_interval + 0) print "frame interval max " $7 " ms (budget < " max_interval " ms)"
  }
  /^mode: profile/ { profile = 1 }
  /^frame_pacing:/ {
    pace = $2; sub(/%/, "", pace)
    if (pace + 0 < 97) print "frame pacing " pace "% on the next vsync (budget >= 97%)"
  }
  END { if (info != "") print "INFO " info }
  /^page_switches:/ {
    for (i = 1; i <= NF; i++) if ($i == "p95") p95 = $(i + 1)
    if (p95 + 0 >= 34) print "page switch p95 " p95 " ms (budget < 34 ms)"
  }
  /^frame_budget:/ {
    match($0, /, [0-9.]+% within 8\.3/); share = substr($0, RSTART + 2, RLENGTH - 14)
    if (profile && share + 0 < 95) print "profile frames built within 8.3 ms " share "% (budget >= 95%)"
  }
' "$report")
infos=$(grep '^INFO ' <<< "$misses" | sed 's/^INFO //' || true)
misses=$(grep -v '^INFO ' <<< "$misses" || true)
if [ -n "$infos" ]; then
  while IFS= read -r info; do echo "::notice::${info}"; done <<< "$infos"
fi
if [ -n "$misses" ]; then
  while IFS= read -r miss; do echo "::${level}::stress budget missed: ${miss}"; done <<< "$misses"
  if [ "$level" = "error" ]; then exit 1; fi
else
  echo "stress budget met"
fi
