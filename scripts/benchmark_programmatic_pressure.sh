#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MATRIX_PATH="${1:-examples/benchmarks/programmatic-pressure-matrix.json}"
BASELINE_PATH="${2:-examples/benchmarks/programmatic-pressure-baseline.json}"
OUTPUT_PATH="${3:-target/benchmarks/programmatic-pressure-report.json}"
PREFLIGHT_FAIL_ON_WARNINGS="${4:-false}"
EXTRA_ARGS=()

if [[ "$PREFLIGHT_FAIL_ON_WARNINGS" == "true" ]]; then
  EXTRA_ARGS+=(--preflight-fail-on-warnings)
fi

cargo run -p loongclaw-daemon -- benchmark-programmatic-pressure \
  --matrix "$MATRIX_PATH" \
  --baseline "$BASELINE_PATH" \
  --enforce-gate \
  "${EXTRA_ARGS[@]}" \
  --output "$OUTPUT_PATH"
