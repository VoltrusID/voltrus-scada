#!/usr/bin/env bash
# Factory Floor — Food & Beverage Filling Line
#
# Devices:
#   ENIP   :44818 — Allen-Bradley CompactLogix L33ER (line control, filler, labeler, case packer)
#   Modbus :5020  — Schneider PM5560 main power inlet
#   S7     :10200 — Siemens S7-1200 CIP (Clean-In-Place) system
#   DNP3   :20000 — SEL-351S backup generator protection relay

SESSION="voltrus-$SCENARIO"
BIN="target/release"

# ── EtherNet/IP: CompactLogix ──
tmux new-window -t "$SESSION" -n "enip-plc"
tmux send-keys -t "$SESSION:enip-plc" \
    "$BIN/enip-sim --scenario factory-floor --port 44818 2>&1 | sed 's/^/[l33er] /'" C-m

# ── Modbus: PM5560 ──
tmux new-window -t "$SESSION" -n "modbus-power"
tmux send-keys -t "$SESSION:modbus-power" \
    "$BIN/modbus-sim --scenario factory-floor --role grid-meter --port 5020 2>&1 | sed 's/^/[pm5560] /'" C-m

# ── S7: CIP System ──
tmux new-window -t "$SESSION" -n "s7-cip"
tmux send-keys -t "$SESSION:s7-cip" \
    "$BIN/s7-sim --scenario factory-floor --port 10200 2>&1 | sed 's/^/[s7-cip] /'" C-m

# ── DNP3: Generator Relay ──
tmux new-window -t "$SESSION" -n "dnp3-gen"
tmux send-keys -t "$SESSION:dnp3-gen" \
    "$BIN/dnp3-sim 20000 10 2>&1 | sed 's/^/[sel351] /'" C-m

sleep 2

# ── Voltrus ──
tmux new-window -t "$SESSION" -n "voltrus"
tmux send-keys -t "$SESSION:voltrus" \
    "$BIN/voltrus $SCENARIO_DIR/voltrus.yaml" C-m

VOLTRUS_PORT=3000
