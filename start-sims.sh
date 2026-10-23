#!/usr/bin/env bash
# Start all 7 Voltrus protocol simulators + main server in a tmux session
# Usage:   bash start-sims.sh [config-path]
# Kill:    tmux kill-session -t voltrus-sims
# Attach:  tmux attach -t voltrus-sims

cd "$(dirname "$0")"
SESSION="voltrus-sims"
CONFIG="${1:-scenarios/factory-floor/voltrus.yaml}"

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}=== Starting Voltrus Simulator Stack ===${NC}"
echo ""

# Kill existing session if any
tmux kill-session -t "$SESSION" 2>/dev/null

# Build first
echo "Building release binaries..."
cargo build --release 2>/dev/null
echo -e "${GREEN}Build complete.${NC}"
echo ""

tmux new-session -d -s "$SESSION" -n "modbus"
tmux send-keys -t "$SESSION:modbus" "cargo run --release --bin modbus-sim -- --port 5020 2>&1 | sed 's/^/[modbus] /'" C-m

tmux new-window -t "$SESSION" -n "s7"
tmux send-keys -t "$SESSION:s7" "cargo run --release --bin s7-sim -- --port 10200 2>&1 | sed 's/^/[s7] /'" C-m

tmux new-window -t "$SESSION" -n "opcua"
tmux send-keys -t "$SESSION:opcua" "cargo run --release --bin opcua-sim -- --port 4840 2>&1 | sed 's/^/[opcua] /'" C-m

tmux new-window -t "$SESSION" -n "enip"
tmux send-keys -t "$SESSION:enip" "cargo run --release --bin enip-sim -- --port 44818 2>&1 | sed 's/^/[enip] /'" C-m

tmux new-window -t "$SESSION" -n "dnp3"
tmux send-keys -t "$SESSION:dnp3" "cargo run --release --bin dnp3-sim -- 20000 10 2>&1 | sed 's/^/[dnp3] /'" C-m

tmux new-window -t "$SESSION" -n "bacnet"
tmux send-keys -t "$SESSION:bacnet" "cargo run --release --bin bacnet-sim -- --port 47809 2>&1 | sed 's/^/[bacnet] /'" C-m

tmux new-window -t "$SESSION" -n "mqtt"
tmux send-keys -t "$SESSION:mqtt" "cargo run --release --bin mqtt-sim -- --port 1883 2>&1 | sed 's/^/[mqtt] /'" C-m

# Wait for simulators to bind
sleep 2

tmux new-window -t "$SESSION" -n "voltrus"
tmux send-keys -t "$SESSION:voltrus" "cargo run --release -- $CONFIG" C-m

tmux select-window -t "$SESSION:voltrus"

echo ""
echo -e "${GREEN}Simulators running in tmux session '${SESSION}'${NC}"
echo ""
echo "  Windows:"
echo "    0: modbus   (port 5020)"
echo "    1: s7       (port 10200)"
echo "    2: opcua    (port 4840)"
echo "    3: enip     (port 44818)"
echo "    4: dnp3     (port 20000)"
echo "    5: bacnet   (port 47809)"
echo "    6: mqtt     (port 1883)"
echo "    7: voltrus  (main server)"
echo ""
echo "  Commands:"
echo "    tmux attach -t $SESSION    # attach to session"
echo "    Ctrl+b n                    # next window"
echo "    Ctrl+b p                    # previous window"
echo "    Ctrl+b d                    # detach"
echo "    tmux kill-session -t $SESSION  # stop everything"
echo ""
echo -e "  Dashboard: ${CYAN}http://localhost:3000${NC}"
echo ""
