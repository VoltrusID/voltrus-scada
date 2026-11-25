#!/usr/bin/env bash
# Measure RSS, PSS and average CPU of host processes over a window.
#   procstat.sh <seconds> <pid> [pid...]
# RSS/PSS from /proc/<pid>/status and smaps_rollup at the end of the window; CPU = (utime+stime) delta / wall time,
# in % of one core. Prints one line per pid and a total.
set -euo pipefail
win=$1; shift
hz=$(getconf CLK_TCK)
declare -A t0
for p in "$@"; do t0[$p]=$(awk '{print $14+$15}' /proc/$p/stat); done
sleep "$win"
tr=0; tp=0; tc=0
printf '%-8s %-22s %10s %10s %8s\n' pid name rss_MiB pss_MiB cpu_%
for p in "$@"; do
  t1=$(awk '{print $14+$15}' /proc/$p/stat)
  rss=$(awk '/^VmRSS/{print $2}' /proc/$p/status)
  pss=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup)
  cpu=$(awk -v a="${t0[$p]}" -v b="$t1" -v hz="$hz" -v w="$win" 'BEGIN{printf "%.2f", (b-a)/hz/w*100}')
  name=$(tr '\0' ' ' < /proc/$p/cmdline | cut -c1-22)
  printf '%-8s %-22s %10.1f %10.1f %8s\n' "$p" "$name" "$(awk -v k=$rss 'BEGIN{print k/1024}')" "$(awk -v k=$pss 'BEGIN{print k/1024}')" "$cpu"
  tr=$((tr+rss)); tp=$((tp+pss)); tc=$(awk -v a=$tc -v b=$cpu 'BEGIN{print a+b}')
done
printf '%-31s %10.1f %10.1f %8s\n' TOTAL "$(awk -v k=$tr 'BEGIN{print k/1024}')" "$(awk -v k=$tp 'BEGIN{print k/1024}')" "$tc"
