#!/usr/bin/env bash
# Run the solar-farm plant for the console's end-to-end test and screenshots,
# or as a demo plant on a Mac: a Voltrus server on http://127.0.0.1:3000 plus
# the scenario's two Modbus simulators (:5020, :5021), an admin whose
# generated password is replaced through the API, and plant-like alarm limits.
#
# Usage: native/scripts/ci-server.sh <work dir>
# Needs the release binaries `voltrus` and `modbus-sim` in target/release
# (relative to the checkout or the unpacked demo-plant artifact), curl, jq and
# openssl. VOLTRUS_PLANT_PASSWORD picks the admin's new password (at least 8
# characters; default: random). In CI ($GITHUB_ENV set) the password is masked
# and appended to $GITHUB_ENV with the URL and username; elsewhere it is
# printed. The processes keep running; stop them with
# `kill $(cat <work dir>/*.pid)`.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
bin="$root/target/release"
work="$1"
mkdir -p "$work"
cd "$work"

port=3000
url="http://127.0.0.1:$port"
json='content-type: application/json'

# The scenario's devices, plus what plain-HTTP sign-in needs: cookies without
# the Secure flag, and a fixed signing secret.
{
  cat "$root/scenarios/solar-farm/voltrus.yaml"
  printf '\ncookie_secure: false\nauth_secret: "%s"\n' "$(openssl rand -hex 24)"
  # SMTP settings for Data Manager › System › SMTP: the e2e changes and
  # restores them (nothing listens on :2525, so a test email fails).
  printf 'smtp:\n  host: 127.0.0.1\n  port: 2525\n  username: ci\n  password: ci-smtp-%s\n  from_address: scada@plant.local\n  from_name: Voltrus CI\n  encryption: none\n' "$(openssl rand -hex 4)"
} > voltrus.yaml

"$bin/voltrus" voltrus.yaml > server.log 2>&1 &
echo $! > server.pid
for _ in $(seq 1 120); do
  if curl -fsS "$url/api/v1/health" > /dev/null 2>&1; then break; fi
  sleep 1
done
curl -fsS "$url/api/v1/health" > /dev/null || { echo "::error::the server did not start"; tail -50 server.log; exit 1; }

# A fresh admin password from the CLI; the server wants it changed at once.
generated="$("$bin/voltrus" --reset-admin-password voltrus.yaml | sed -n 's/^New password: //p')"
[ -n "$generated" ] || { echo "::error::no password from --reset-admin-password"; exit 1; }
echo "::add-mask::$generated"

login() {
  curl -fsS -c cookies.txt -H "$json" \
    -d "$(jq -n --arg user admin --arg pass "$1" '{username: $user, password: $pass}')" \
    "$url/api/v1/auth/login"
}
[ "$(login "$generated" | jq -r .must_change_password)" = "true" ] \
  || echo "::warning::the reset password did not ask for a change"

password="${VOLTRUS_PLANT_PASSWORD:-ci-$(openssl rand -hex 12)}"
echo "::add-mask::$password"
curl -fsS -b cookies.txt -H "$json" \
  -d "$(jq -n --arg old "$generated" --arg new "$password" '{old_password: $old, new_password: $new}')" \
  "$url/api/v1/auth/change-password" > /dev/null
[ "$(login "$password" | jq -r .must_change_password)" = "false" ] \
  || { echo "::error::the password change did not clear must_change_password"; exit 1; }

