#!/usr/bin/env bash
# Data Center — Tier-III, 2MW Critical Load
#
# Devices:
#   Modbus :5020  — Raritan PX3-5902V rack PDU (Hall A, Row 1)
#   Modbus :5021  — Schneider PM5560 UPS input/output meter
#   BACnet :47809 — Trane Tracer SC+ CRAC unit (Hall A)
#   MQTT   :1883  — Environmental sensors gateway (temp, RH, leak, door)
#   OPC-UA :4840  — DCIM aggregation server

SESSION="voltrus-$SCENARIO"
BIN="target/release"

# ── Modbus: Raritan PDU ──
tmux new-window -t "$SESSION" -n "modbus-pdu"
tmux send-keys -t "$SESSION:modbus-pdu" \
    "$BIN/modbus-sim --scenario data-center --role pdu --port 5020 2>&1 | sed 's/^/[px3-pdu] /'" C-m

# ── Modbus: PM5560 UPS ──
tmux new-window -t "$SESSION" -n "modbus-ups"
tmux send-keys -t "$SESSION:modbus-ups" \
    "$BIN/modbus-sim --scenario data-center --role ups --port 5021 2>&1 | sed 's/^/[pm5560-ups] /'" C-m

# ── BACnet: Trane CRAC ──
tmux new-window -t "$SESSION" -n "bacnet-crac"
tmux send-keys -t "$SESSION:bacnet-crac" \
    "$BIN/bacnet-sim --scenario data-center --port 47809 --device 201 2>&1 | sed 's/^/[trane-crac] /'" C-m

# ── MQTT: Environmental Sensors ──
tmux new-window -t "$SESSION" -n "mqtt-env"
tmux send-keys -t "$SESSION:mqtt-env" \
    "$BIN/mqtt-sim --scenario data-center --port 1883 2>&1 | sed 's/^/[dc-env] /'" C-m

# ── OPC-UA: DCIM Server ──
tmux new-window -t "$SESSION" -n "opcua-dcim"
tmux send-keys -t "$SESSION:opcua-dcim" \
    "$BIN/opcua-sim --scenario data-center --port 4840 2>&1 | sed 's/^/[dcim] /'" C-m

sleep 2

# ── Voltrus ──
tmux new-window -t "$SESSION" -n "voltrus"
tmux send-keys -t "$SESSION:voltrus" \
    "$BIN/voltrus $SCENARIO_DIR/voltrus.yaml" C-m

VOLTRUS_PORT=3000
