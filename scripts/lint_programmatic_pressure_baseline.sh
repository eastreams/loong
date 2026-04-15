#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MATRIX_PATH="${1:-examples/benchmarks/programmatic-pressure-matrix.json}"
BASELINE_PATH="${2:-examples/benchmarks/programmatic-pressure-baseline.json}"
OUTPUT_PATH="${3:-target/benchmarks/programmatic-pressure-baseline-lint-report.json}"
FAIL_ON_WARNINGS="${4:-false}"
EXTRA_ARGS=()
COMMAND_ARGS=()

if [[ "$FAIL_ON_WARNINGS" == "true" ]]; then
  EXTRA_ARGS+=(--fail-on-warnings)
fi

COMMAND_ARGS+=(cargo)
COMMAND_ARGS+=(run)
COMMAND_ARGS+=(-p)
COMMAND_ARGS+=(loong)
COMMAND_ARGS+=(--bin)
COMMAND_ARGS+=(loong)
COMMAND_ARGS+=(--)
COMMAND_ARGS+=(benchmark-programmatic-pressure-lint)
COMMAND_ARGS+=(--matrix)
COMMAND_ARGS+=("$MATRIX_PATH")
COMMAND_ARGS+=(--baseline)
COMMAND_ARGS+=("$BASELINE_PATH")
COMMAND_ARGS+=(--enforce-gate)
if ((${#EXTRA_ARGS[@]} > 0)); then
  COMMAND_ARGS+=("${EXTRA_ARGS[@]}")
fi
COMMAND_ARGS+=(--output)
COMMAND_ARGS+=("$OUTPUT_PATH")

"${COMMAND_ARGS[@]}"
