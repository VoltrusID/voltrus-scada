#!/usr/bin/env bash
# Quick health check for all Voltrus components
# Usage: bash health-check.sh [host]

HOST="${1:-http://localhost:3000}"

echo "╔══════════════════════════════════════════════════╗"
echo "║         Voltrus Health Check                     ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

PASS=0
FAIL=0
WARN=0

check_port() {
  local name=$1
  local port=$2
  if nc -z 127.0.0.1 "$port" 2>/dev/null; then
    echo -e "  $name:  \033[0;32m✅ Running (port $port)\033[0m"
    ((PASS++))
  else
    echo -e "  $name:  \033[0;33m⚠️  Not running\033[0m"
    ((WARN++))
  fi
}

# Server
echo "── Server ──"
HTTP_CODE=$(curl -s -o /dev/null -w '%{http_code}' "$HOST/api/v1/health" 2>/dev/null)
if [ "$HTTP_CODE" = "200" ]; then
  echo -e "  Health:  \033[0;32m✅ OK ($HTTP_CODE)\033[0m"
  ((PASS++))
else
  echo -e "  Health:  \033[0;31m❌ FAIL ($HTTP_CODE)\033[0m"
  ((FAIL++))
fi

VERSION=$(curl -s "$HOST/api/v1/version" 2>/dev/null | grep -o '"version":"[^"]*"' | cut -d'"' -f4)
if [ -n "$VERSION" ]; then
  echo "  Version: $VERSION"
else
  echo "  Version: (unable to detect)"
fi

# Simulators
echo ""
echo "── Simulators ──"
check_port "Modbus TCP" 5020
check_port "S7          " 10200
check_port "OPC-UA      " 4840
check_port "EtherNet/IP " 44818
check_port "DNP3        " 20000
check_port "BACnet/IP   " 47809
check_port "MQTT        " 1883

# Database
echo ""
echo "── Database ──"
for db in test-voltrus.db voltrus.db data.db; do
  if [ -f "$db" ]; then
    SIZE=$(du -h "$db" | cut -f1)
    INTEGRITY=$(sqlite3 "$db" "PRAGMA integrity_check;" 2>/dev/null | head -1)
    if [ "$INTEGRITY" = "ok" ]; then
      echo -e "  $db: \033[0;32m✅ $SIZE (integrity: ok)\033[0m"
      ((PASS++))
    else
      echo -e "  $db: \033[0;31m❌ $SIZE (integrity: $INTEGRITY)\033[0m"
      ((FAIL++))
    fi
    break
  fi
done

# Rust tests
echo ""
echo "── Automated Tests ──"
echo -n "  Running cargo test... "
if cargo test 2>&1 | tail -1 | grep -q "passed"; then
  RESULT=$(cargo test 2>&1 | tail -1)
  echo -e "\033[0;32m✅ $RESULT\033[0m"
  ((PASS++))
else
  echo -e "\033[0;31m❌ Failed\033[0m"
  ((FAIL++))
fi

# Summary
echo ""
echo "══════════════════════════════════════════════════"
echo -e "  \033[0;32m$PASS passed\033[0m, \033[0;33m$WARN warnings\033[0m, \033[0;31m$FAIL failed\033[0m"
echo ""
