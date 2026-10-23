#!/usr/bin/env bash
# Water Treatment Plant — Municipal 50,000 m³/d
#
# Devices:
#   S7     :10200 — Siemens S7-1200 process PLC (DB1 coagulation, DB2 filtration, DB3 disinfection)
#   Modbus :5020  — Endress+Hauser Promag 10W flowmeter (raw water intake)
#   Modbus :5021  — ABB AZL402 water quality analyzer (pH, turbidity, DO, chlorine)
#   Modbus :5022  — Schneider PM5560 main power meter
#   BACnet :47809 — Johnson Controls FEC HVAC controller (treatment building)
#   MQTT   :1883  — SCADA gateway telemetry

SESSION="voltrus-$SCENARIO"
BIN="target/release"

# ── S7: Process PLC ──
tmux new-window -t "$SESSION" -n "s7-process"
tmux send-keys -t "$SESSION:s7-process" \
    "$BIN/s7-sim --scenario water-treatment --port 10200 2>&1 | sed 's/^/[s7-wtp] /'" C-m

# ── Modbus: E+H Flowmeter ──
tmux new-window -t "$SESSION" -n "modbus-flow"
tmux send-keys -t "$SESSION:modbus-flow" \
    "$BIN/modbus-sim --scenario water-treatment --role flowmeter --port 5020 2>&1 | sed 's/^/[eh-flow] /'" C-m

# ── Modbus: ABB Analyzer ──
tmux new-window -t "$SESSION" -n "modbus-analyzer"
tmux send-keys -t "$SESSION:modbus-analyzer" \
    "$BIN/modbus-sim --scenario water-treatment --role analyzer --port 5021 2>&1 | sed 's/^/[abb-wq] /'" C-m

# ── Modbus: PM5560 Power ──
tmux new-window -t "$SESSION" -n "modbus-power"
tmux send-keys -t "$SESSION:modbus-power" \
    "$BIN/modbus-sim --scenario water-treatment --role grid-meter --port 5022 2>&1 | sed 's/^/[pm5560] /'" C-m

# ── BACnet: HVAC ──
tmux new-window -t "$SESSION" -n "bacnet-hvac"
tmux send-keys -t "$SESSION:bacnet-hvac" \
    "$BIN/bacnet-sim --scenario water-treatment --port 47809 --device 101 2>&1 | sed 's/^/[jc-hvac] /'" C-m

# ── MQTT: SCADA Gateway ──
tmux new-window -t "$SESSION" -n "mqtt-scada"
tmux send-keys -t "$SESSION:mqtt-scada" \
    "$BIN/mqtt-sim --scenario water-treatment --port 1883 2>&1 | sed 's/^/[mqtt-gw] /'" C-m

sleep 2

# ── Voltrus ──
tmux new-window -t "$SESSION" -n "voltrus"
tmux send-keys -t "$SESSION:voltrus" \
    "$BIN/voltrus $SCENARIO_DIR/voltrus.yaml" C-m

VOLTRUS_PORT=3000
