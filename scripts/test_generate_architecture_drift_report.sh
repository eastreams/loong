#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/generate_architecture_drift_report.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_not_contains() {
  local file="$1"
  local needle="$2"
  if grep -Fq "$needle" "$file"; then
    echo "did not expect to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_blank_line_after() {
  local file="$1"
  local heading="$2"
  local heading_line
  heading_line="$(grep -nF "$heading" "$file" | head -n 1 | cut -d: -f1)"
  if [[ -z "$heading_line" ]]; then
    echo "expected heading '$heading' in $file" >&2
    cat "$file" >&2
    exit 1
  fi

  local next_line_number
  next_line_number=$((heading_line + 1))
  local next_line
  next_line="$(sed -n "${next_line_number}p" "$file")"
  if [[ -n "$next_line" ]]; then
    echo "expected a blank line after '$heading' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

run_no_baseline_test() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  local output_file="$tmp_dir/architecture-drift-2099-01.md"
  LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
    "$SCRIPT_UNDER_TEST" "$output_file"

  [[ -f "$output_file" ]] || {
    echo "expected generated report at $output_file" >&2
    exit 1
  }

  assert_contains "$output_file" "# Architecture Drift Report 2099-01"
  assert_contains "$output_file" "SLO status: PASS"
  assert_contains "$output_file" "Baseline report: none"
  assert_contains "$output_file" "Hotspots tracked: 14"
  assert_contains "$output_file" "| Key | Classes | File |"
  assert_contains "$output_file" "| spec_runtime |"
  assert_contains "$output_file" "| channel_registry |"
  assert_contains "$output_file" "| turn_coordinator |"
  assert_contains "$output_file" "| onboard_cli |"
  assert_contains "$output_file" "| tools_mod |"
  assert_contains "$output_file" '`foundation`'
  assert_contains "$output_file" '`structural_size,operational_density`'
  assert_contains "$output_file" "## Prioritization Signals"
  assert_contains "$output_file" "BREACH hotspots (>100% of any tracked budget):"
  assert_contains "$output_file" "TIGHT hotspots (>=95% of any tracked budget)"
  assert_contains "$output_file" "WATCH hotspots (>=85% and <95% of any tracked budget)"
  assert_contains "$output_file" "Mixed-class hotspots (size plus operational density)"
  assert_not_contains "$output_file" "Prev Lines"
  assert_not_contains "$output_file" "Line Growth"
  assert_not_contains "$output_file" "Growth SLO"
  assert_not_contains "$output_file" "Prev Functions"
  assert_contains "$output_file" "turn_coordinator"
  assert_contains "$output_file" "channel_registry"
  assert_contains "$output_file" "<!-- arch-hotspot key=spec_runtime"
  assert_contains "$output_file" "<!-- arch-hotspot key=channel_registry"
  assert_contains "$output_file" "<!-- arch-hotspot key=tools_mod"
  assert_contains "$output_file" "<!-- arch-boundary key=memory_literals status=PASS -->"
  assert_contains "$output_file" "<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->"
  assert_contains "$output_file" "<!-- arch-boundary key=spec_app_dependency status=PASS -->"
  assert_blank_line_after "$output_file" "## Hotspot Metrics"
  assert_blank_line_after "$output_file" "## Boundary Checks"
}

run_breach_baseline_test() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  local baseline_file="$tmp_dir/architecture-drift-2098-12.md"
  cat >"$baseline_file" <<'BASELINE'
<!-- arch-hotspot key=spec_runtime lines=1 functions=1 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
BASELINE

  local output_file="$tmp_dir/architecture-drift-2099-01.md"
  LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
    LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT="$baseline_file" \
    "$SCRIPT_UNDER_TEST" "$output_file"

  [[ -f "$output_file" ]] || {
    echo "expected generated report at $output_file" >&2
    exit 1
  }

  assert_contains "$output_file" "Baseline report: $baseline_file"
  assert_contains "$output_file" "SLO status: FAIL"
  assert_contains "$output_file" "| spec_runtime |"
  assert_contains "$output_file" "| chat_runtime |"
  assert_contains "$output_file" "Prev Lines"
  assert_contains "$output_file" "Line Growth"
  assert_contains "$output_file" "Growth SLO"
  assert_contains "$output_file" "Prev Functions"
  assert_contains "$output_file" "## Prioritization Signals"
  assert_contains "$output_file" "BREACH"
}

run_no_baseline_test
run_breach_baseline_test

echo "generate_architecture_drift_report.sh checks passed"
