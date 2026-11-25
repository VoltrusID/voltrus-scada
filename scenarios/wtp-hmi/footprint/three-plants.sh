#!/usr/bin/env bash
# Footprint test: three Voltrus SCADA projects side by side on one laptop (measured 2026-11-25 with v0.52.0).
# Needs Linux x86_64, systemd --user, python3 and curl. Method and results: docs/footprint.md
# Each plant = its own plant-sim.py (Modbus, :5031-5033), its own voltrus.yaml + SQLite DB, its own port (:3101-3103),
# all from ONE copy of the binary. Each server runs in a systemd --user scope so scenarios/wtp-hmi/footprint/cstat.sh can read its cgroup.
#   scenarios/wtp-hmi/footprint/three-plants.sh start <workdir>     # downloads the release once, seeds the WTP screens
#   scenarios/wtp-hmi/footprint/cstat.sh 60 scmp-voltrus-plant{1,2,3}.scope    # CPU/memory;  scenarios/wtp-hmi/footprint/procstat.sh 5 <pids>  # RSS/PSS
#   scenarios/wtp-hmi/footprint/three-plants.sh stop <workdir>
set -euo pipefail
cmd=$1; W=$(realpath -m "$2"); V=${VOLTRUS_VERSION:-v0.52.0}
S=${VOLTRUS_SCENARIO:-$(cd "$(dirname "$0")/.." && pwd)}
BIN=$W/voltrus-core-$V-linux-x86_64/voltrus
if [[ $cmd == stop ]]; then
  for i in 1 2 3; do systemctl --user stop scmp-voltrus-plant$i.scope 2>/dev/null || true; kill "$(cat "$W/plant$i/sim.pid")" 2>/dev/null || true; done
  exit 0
fi
mkdir -p "$W"
if [[ ! -x $BIN ]]; then
  u=https://github.com/VoltrusID/voltrus-scada/releases/download/$V/voltrus-core-$V-linux-x86_64.tar.gz
  (cd "$W" && curl -fsSLO "$u" && curl -fsSLO "$u.sha256" && sha256sum -c ./*.sha256 \
     && tar xzf voltrus-core-$V-linux-x86_64.tar.gz && rm voltrus-core-$V-linux-x86_64.tar.gz*)
fi
for i in 1 2 3; do
  d=$W/plant$i; mkdir -p "$d"
  sed -e "s/^server_port: 3100/server_port: 310$i\nserver_host: \"127.0.0.1\"/" -e "s/port: 5030/port: 503$i/" \
      -e "s/wtp-hmi.db/plant$i.db/" "$S/voltrus.yaml" > "$d/voltrus.yaml"
  (cd "$d" && setsid nohup python3 -I "$S/plant-sim.py" --port 503$i > sim.log 2>&1 & echo $! > "$d/sim.pid")
done
sleep 1
for i in 1 2 3; do
  d=$W/plant$i; cd "$d"
  t0=$(date +%s%N)
  setsid nohup systemd-run --user --scope -q -u scmp-voltrus-plant$i "$BIN" voltrus.yaml > server.log 2>&1 &
  until curl -sf -o /dev/null http://127.0.0.1:310$i/; do sleep 0.02; done
  echo "plant$i: start to first HTTP 200 in $(( ($(date +%s%N)-t0)/1000000 )) ms"
  if [[ -f initial-admin-password ]]; then
    python3 -I "$S/seed.py" --url http://127.0.0.1:310$i --password "$(cat initial-admin-password)" --new-password "scmp-lab-plant$i" | tail -1
  fi
done
