#!/usr/bin/env bash
# Measure Docker containers from their cgroup (v2): average CPU over a window and memory at the end.
#   cstat.sh <seconds> <container|user-scope.scope> [...]   (a name ending in .scope = a systemd --user scope)
# mem = memory.current - inactive_file (what `docker stats` shows); anon = anonymous memory only.
# CPU in % of one core = usage_usec delta / window.
set -euo pipefail
win=$1; shift
cg() {
  if [[ $1 == *.scope ]]; then echo /sys/fs/cgroup$(systemctl --user show -p ControlGroup --value "$1")
  else echo /sys/fs/cgroup/system.slice/docker-$(docker inspect -f '{{.Id}}' "$1").scope; fi
}
declare -A u0
for c in "$@"; do u0[$c]=$(awk '/^usage_usec/{print $2}' "$(cg "$c")/cpu.stat"); done
sleep "$win"
printf '%-18s %10s %10s %8s %6s\n' container mem_MiB anon_MiB cpu_% procs
for c in "$@"; do
  d=$(cg "$c")
  u1=$(awk '/^usage_usec/{print $2}' "$d/cpu.stat")
  cur=$(cat "$d/memory.current")
  inact=$(awk '/^inactive_file /{print $2}' "$d/memory.stat")
  anon=$(awk '/^anon /{print $2}' "$d/memory.stat")
  n=$(wc -l < "$d/cgroup.procs")
  awk -v c="$c" -v cur="$cur" -v ina="$inact" -v an="$anon" -v a="${u0[$c]}" -v b="$u1" -v w="$win" -v n="$n" \
    'BEGIN{printf "%-18s %10.1f %10.1f %8.2f %6d\n", c, (cur-ina)/1048576, an/1048576, (b-a)/1e6/w*100, n}'
done
