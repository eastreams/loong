#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

REPORT_MONTH="${LOONGCLAW_ARCH_REPORT_MONTH:-$(date -u +%Y-%m)}"
REPORT_PATH="${1:-docs/releases/architecture-drift-${REPORT_MONTH}.md}"
TEMP_REPORT="$(mktemp)"
NORMALIZED_TRACKED="$(mktemp)"
NORMALIZED_GENERATED="$(mktemp)"
DIFF_OUTPUT="$(mktemp)"
trap 'rm -f "$TEMP_REPORT" "$NORMALIZED_TRACKED" "$NORMALIZED_GENERATED" "$DIFF_OUTPUT"' EXIT

normalize_architecture_drift_report() {
  local input_path="${1:?input_path is required}"
  sed '/^- Generated at: /d' "$input_path"
}

if ! git ls-files --error-unmatch "$REPORT_PATH" >/dev/null 2>&1; then
  echo "[arch-drift] report path must already be tracked by git: ${REPORT_PATH}" >&2
  exit 1
fi

LOONGCLAW_ARCH_REPORT_MONTH="$REPORT_MONTH" scripts/generate_architecture_drift_report.sh "$TEMP_REPORT"
normalize_architecture_drift_report "$REPORT_PATH" >"$NORMALIZED_TRACKED"
normalize_architecture_drift_report "$TEMP_REPORT" >"$NORMALIZED_GENERATED"

if ! diff -u "$NORMALIZED_TRACKED" "$NORMALIZED_GENERATED" >"$DIFF_OUTPUT"; then
  echo "[arch-drift] stale tracked architecture drift report: ${REPORT_PATH}" >&2
  cat "$DIFF_OUTPUT" >&2
  exit 1
fi

echo "[arch-drift] tracked architecture drift report is fresh: ${REPORT_PATH}"
