#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

REPORT_MONTH="${LOONG_ARCH_REPORT_MONTH:-${LOONGCLAW_ARCH_REPORT_MONTH:-$(date -u +%Y-%m)}}"
REPORT_PATH="${1:-docs/releases/support/architecture-drift-${REPORT_MONTH}.md}"
REPORT_DIR="$(dirname "$REPORT_PATH")"
derive_logical_report_path() {
  python3 - "$REPORT_PATH" <<'PY'
import sys
from pathlib import PurePosixPath

report_path = sys.argv[1].replace("\\", "/")
marker = "docs/releases/"
index = report_path.find(marker)
if index >= 0:
    print(report_path[index:])
else:
    print(PurePosixPath(report_path).as_posix())
PY
}

REPORT_LOGICAL_PATH="$(derive_logical_report_path)"
BASELINE_DIR_OVERRIDE="$REPORT_DIR"
if [[ -n "${LOONG_ARCH_DRIFT_BASELINE_DIR:-}" ]]; then
  BASELINE_DIR_OVERRIDE="$LOONG_ARCH_DRIFT_BASELINE_DIR"
elif [[ -n "${LOONGCLAW_ARCH_DRIFT_BASELINE_DIR:-}" ]]; then
  BASELINE_DIR_OVERRIDE="$LOONGCLAW_ARCH_DRIFT_BASELINE_DIR"
fi
mkdir -p "$REPORT_DIR"
# Keep the temp report beside the tracked report so baseline resolution uses the same directory.
TEMP_REPORT="$(mktemp "${REPORT_DIR}/architecture-drift-check.XXXXXX")"
NORMALIZED_TRACKED="$(mktemp)"
NORMALIZED_GENERATED="$(mktemp)"
DIFF_OUTPUT="$(mktemp)"
trap 'rm -f "$TEMP_REPORT" "$NORMALIZED_TRACKED" "$NORMALIZED_GENERATED" "$DIFF_OUTPUT"' EXIT

normalize_architecture_drift_report() {
  local input_path="${1:?input_path is required}"

  # Freshness should compare report substance, not volatile provenance metadata.
  local generated_at_pattern
  generated_at_pattern='/^- Generated at: /d'

  local baseline_report_pattern
  baseline_report_pattern='/^- Baseline report: /d'

  sed     -e "$generated_at_pattern"     -e "$baseline_report_pattern"     "$input_path"
}

if ! git ls-files --error-unmatch "$REPORT_LOGICAL_PATH" >/dev/null 2>&1; then
  echo "[arch-drift] report path must already be tracked by git: ${REPORT_PATH}" >&2
  exit 1
fi

LOONG_ARCH_REPORT_MONTH="$REPORT_MONTH" \
LOONGCLAW_ARCH_REPORT_MONTH="$REPORT_MONTH" \
LOONG_ARCH_DRIFT_BASELINE_DIR="$BASELINE_DIR_OVERRIDE" \
LOONGCLAW_ARCH_DRIFT_BASELINE_DIR="$BASELINE_DIR_OVERRIDE" \
LOONG_ARCH_REPORT_LINK_PATH="$REPORT_LOGICAL_PATH" \
  scripts/generate_architecture_drift_report.sh "$TEMP_REPORT"
normalize_architecture_drift_report "$REPORT_PATH" >"$NORMALIZED_TRACKED"
normalize_architecture_drift_report "$TEMP_REPORT" >"$NORMALIZED_GENERATED"

if ! diff -u "$NORMALIZED_TRACKED" "$NORMALIZED_GENERATED" >"$DIFF_OUTPUT"; then
  echo "[arch-drift] stale tracked architecture drift report: ${REPORT_PATH}" >&2
  cat "$DIFF_OUTPUT" >&2
  exit 1
fi

echo "[arch-drift] tracked architecture drift report is fresh: ${REPORT_PATH}"
