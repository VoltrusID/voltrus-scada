#!/usr/bin/env bash
# Solar Farm — 100kW Rooftop PV Plant
#
# Devices:
#   Modbus :5020  — SMA STP 60-US-10 solar inverter (DC/AC, yield, irradiance)
#   Modbus :5021  — Schneider PM5560 grid interconnection meter (import/export)
#   S7     :10200 — Siemens S7-1200 weather station + irradiance
#   MQTT   :1883  — Weather gateway telemetry

SESSION="voltrus-$SCENARIO"
BIN="target/release"

# ── Modbus: SMA Inverter ──
tmux new-window -t "$SESSION" -n "modbus-inv"
tmux send-keys -t "$SESSION:modbus-inv" \
    "$BIN/modbus-sim --scenario solar-farm --role inverter --port 5020 2>&1 | sed 's/^/[sma-inv] /'" C-m

# ── Modbus: PM5560 Grid Meter ──
tmux new-window -t "$SESSION" -n "modbus-meter"
tmux send-keys -t "$SESSION:modbus-meter" \
    "$BIN/modbus-sim --scenario solar-farm --role grid-meter --port 5021 2>&1 | sed 's/^/[pm5560] /'" C-m

# ── S7: Weather Station ──
tmux new-window -t "$SESSION" -n "s7-weather"
tmux send-keys -t "$SESSION:s7-weather" \
    "$BIN/s7-sim --scenario solar-farm --port 10200 2>&1 | sed 's/^/[s7-wx] /'" C-m

# ── MQTT: Weather Gateway ──
tmux new-window -t "$SESSION" -n "mqtt-weather"
tmux send-keys -t "$SESSION:mqtt-weather" \
    "$BIN/mqtt-sim --scenario solar-farm --port 1883 2>&1 | sed 's/^/[mqtt-wx] /'" C-m

# Wait for simulators to bind
sleep 2

# ── Voltrus Server ──
tmux new-window -t "$SESSION" -n "voltrus"
tmux send-keys -t "$SESSION:voltrus" \
    "$BIN/voltrus $SCENARIO_DIR/voltrus.yaml" C-m

VOLTRUS_PORT=3000
