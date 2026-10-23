#!/usr/bin/env bash
# Run the full Voltrus test suite — simulates what CI runs
# Usage: bash run-all-tests.sh
set -e

echo "╔══════════════════════════════════════════════════╗"
echo "║         Voltrus Full Test Suite                   ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

FAIL=0
START=$(date +%s)

echo "1/5  Formatting check (cargo fmt --check)..."
if cargo fmt -- --check; then
  echo "     ✅ Pass"
else
  echo "     ❌ FAIL — run 'cargo fmt' to fix"
  FAIL=1
fi
echo ""

echo "2/5  Clippy strict mode (-D warnings)..."
if cargo clippy -- -D warnings 2>&1; then
  echo "     ✅ Pass"
else
  echo "     ❌ FAIL — fix clippy warnings"
  FAIL=1
fi
echo ""

echo "3/5  Rust unit + integration tests..."
if cargo test 2>&1; then
  RESULT=$(cargo test 2>&1 | grep "test result:" | tail -1)
  echo "     ✅ $RESULT"
else
  echo "     ❌ FAIL — some tests failed"
  FAIL=1
fi
echo ""

echo "4/5  Release build..."
if cargo build --release 2>&1; then
  echo "     ✅ Pass"
else
  echo "     ❌ FAIL — build error"
  FAIL=1
fi
echo ""

echo "5/5  Frontend build..."
cd client
if npm ci --quiet && npm run build; then
  echo "     ✅ Pass"
else
  echo "     ❌ FAIL — frontend build error"
  FAIL=1
fi
cd ..
echo ""

END=$(date +%s)
DURATION=$((END - START))

echo "══════════════════════════════════════════════════"
echo ""
if [ $FAIL -eq 0 ]; then
  echo "╔══════════════════════════════════════════════════╗"
  echo "║  ✅ ALL 5 TESTS PASSED — Ready for deployment    ║"
  echo "║  Duration: ${DURATION}s                                    ║"
  echo "╚══════════════════════════════════════════════════╝"
else
  echo "╔══════════════════════════════════════════════════╗"
  echo "║  ❌ SOME TESTS FAILED — Do NOT deploy             ║"
  echo "║  Duration: ${DURATION}s                                    ║"
  echo "╚══════════════════════════════════════════════════╝"
  exit 1
fi
