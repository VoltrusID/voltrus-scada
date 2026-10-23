#!/usr/bin/env bash
# Oil & Gas — Remote Well Pad
#
# Devices:
#   Modbus :5020  — Daniel 1000 gas flow computer (orifice meter, gas chromatograph)
#   DNP3   :20000 — SEL-351S well pump motor protection relay
#   DNP3   :20001 — Emerson ROC800 RTU (flow aggregation)
#   S7     :10200 — Siemens S7-1200 compressor controller
#   MQTT   :1883  — Satellite gateway telemetry

SESSION="voltrus-$SCENARIO"
BIN="target/release"

# ── Modbus: Daniel Flow Computer ──
tmux new-window -t "$SESSION" -n "modbus-daniel"
tmux send-keys -t "$SESSION:modbus-daniel" \
    "$BIN/modbus-sim --scenario oil-gas-well --role gas-flow --port 5020 2>&1 | sed 's/^/[daniel-fc] /'" C-m

# ── DNP3: SEL-351S Motor Protection ──
tmux new-window -t "$SESSION" -n "dnp3-sel"
tmux send-keys -t "$SESSION:dnp3-sel" \
    "$BIN/dnp3-sim 20000 10 2>&1 | sed 's/^/[sel-351] /'" C-m

# ── DNP3: ROC800 RTU ──
tmux new-window -t "$SESSION" -n "dnp3-roc"
tmux send-keys -t "$SESSION:dnp3-roc" \
    "$BIN/dnp3-sim 20001 20 2>&1 | sed 's/^/[roc800] /'" C-m

# ── S7: Compressor Controller ──
tmux new-window -t "$SESSION" -n "s7-compressor"
tmux send-keys -t "$SESSION:s7-compressor" \
    "$BIN/s7-sim --scenario oil-gas-well --port 10200 2>&1 | sed 's/^/[s7-comp] /'" C-m

# ── MQTT: Satellite Gateway ──
tmux new-window -t "$SESSION" -n "mqtt-sat"
tmux send-keys -t "$SESSION:mqtt-sat" \
    "$BIN/mqtt-sim --scenario oil-gas-well --port 1883 2>&1 | sed 's/^/[sat-gw] /'" C-m

sleep 2

# ── Voltrus ──
tmux new-window -t "$SESSION" -n "voltrus"
tmux send-keys -t "$SESSION:voltrus" \
    "$BIN/voltrus $SCENARIO_DIR/voltrus.yaml" C-m

VOLTRUS_PORT=3000
