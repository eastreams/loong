#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
. "$REPO_ROOT/scripts/architecture_budget_lib.sh"

REPORT_MONTH="${LOONGCLAW_ARCH_REPORT_MONTH:-$(date +%Y-%m)}"
OUTPUT_PATH="${1:-docs/releases/architecture-drift-${REPORT_MONTH}.md}"
EXPLICIT_BASELINE="${LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT:-}"
EXPLICIT_BASELINE_DIR="${LOONGCLAW_ARCH_DRIFT_BASELINE_DIR:-}"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

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

resolve_baseline_path() {
  if [[ -n "$EXPLICIT_BASELINE" ]]; then
    printf '%s\n' "$EXPLICIT_BASELINE"
    return 0
  fi

  local previous_month
  previous_month="$(derive_previous_month "$REPORT_MONTH")"
  local baseline_dir
  if [[ -n "$EXPLICIT_BASELINE_DIR" ]]; then
    baseline_dir="$EXPLICIT_BASELINE_DIR"
  else
    baseline_dir="$(dirname "$OUTPUT_PATH")"
  fi
  printf '%s/architecture-drift-%s.md\n' "$baseline_dir" "$previous_month"
}

baseline_hotspot_value() {
  local baseline_file="$1"
  local key="$2"
  local field="$3"
  [[ -f "$baseline_file" ]] || return 1
  local line
  line="$(grep -E "^<!-- arch-hotspot key=${key} lines=[0-9]+ functions=[0-9]+ -->$" "$baseline_file" | head -n 1 || true)"
  [[ -n "$line" ]] || return 1
  case "$field" in
    lines)
      printf '%s\n' "$line" | sed -E 's/^<!-- arch-hotspot key=[^ ]+ lines=([0-9]+) functions=[0-9]+ -->$/\1/'
      ;;
    functions)
      printf '%s\n' "$line" | sed -E 's/^<!-- arch-hotspot key=[^ ]+ lines=[0-9]+ functions=([0-9]+) -->$/\1/'
      ;;
    *)
      return 1
      ;;
  esac
}

baseline_boundary_status() {
  local baseline_file="$1"
  local key="$2"
  [[ -f "$baseline_file" ]] || return 1
  local line
  line="$(grep -E "^<!-- arch-boundary key=${key} status=(PASS|FAIL) -->$" "$baseline_file" | head -n 1 || true)"
  [[ -n "$line" ]] || return 1
  printf '%s\n' "$line" | sed -E 's/^<!-- arch-boundary key=[^ ]+ status=(PASS|FAIL) -->$/\1/'
}

format_percent_growth() {
  local current="$1"
  local previous="$2"
  if [[ -z "$previous" || "$previous" -eq 0 ]]; then
    echo "n/a"
    return 0
  fi
  awk -v current="$current" -v previous="$previous" 'BEGIN { printf "%.1f%%", ((current - previous) / previous) * 100 }'
}

join_by_comma() {
  local item
  local output=""
  for item in "$@"; do
    [[ -z "$item" ]] && continue
    if [[ -n "$output" ]]; then
      output="${output}, "
    fi
    output="${output}${item}"
  done
  printf '%s\n' "$output"
}

growth_slo_status() {
  local current="$1"
  local previous="$2"
  if [[ -z "$previous" || "$previous" -eq 0 ]]; then
    echo "N/A"
    return 0
  fi
  awk -v current="$current" -v previous="$previous" 'BEGIN { if (current > previous * 1.10) print "BREACH"; else print "PASS"; }'
}

mkdir -p "$(dirname "$OUTPUT_PATH")"

BASELINE_PATH="$(resolve_baseline_path)"
if [[ -f "$BASELINE_PATH" ]]; then
  BASELINE_LABEL="$BASELINE_PATH"
  BASELINE_AVAILABLE=1
else
  BASELINE_AVAILABLE=0
  if [[ -n "$EXPLICIT_BASELINE" ]]; then
    BASELINE_LABEL="missing: $BASELINE_PATH"
  else
    BASELINE_LABEL="none"
  fi
fi

