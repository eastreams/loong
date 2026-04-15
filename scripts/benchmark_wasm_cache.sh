#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

WASM_PATH="${1:-examples/plugins-wasm/secure_echo.wasm}"
OUTPUT_PATH="${2:-target/benchmarks/wasm-cache-benchmark-report.json}"
COLD_ITERATIONS="${3:-8}"
HOT_ITERATIONS="${4:-24}"
WARMUP_ITERATIONS="${5:-2}"
ENFORCE_GATE="${6:-true}"
MIN_SPEEDUP_RATIO="${7:-1.5}"
BENCH_PROFILE="${LOONG_BENCH_PROFILE:-${LOONGCLAW_BENCH_PROFILE:-release}}"

if [[ "$BENCH_PROFILE" != "dev" && "$BENCH_PROFILE" != "release" ]]; then
  echo "invalid LOONG_BENCH_PROFILE/LOONGCLAW_BENCH_PROFILE: $BENCH_PROFILE (expected dev|release)" >&2
  exit 2
fi

if [[ "$ENFORCE_GATE" != "true" && "$ENFORCE_GATE" != "false" ]]; then
  echo "invalid ENFORCE_GATE value: $ENFORCE_GATE (expected true|false)" >&2
  exit 2
fi

CARGO_ARGS=(run -p loong)
if [[ "$BENCH_PROFILE" == "release" ]]; then
  CARGO_ARGS+=(--release)
fi

CMD=(
  cargo "${CARGO_ARGS[@]}" -- benchmark-wasm-cache
  --wasm "$WASM_PATH"
  --output "$OUTPUT_PATH"
  --cold-iterations "$COLD_ITERATIONS"
  --hot-iterations "$HOT_ITERATIONS"
  --warmup-iterations "$WARMUP_ITERATIONS"
  --min-speedup-ratio "$MIN_SPEEDUP_RATIO"
)

if [[ "$ENFORCE_GATE" == "true" ]]; then
  CMD+=(--enforce-gate)
fi

"${CMD[@]}"
