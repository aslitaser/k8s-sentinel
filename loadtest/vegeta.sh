#!/usr/bin/env bash
#
# Quick load test using Vegeta (https://github.com/tsenart/vegeta).
# Simpler alternative to k6 — good for one-liner benchmarks.
#
# Usage:
#   ./loadtest/vegeta.sh                     # 500 RPS for 60s against /mutate
#   ./loadtest/vegeta.sh validate 200 30s    # 200 RPS for 30s against /validate
#   ./loadtest/vegeta.sh mutate 1000 120s    # 1000 RPS for 2min against /mutate
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ENDPOINT="${1:-mutate}"
RATE="${2:-500}"
DURATION="${3:-60s}"
TARGET_URL="https://localhost:8443"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC}  $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

if ! command -v vegeta &>/dev/null; then
  error "vegeta is not installed. Install it: go install github.com/tsenart/vegeta/v12@latest"
  exit 1
fi

# Pick a payload based on endpoint
PAYLOAD="$SCRIPT_DIR/payloads/valid_pod.json"

info "Vegeta attack: ${RATE} req/s for ${DURATION} → ${TARGET_URL}/${ENDPOINT}"
echo ""

# Run the attack, pipe through encode + report
echo "POST ${TARGET_URL}/${ENDPOINT}
Content-Type: application/json
@${PAYLOAD}" | \
  vegeta attack \
    -rate="${RATE}/s" \
    -duration="$DURATION" \
    -insecure \
    -timeout=5s | \
  tee "$SCRIPT_DIR/vegeta-results.bin" | \
  vegeta report

echo ""
info "Binary results saved to: $SCRIPT_DIR/vegeta-results.bin"
info "Generate plots with: vegeta plot < $SCRIPT_DIR/vegeta-results.bin > plot.html"

# Also dump latency histogram
echo ""
info "Latency histogram:"
vegeta report -type=hist[0,1ms,5ms,10ms,25ms,50ms,100ms,250ms,500ms,1s] < "$SCRIPT_DIR/vegeta-results.bin"