# The seeded limits (80/90 on almost every register) put the whole plant in
# alarm. Set limits like a plant engineer would, so a few tags stand out:
# AC current L1 critical, irradiance and current distortion warning.
limits() {
  case "$1" in
    dc_voltage_a | dc_voltage_b) echo "750 790" ;;
    dc_current_a | dc_current_b) echo "100 108" ;;
    dc_power) echo "61000 61800" ;;
    ac_voltage_l1 | ac_voltage_l2 | ac_voltage_l3) echo "245 250" ;;
    voltage_l1n | voltage_l2n | voltage_l3n) echo "245 250" ;;
    ac_current_l1) echo "80 85" ;;
    ac_current_l2 | ac_current_l3) echo "95 98" ;;
    ac_power) echo "59500 59900" ;;
    frequency) echo "50.15 50.19" ;;
    irradiance) echo "800 1050" ;;
    temperature) echo "70 85" ;;
    current_l1 | current_l2 | current_l3) echo "400 500" ;;
    active_power) echo "150000 180000" ;;
    thd_voltage) echo "5 8" ;;
    thd_current) echo "5 8" ;;
    *) echo "0 0" ;;
  esac
}
curl -fsS -b cookies.txt "$url/api/v1/data-sources" | jq -r '.[] | "\(.id) \(.key)"' |
  while read -r id key; do
    read -r warn crit <<< "$(limits "$key")"
    curl -fsS -b cookies.txt -X PUT -H "$json" \
      -d "{\"warn_val\": $warn, \"crit_val\": $crit}" \
      "$url/api/v1/data-sources/$id" > /dev/null
  done

# Card kinds for the dashboards: gauges and big numbers beside the default
# sparklines, the gauges with a range they can show.
card() {
  case "$1" in
    dc_voltage_a | dc_voltage_b) echo '{"chart_type": "gauge", "min_val": 0, "max_val": 800}' ;;
    ac_power) echo '{"chart_type": "gauge", "min_val": 0, "max_val": 62000}' ;;
    frequency) echo '{"chart_type": "gauge", "min_val": 49.5, "max_val": 50.5}' ;;
    dc_current_b | dc_power | efficiency) echo '{"chart_type": "bignumber"}' ;;
    *) echo "" ;;
  esac
}
curl -fsS -b cookies.txt "$url/api/v1/data-sources" | jq -r '.[] | "\(.id) \(.key)"' |
  while read -r id key; do
    body="$(card "$key")"
    if [ -n "$body" ]; then
      curl -fsS -b cookies.txt -X PUT -H "$json" -d "$body" \
        "$url/api/v1/data-sources/$id" > /dev/null
    fi
  done

# A stopped demo flow and a sample script for the Engineering sections'
# screenshots (the e2e makes and deletes its own).
curl -fsS -b cookies.txt -H "$json" -d "$(jq -n '{id: "demo_boiler_power", name: "Boiler power", enabled: false,
  trigger_config: ({type: "modbus", device_id: "*", metric: "", debounce_ms: 1000, on_change_only: false} | tostring),
  nodes: ([{id: "n1", type: "filter", condition: "payload.metrics.temp > 0"},
           {id: "n2", type: "expression", output_field: "payload.power_kw", expr: "payload.metrics.voltage * payload.metrics.current * 0.001"},
           {id: "n3", type: "alarm_out", severity: "warning", source_key: "flow_alert", message: "Boiler power high"}] | tostring),
  created_at: 0, updated_at: 0}')" "$url/api/v1/flows" > /dev/null
curl -fsS -b cookies.txt -H "$json" -X POST "$url/api/v1/scripts" \
  -d '{"name":"Inverter kW to MW","source":"// Scale an inverter reading\nfunction main(input) {\n  return { mw: input.kw / 1000 };\n}\n","cpu_limit_ms":1000,"memory_limit_bytes":8388608}' > /dev/null
# An HMI mimic for the overview (the web client's SCADA view, the console's
# Mimic page), unless the plant has one: native/scripts/mimic-solar-farm.json
# names its tags "<device>/<key>"; here they become data source ids. The
# analog widgets get ranges they can show.
mimic="$root/native/scripts/mimic-solar-farm.json"
mimic_ranges() {
  case "$1" in
    irradiance) echo "0 1200" ;;
    efficiency) echo "90 100" ;;
    power_factor) echo "0.8 1" ;;
    temperature) echo "0 250" ;;
    dc_current_a | dc_current_b) echo "0 110" ;;
    ac_voltage_l1 | ac_voltage_l2 | ac_voltage_l3) echo "200 260" ;;
    voltage_l1n | voltage_l2n | voltage_l3n) echo "200 260" ;;
    current_l1 | current_l2 | current_l3) echo "0 600" ;;
    ac_current_l1 | ac_current_l2 | ac_current_l3) echo "0 110" ;;
    thd_voltage | thd_current) echo "0 20" ;;
    active_power) echo "0 200000" ;;
    *) echo "" ;;
  esac
}
seeded="$(curl -fsS -b cookies.txt "$url/api/v1/screens/__overview/scada-equipment" | jq '.items | length')"
if [ ! -f "$mimic" ]; then
  echo "::warning::no $mimic: the overview gets no mimic"
