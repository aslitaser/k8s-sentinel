#!/usr/bin/env bash
#
# Run k6 load tests against a local k8s-sentinel instance.
# Usage:
#   ./loadtest/run.sh                    # run all scenarios
#   ./loadtest/run.sh baseline           # run a single scenario
#   BINARY=./target/debug/k8s-sentinel ./loadtest/run.sh  # custom binary
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

BINARY="${BINARY:-$ROOT_DIR/target/release/k8s-sentinel}"
CONFIG="${CONFIG:-$ROOT_DIR/config/policies.yaml}"
HEALTH_URL="http://localhost:9090/healthz"
TARGET_URL="https://localhost:8443"
SCENARIO="${1:-}"
K6_OUT="${K6_OUT:-$SCRIPT_DIR/results.json}"

# -- Colors ----------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[INFO]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# -- Preflight checks -----------------------------------------------------
for cmd in k6; do
  if ! command -v "$cmd" &>/dev/null; then
    error "$cmd is not installed. Install it: https://grafana.com/docs/k6/latest/set-up/install-k6/"
    exit 1
  fi
done

if [[ ! -x "$BINARY" ]]; then
  info "Binary not found at $BINARY, building release..."
  cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml"
fi

if [[ ! -f "$ROOT_DIR/certs/tls.crt" ]]; then
  warn "TLS certs not found. Generating self-signed certs..."
  if [[ -x "$ROOT_DIR/generate-certs.sh" ]]; then
    "$ROOT_DIR/generate-certs.sh"
  else
    mkdir -p "$ROOT_DIR/certs"
    openssl req -x509 -newkey rsa:2048 -nodes \
      -keyout "$ROOT_DIR/certs/tls.key" \
      -out "$ROOT_DIR/certs/tls.crt" \
      -days 365 \
      -subj "/CN=localhost" \
      -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
      2>/dev/null
    info "Self-signed certs generated in $ROOT_DIR/certs/"
  fi
fi

# -- Start server ----------------------------------------------------------
info "Starting k8s-sentinel..."
"$BINARY" --config "$CONFIG" &
SERVER_PID=$!

cleanup() {
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    info "Stopping server (PID $SERVER_PID)..."
    kill "$SERVER_PID"
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# -- Wait for healthy ------------------------------------------------------
info "Waiting for server health check..."
MAX_WAIT=15
for i in $(seq 1 $MAX_WAIT); do
  if curl -sf "$HEALTH_URL" >/dev/null 2>&1; then
    info "Server is healthy (took ${i}s)"
    break
  fi
  if [[ $i -eq $MAX_WAIT ]]; then
    error "Server failed to become healthy after ${MAX_WAIT}s"
    exit 1
  fi
  sleep 1
done

# -- Run k6 ----------------------------------------------------------------
info "Running k6 load test..."
if [[ -n "$SCENARIO" ]]; then
  info "Scenario: $SCENARIO"
fi

K6_ARGS=(
  run
  --out "json=$K6_OUT"
  --env "TARGET_URL=$TARGET_URL"
  --summary-trend-stats "avg,min,med,max,p(90),p(95),p(99)"
)

if [[ -n "$SCENARIO" ]]; then
  K6_ARGS+=(--env "SCENARIO=$SCENARIO")
fi

K6_ARGS+=("$SCRIPT_DIR/k6.js")

k6 "${K6_ARGS[@]}"
K6_EXIT=$?

# -- Summary ---------------------------------------------------------------
echo ""
if [[ $K6_EXIT -eq 0 ]]; then
  info "Load test PASSED"
  info "Results written to: $K6_OUT"
else
  error "Load test FAILED (exit code: $K6_EXIT)"
  error "Results written to: $K6_OUT"
fi

exit $K6_EXIT
