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

derive_previous_month() {
  local label="$1"
  local year="${label%-*}"
  local month="${label#*-}"
  month=$((10#$month))
  if (( month == 1 )); then
    year=$((year - 1))
    month=12
  else
    month=$((month - 1))
  fi
  printf '%04d-%02d' "$year" "$month"
}

resolve_adjacent_baseline_report() {
  local report_path="$1"
  local report_month="$2"
  local report_dir
  report_dir="$(dirname "$report_path")"
  local previous_month
  previous_month="$(derive_previous_month "$report_month")"
  printf '%s/architecture-drift-%s.md\n' "$report_dir" "$previous_month"
}

normalize_architecture_drift_report() {
  local input_path="${1:?input_path is required}"
  sed '/^- Generated at: /d' "$input_path"
}

if ! git ls-files --error-unmatch "$REPORT_PATH" >/dev/null 2>&1; then
  echo "[arch-drift] report path must already be tracked by git: ${REPORT_PATH}" >&2
  exit 1
fi

GENERATE_ENV=(
  "LOONGCLAW_ARCH_REPORT_MONTH=${REPORT_MONTH}"
)

if [[ -n "${LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT:-}" ]]; then
  GENERATE_ENV+=(
    "LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT=${LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT}"
  )
else
  DEFAULT_BASELINE_REPORT="$(resolve_adjacent_baseline_report "$REPORT_PATH" "$REPORT_MONTH")"
  if [[ -f "$DEFAULT_BASELINE_REPORT" ]]; then
    GENERATE_ENV+=(
      "LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT=${DEFAULT_BASELINE_REPORT}"
    )
  fi
fi

env "${GENERATE_ENV[@]}" scripts/generate_architecture_drift_report.sh "$TEMP_REPORT"
normalize_architecture_drift_report "$REPORT_PATH" >"$NORMALIZED_TRACKED"
normalize_architecture_drift_report "$TEMP_REPORT" >"$NORMALIZED_GENERATED"

if ! diff -u "$NORMALIZED_TRACKED" "$NORMALIZED_GENERATED" >"$DIFF_OUTPUT"; then
  echo "[arch-drift] stale tracked architecture drift report: ${REPORT_PATH}" >&2
  cat "$DIFF_OUTPUT" >&2
  exit 1
fi

echo "[arch-drift] tracked architecture drift report is fresh: ${REPORT_PATH}"