tmp_hotspots="$(mktemp)"
tmp_boundaries="$(mktemp)"
trap 'rm -f "$tmp_hotspots" "$tmp_boundaries"' EXIT

hotspot_breach=0
boundary_breach=0
hotspot_count=0
boundary_count=0
breach_hotspots=()
tight_hotspots=()
watch_hotspots=()
mixed_class_hotspots=()
breach_hotspot_summary="none"
tight_hotspot_summary="none"
watch_hotspot_summary="none"
mixed_class_hotspot_summary="none"
hotspot_rows="$(architecture_hotspot_rows)" || exit 1

while IFS='|' read -r key file classes lines max_lines _line_status functions max_functions _fn_status peak_usage pressure; do
  [[ -z "$key" ]] && continue
  hotspot_count=$((hotspot_count + 1))
  prev_lines="$(baseline_hotspot_value "$BASELINE_PATH" "$key" lines || true)"
  prev_functions="$(baseline_hotspot_value "$BASELINE_PATH" "$key" functions || true)"
  line_growth="$(format_percent_growth "$lines" "${prev_lines:-}")"
  growth_status="$(growth_slo_status "$lines" "${prev_lines:-}")"
  if [[ "$growth_status" == "BREACH" ]]; then
    hotspot_breach=1
  fi
  line_headroom=$((max_lines - lines))
  fn_headroom=$((max_functions - functions))
  case "$pressure" in
    BREACH)
      breach_hotspots+=("${key} (${peak_usage})")
      ;;
    TIGHT)
      tight_hotspots+=("${key} (${peak_usage})")
      ;;
    WATCH)
      watch_hotspots+=("${key} (${peak_usage})")
      ;;
  esac
  if [[ "$classes" == *,* ]]; then
    mixed_class_hotspots+=("$key")
  fi
  printf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
    "$key" "$classes" "$file" "$lines" "$max_lines" "$line_headroom" "$functions" "$max_functions" "$fn_headroom" \
    "$peak_usage" "$pressure" "${prev_lines:-n/a}" "$line_growth" "$growth_status" "${prev_functions:-n/a}" >>"$tmp_hotspots"
done <<EOF_HOTSPOTS
${hotspot_rows}
EOF_HOTSPOTS

