#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

. "$REPO_ROOT/scripts/architecture_budget_lib.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_equals() {
  local expected="$1"
  local actual="$2"

  if [[ "$expected" != "$actual" ]]; then
    echo "expected '$expected' but got '$actual'" >&2
    exit 1
  fi
}

make_fixture_repo() {
  local fixture
  fixture="$(mktemp -d)"

  mkdir -p \
    "$fixture/scripts" \
    "$fixture/crates/app/src" \
    "$fixture/crates/spec/src" \
    "$fixture/crates/spec" \
    "$fixture/crates/daemon/src"

  cp "$REPO_ROOT/scripts/architecture_budget_lib.sh" "$fixture/scripts/architecture_budget_lib.sh"
  cp "$REPO_ROOT/scripts/check_architecture_boundaries.sh" "$fixture/scripts/check_architecture_boundaries.sh"
  cp "$REPO_ROOT/scripts/generate_architecture_drift_report.sh" "$fixture/scripts/generate_architecture_drift_report.sh"
  chmod +x \
    "$fixture/scripts/architecture_budget_lib.sh" \
    "$fixture/scripts/check_architecture_boundaries.sh" \
    "$fixture/scripts/generate_architecture_drift_report.sh"

  copy_hotspot_fixture_files "$fixture"
  copy_boundary_fixture_files "$fixture"

  printf '%s\n' "$fixture"
}

copy_hotspot_fixture_files() {
  local fixture="$1"
  local key spec file

  while IFS= read -r key; do
    [[ -z "$key" ]] && continue
    spec="$(architecture_hotspot_spec "$key")" || exit 1
    IFS='|' read -r file _max_lines _max_functions <<EOF_SPEC
$spec
EOF_SPEC
    mkdir -p "$fixture/$(dirname "$file")"
    cp "$REPO_ROOT/$file" "$fixture/$file"
  done <<EOF_KEYS
$(architecture_hotspot_keys)
EOF_KEYS
}

copy_boundary_fixture_files() {
  local fixture="$1"

  mkdir -p \
    "$fixture/crates/spec" \
    "$fixture/crates/app/src/provider" \
    "$fixture/crates/app/src/memory" \
    "$fixture/crates/app/src/conversation"

  cp "$REPO_ROOT/crates/spec/Cargo.toml" "$fixture/crates/spec/Cargo.toml"
  cp "$REPO_ROOT/crates/app/src/provider/mod.rs" "$fixture/crates/app/src/provider/mod.rs"
  cp "$REPO_ROOT/crates/app/src/memory/mod.rs" "$fixture/crates/app/src/memory/mod.rs"
  cp "$REPO_ROOT/crates/app/src/conversation/runtime.rs" "$fixture/crates/app/src/conversation/runtime.rs"
}

run_hotspot_metadata_helpers_test() {
  local hotspot_spec
  local hotspot_classes

  hotspot_spec="$(architecture_hotspot_spec "turn_coordinator")"
  hotspot_classes="$(architecture_hotspot_classes "turn_coordinator")"

  assert_equals \
    "crates/app/src/conversation/turn_coordinator.rs|11200|120" \
    "$hotspot_spec"
  assert_equals \
    "structural_size,operational_density" \
    "$hotspot_classes"
}

run_hotspot_pressure_helpers_test() {
  local healthy_pressure
  local watch_pressure
  local tight_pressure
  local breach_pressure
  local peak_usage

  healthy_pressure="$(architecture_hotspot_pressure 40 100 10 100)"
  watch_pressure="$(architecture_hotspot_pressure 84 100 85 100)"
  tight_pressure="$(architecture_hotspot_pressure 95 100 10 100)"
  breach_pressure="$(architecture_hotspot_pressure 101 100 10 100)"
  peak_usage="$(architecture_hotspot_peak_usage_percent 80 100 92 100)"

  assert_equals "HEALTHY" "$healthy_pressure"
  assert_equals "WATCH" "$watch_pressure"
  assert_equals "TIGHT" "$tight_pressure"
  assert_equals "BREACH" "$breach_pressure"
  assert_equals "92.0%" "$peak_usage"
}

run_check_fails_on_missing_hotspot_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  rm "$fixture/crates/spec/src/spec_runtime.rs"

  local output_file="$fixture/check.out"
  if (
    cd "$fixture" &&
      LOONGCLAW_ARCH_STRICT=true scripts/check_architecture_boundaries.sh >"$output_file" 2>&1
  ); then
    echo "expected architecture boundary check to fail when a tracked hotspot file is missing" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "missing hotspot file"
  assert_contains "$output_file" "crates/spec/src/spec_runtime.rs"
}

run_report_fails_on_missing_hotspot_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  rm "$fixture/crates/spec/src/spec_runtime.rs"

  local report_file="$fixture/architecture-drift-2099-01.md"
  local output_file="$fixture/report.out"
  if (
    cd "$fixture" &&
      LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
        scripts/generate_architecture_drift_report.sh "$report_file" >"$output_file" 2>&1
  ); then
    echo "expected architecture drift report generation to fail when a tracked hotspot file is missing" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "missing hotspot file"
  assert_contains "$output_file" "crates/spec/src/spec_runtime.rs"
}

run_hotspot_metadata_helpers_test
run_hotspot_pressure_helpers_test
run_check_fails_on_missing_hotspot_test
run_report_fails_on_missing_hotspot_test

echo "architecture budget script checks passed"
