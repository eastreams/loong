#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

make_fixture_repo() {
  local fixture
  fixture="$(mktemp -d)"

  mkdir -p \
    "$fixture/scripts" \
    "$fixture/crates/spec/src" \
    "$fixture/crates/spec" \
    "$fixture/crates/app/src/provider" \
    "$fixture/crates/app/src/memory"

  cp "$REPO_ROOT/scripts/architecture_budget_lib.sh" "$fixture/scripts/architecture_budget_lib.sh"
  cp "$REPO_ROOT/scripts/check_architecture_boundaries.sh" "$fixture/scripts/check_architecture_boundaries.sh"
  cp "$REPO_ROOT/scripts/generate_architecture_drift_report.sh" "$fixture/scripts/generate_architecture_drift_report.sh"
  chmod +x \
    "$fixture/scripts/architecture_budget_lib.sh" \
    "$fixture/scripts/check_architecture_boundaries.sh" \
    "$fixture/scripts/generate_architecture_drift_report.sh"

  cp "$REPO_ROOT/crates/spec/src/spec_runtime.rs" "$fixture/crates/spec/src/spec_runtime.rs"
  cp "$REPO_ROOT/crates/spec/src/spec_execution.rs" "$fixture/crates/spec/src/spec_execution.rs"
  cp "$REPO_ROOT/crates/spec/Cargo.toml" "$fixture/crates/spec/Cargo.toml"
  cp "$REPO_ROOT/crates/app/src/provider/mod.rs" "$fixture/crates/app/src/provider/mod.rs"
  cp "$REPO_ROOT/crates/app/src/memory/mod.rs" "$fixture/crates/app/src/memory/mod.rs"

  printf '%s\n' "$fixture"
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

run_check_fails_on_missing_hotspot_test
run_report_fails_on_missing_hotspot_test

echo "architecture budget script checks passed"
