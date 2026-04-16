#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MATRIX_PATH="${1:-examples/benchmarks/programmatic-pressure-matrix.json}"
BASELINE_PATH="${2:-examples/benchmarks/programmatic-pressure-baseline.json}"
OUTPUT_PATH="${3:-target/benchmarks/programmatic-pressure-report.json}"
PREFLIGHT_FAIL_ON_WARNINGS="${4:-false}"
BENCH_PROFILE="${LOONG_BENCH_PROFILE:-${LOONGCLAW_BENCH_PROFILE:-release}}"
EXTRA_ARGS=()

if [[ "$PREFLIGHT_FAIL_ON_WARNINGS" == "true" ]]; then
  EXTRA_ARGS+=(--preflight-fail-on-warnings)
fi

if [[ "$BENCH_PROFILE" != "dev" && "$BENCH_PROFILE" != "release" ]]; then
  echo "invalid LOONG_BENCH_PROFILE/LOONGCLAW_BENCH_PROFILE: $BENCH_PROFILE (expected dev|release)" >&2
  exit 2
fi

CARGO_ARGS=(run -p loong)
if [[ "$BENCH_PROFILE" == "release" ]]; then
  CARGO_ARGS+=(--release)
fi

CMD=(
  cargo "${CARGO_ARGS[@]}" -- benchmark-programmatic-pressure
  --matrix "$MATRIX_PATH"
  --baseline "$BASELINE_PATH"
  --enforce-gate
  --output "$OUTPUT_PATH"
)

if [[ "${#EXTRA_ARGS[@]}" -gt 0 ]]; then
  CMD+=("${EXTRA_ARGS[@]}")
fi

"${CMD[@]}"