if (( ${#breach_hotspots[@]} > 0 )); then
  breach_hotspot_summary="$(join_by_comma "${breach_hotspots[@]}")"
fi

if (( ${#tight_hotspots[@]} > 0 )); then
  tight_hotspot_summary="$(join_by_comma "${tight_hotspots[@]}")"
fi

if (( ${#watch_hotspots[@]} > 0 )); then
  watch_hotspot_summary="$(join_by_comma "${watch_hotspots[@]}")"
fi

if (( ${#mixed_class_hotspots[@]} > 0 )); then
  mixed_class_hotspot_summary="$(join_by_comma "${mixed_class_hotspots[@]}")"
fi

while IFS= read -r boundary_key; do
  [[ -z "$boundary_key" ]] && continue
  boundary_count=$((boundary_count + 1))
  status="$(architecture_boundary_status "$boundary_key")"
  previous_status="$(baseline_boundary_status "$BASELINE_PATH" "$boundary_key" || true)"
  if [[ -z "$previous_status" ]]; then
    previous_status="n/a"
  fi
  detail="$(architecture_boundary_detail_single_line "$boundary_key")"
  if [[ "$status" == "FAIL" ]]; then
    boundary_breach=1
  fi
  printf '%s|%s|%s|%s\n' "$boundary_key" "$status" "$previous_status" "$detail" >>"$tmp_boundaries"
done <<EOF_BOUNDARIES
$(architecture_boundary_check_keys)
EOF_BOUNDARIES

if [[ "$hotspot_breach" -eq 1 || "$boundary_breach" -eq 1 ]]; then
  overall_status="FAIL"
else
  overall_status="PASS"
fi

{
  echo "# Architecture Drift Report ${REPORT_MONTH}"
  echo
  echo "## Summary"
  echo "- Generated at: ${GENERATED_AT}"
  echo "- Report month: \`${REPORT_MONTH}\`"
  echo "- Baseline report: ${BASELINE_LABEL}"
  echo "- Hotspots tracked: ${hotspot_count}"
  echo "- Boundary checks tracked: ${boundary_count}"
  echo "- SLO status: ${overall_status}"
  echo
  echo "## Hotspot Metrics"
  echo
  if [[ "$BASELINE_AVAILABLE" -eq 1 ]]; then
    echo "| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure | Prev Lines | Line Growth | Growth SLO | Prev Functions |"
    echo "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|---:|"
    while IFS='|' read -r key classes file lines max_lines line_headroom functions max_functions fn_headroom peak_usage pressure prev_lines line_growth growth_status prev_functions; do
      echo "| ${key} | \`${classes}\` | \`${file}\` | ${lines} | ${max_lines} | ${line_headroom} | ${functions} | ${max_functions} | ${fn_headroom} | ${peak_usage} | ${pressure} | ${prev_lines} | ${line_growth} | ${growth_status} | ${prev_functions} |"
    done <"$tmp_hotspots"
  else
    echo "| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure |"
    echo "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|"
    while IFS='|' read -r key classes file lines max_lines line_headroom functions max_functions fn_headroom peak_usage pressure _prev_lines _line_growth _growth_status _prev_functions; do
      echo "| ${key} | \`${classes}\` | \`${file}\` | ${lines} | ${max_lines} | ${line_headroom} | ${functions} | ${max_functions} | ${fn_headroom} | ${peak_usage} | ${pressure} |"
    done <"$tmp_hotspots"
  fi
  echo
  echo "## Prioritization Signals"
  echo "- BREACH hotspots (>100% of any tracked budget): ${breach_hotspot_summary}"
  echo "- TIGHT hotspots (>=95% of any tracked budget): ${tight_hotspot_summary}"
  echo "- WATCH hotspots (>=85% and <95% of any tracked budget): ${watch_hotspot_summary}"
  echo "- Mixed-class hotspots (size plus operational density): ${mixed_class_hotspot_summary}"
  echo
  echo "## Boundary Checks"
  echo
  echo "| Check | Status | Previous Status | Detail |"
  echo "|---|---|---|---|"
  while IFS='|' read -r key status previous_status detail; do
    echo "| ${key} | ${status} | ${previous_status} | ${detail} |"
  done <"$tmp_boundaries"
  echo
  echo "## SLO Assessment"
  if [[ "$hotspot_breach" -eq 1 ]]; then
    echo "- Hotspot growth SLO (>10% month-over-month): FAIL"
  else
    echo "- Hotspot growth SLO (>10% month-over-month): PASS"
  fi
  if [[ "$boundary_breach" -eq 1 ]]; then
    echo "- Boundary ownership SLO (helpers stay behind their module boundaries): FAIL"
  else
    echo "- Boundary ownership SLO (helpers stay behind their module boundaries): PASS"
  fi
  echo "- Overall architecture SLO status: ${overall_status}"
  echo
  echo "## Refactor Budget Policy"
  echo "- Monthly drift report command: \`scripts/generate_architecture_drift_report.sh\`"
  echo "- Release checklist budget field lives in \`docs/releases/TEMPLATE.md\`."
  echo "- Rule: each release must name at least one hotspot metric paid down or explicitly state why no paydown happened."
  echo
  echo "## Detail Links"
  echo "- [Architecture gate](../../scripts/check_architecture_boundaries.sh)"
  echo "- [Release template](TEMPLATE.md)"
  echo "- [CI workflow](../../.github/workflows/ci.yml)"
  echo
  while IFS='|' read -r key _classes _file lines _max_lines _line_headroom functions _max_functions _fn_headroom _peak_usage _pressure _prev_lines _line_growth _growth_status _prev_functions; do
    echo "<!-- arch-hotspot key=${key} lines=${lines} functions=${functions} -->"
  done <"$tmp_hotspots"
  while IFS='|' read -r key status _previous_status _detail; do
    echo "<!-- arch-boundary key=${key} status=${status} -->"
  done <"$tmp_boundaries"
} >"$OUTPUT_PATH"

echo "[arch-drift] wrote ${OUTPUT_PATH}"
