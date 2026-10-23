#!/usr/bin/env bash
# Voltrus Scenario Launcher
# Usage: ./run-scenario.sh <scenario-name>
#
# Scenarios:
#   solar-farm         100kW Rooftop PV Plant
#   water-treatment    Municipal Water Treatment Plant
#   factory-floor      Food & Beverage Factory Floor
#   data-center        Tier-III Data Center
#   oil-gas-well       Remote Oil & Gas Well Pad
#
# Each scenario starts multiple simulator instances on different ports,
# plus a Voltrus server with the matching config.

set -e

GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
NC='\033[0m'

SCENARIO="${1:-}"
if [ -z "$SCENARIO" ]; then
    echo "Usage: $0 <scenario-name>"
    echo ""
    echo "Available scenarios:"
    echo "  solar-farm         100kW Rooftop PV Plant"
    echo "  water-treatment    Municipal Water Treatment Plant"
    echo "  factory-floor      Food & Beverage Factory Floor"
    echo "  data-center        Tier-III Data Center"
    echo "  oil-gas-well       Remote Oil & Gas Well Pad"
    echo ""
    echo "Example:"
    echo "  $0 solar-farm"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCENARIO_DIR="$SCRIPT_DIR/scenarios/$SCENARIO"

if [ ! -d "$SCENARIO_DIR" ]; then
    echo "Unknown scenario: $SCENARIO"
    exit 1
fi

# Build first
echo -e "${CYAN}Building voltrus and simulators...${NC}"
cargo build --release 2>/dev/null
echo -e "${GREEN}Build complete.${NC}"
echo ""

# Kill any existing session
tmux kill-session -t "voltrus-$SCENARIO" 2>/dev/null || true

SESSION="voltrus-$SCENARIO"
tmux new-session -d -s "$SESSION" -n "info"

# Run the scenario-specific launcher
source "$SCENARIO_DIR/start.sh"

# Select window to Voltrus
tmux select-window -t "$SESSION:voltrus"

echo ""
echo -e "${GREEN}Scenario '$SCENARIO' running in tmux session '$SESSION'${NC}"
echo ""
echo "  Dashboard: ${CYAN}http://localhost:${VOLTRUS_PORT:-3000}${NC}"
echo ""
echo "  tmux commands:"
echo "    tmux attach -t $SESSION     # attach"
echo "    Ctrl+b n                     # next window"
echo "    Ctrl+b p                     # prev window"
echo "    Ctrl+b d                     # detach"
echo "    tmux kill-session -t $SESSION  # stop all"
echo ""
