#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

REPORT_PATH="${1:-target/benchmarks/programmatic-pressure-report.json}"
BASELINE_PATH="${2:-examples/benchmarks/programmatic-pressure-baseline.json}"

if [[ ! -f "$REPORT_PATH" ]]; then
  echo "error: report file not found: $REPORT_PATH" >&2
  exit 1
fi

if [[ ! -f "$BASELINE_PATH" ]]; then
  echo "error: baseline file not found: $BASELINE_PATH" >&2
  exit 1
fi

tmp_file="$(mktemp)"
trap 'rm -f "$tmp_file"' EXIT

jq -n \
  --slurpfile baseline "$BASELINE_PATH" \
  --slurpfile report "$REPORT_PATH" '
  reduce ($report[0].scenarios[] | select(.schema_fingerprint != null)) as $scenario ($baseline[0];
    if .scenarios[$scenario.name] then
      .scenarios[$scenario.name].expected_schema_fingerprint = $scenario.schema_fingerprint
    else
      error("baseline is missing scenario: \($scenario.name)")
    end
  )
' >"$tmp_file"

mv "$tmp_file" "$BASELINE_PATH"
trap - EXIT

echo "updated schema fingerprints in $BASELINE_PATH from $REPORT_PATH"