elif [ "$seeded" -gt 0 ]; then
  echo "the overview has a mimic already ($seeded widgets)"
else
  sources="$(curl -fsS -b cookies.txt "$url/api/v1/data-sources")"
  echo "$sources" | jq -r '.[] | "\(.id) \(.key)"' |
    while read -r id key; do
      range="$(mimic_ranges "$key")"
      if [ -n "$range" ]; then
        read -r low high <<< "$range"
        curl -fsS -b cookies.txt -X PUT -H "$json" \
          -d "{\"min_val\": $low, \"max_val\": $high}" \
          "$url/api/v1/data-sources/$id" > /dev/null
      fi
    done
  ids="$(echo "$sources" | jq 'map({key: "\(.device_id)/\(.key)", value: .id}) | from_entries')"
  jq --argjson ids "$ids" '
    def tag($name): ($ids[$name] // error("the plant has no tag \($name)"));
    {items: [.equipment | to_entries[] | .key as $z | .value
      | .sensor_bindings = ([.tags[] | tag(.)] | tojson)
      | .rotation = ((.config.rotation // 0) | tonumber)
      | .config = (.config
          | with_entries(if (.value | type) == "string" and (.value | startswith("@"))
                         then .value = tag(.value[1:]) else . end)
          | .z = $z | tojson)
      | del(.tags)]}' "$mimic" > mimic-equipment.json
  jq '{items: [.connections[] | .waypoints = (.waypoints | tojson)]}' "$mimic" > mimic-connections.json
  curl -fsS -b cookies.txt -X PUT -H "$json" -d @mimic-equipment.json \
    "$url/api/v1/screens/__overview/scada-equipment" > /dev/null
  curl -fsS -b cookies.txt -X PUT -H "$json" -d @mimic-connections.json \
    "$url/api/v1/screens/__overview/scada-connections" > /dev/null
  echo "seeded the overview's mimic: $(jq '.items | length' mimic-equipment.json) widgets, $(jq '.items | length' mimic-connections.json) connections"
fi

"$bin/modbus-sim" --scenario solar-farm --role inverter --port 5020 > inverter.log 2>&1 &
echo $! > inverter.pid
"$bin/modbus-sim" --scenario solar-farm --role grid-meter --port 5021 > meter.log 2>&1 &
echo $! > meter.pid

# Wait until polled values reach the history store (it persists every 5 s).
for _ in $(seq 1 90); do
  stored="$(curl -fsS -b cookies.txt "$url/api/v1/history?range=30m" | jq '[.data[] | .values | length] | add // 0')"
  if [ "$stored" -gt 0 ]; then break; fi
  sleep 1
done
echo "history samples: $stored"
[ "$stored" -gt 0 ] || { echo "::error::no values reached the server"; tail -50 server.log; exit 1; }
echo "open alarms: $(curl -fsS -b cookies.txt "$url/api/v1/alarms?status=active&limit=100" | jq length)"

echo "server: $url"
if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "VOLTRUS_E2E_URL=$url"
    echo "VOLTRUS_E2E_USERNAME=admin"
    echo "VOLTRUS_E2E_PASSWORD=$password"
    echo "VOLTRUS_NATIVE_URL=$url"
    echo "VOLTRUS_NATIVE_USER=admin"
    echo "VOLTRUS_NATIVE_PASSWORD=$password"
  } >> "$GITHUB_ENV"
else
  echo "sign in as admin with password: $password"
  echo "stop the plant: kill \$(cat $PWD/*.pid)"
fi
