#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/check_architecture_drift_freshness.sh"

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

make_fixture_repo() {
  local fixture
  fixture="$(mktemp -d)"

  mkdir -p \
    "$fixture/scripts" \
    "$fixture/crates/app/src" \
    "$fixture/crates/spec/src" \
    "$fixture/crates/spec" \
    "$fixture/crates/daemon/src" \
    "$fixture/docs/releases"

  cp "$REPO_ROOT/scripts/architecture_budget_lib.sh" "$fixture/scripts/architecture_budget_lib.sh"
  cp "$REPO_ROOT/scripts/generate_architecture_drift_report.sh" "$fixture/scripts/generate_architecture_drift_report.sh"
  cp "$SCRIPT_UNDER_TEST" "$fixture/scripts/check_architecture_drift_freshness.sh"
  chmod +x \
    "$fixture/scripts/architecture_budget_lib.sh" \
    "$fixture/scripts/generate_architecture_drift_report.sh" \
    "$fixture/scripts/check_architecture_drift_freshness.sh"

  copy_hotspot_fixture_files "$fixture"
  copy_boundary_fixture_files "$fixture"

  (
    cd "$fixture"
    git init -q
    git config user.name "Codex Test"
    git config user.email "codex@example.com"
    git add .
    git commit -qm "seed source inputs"
  )

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

run_fresh_report_passes_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local report_file="$fixture/docs/releases/architecture-drift-2099-01.md"
  (
    cd "$fixture"
    LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
      scripts/generate_architecture_drift_report.sh "$report_file"
    git add "$report_file"
    git commit -qm "seed fresh architecture drift report"
  )

  local output_file="$fixture/fresh.out"
  (
    cd "$fixture"
    LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
      scripts/check_architecture_drift_freshness.sh "$report_file" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "tracked architecture drift report is fresh"
}

run_fresh_report_with_adjacent_baseline_passes_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local baseline_file="$fixture/docs/releases/architecture-drift-2098-12.md"
  local report_file="$fixture/docs/releases/architecture-drift-2099-01.md"

  (
    cd "$fixture"
    LOONGCLAW_ARCH_REPORT_MONTH="2098-12" \
      scripts/generate_architecture_drift_report.sh "$baseline_file"
    LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
      scripts/generate_architecture_drift_report.sh "$report_file"
    git add "$baseline_file"
    git add "$report_file"
    git commit -qm "seed fresh architecture drift reports with baseline"
  )

  local output_file="$fixture/fresh-with-baseline.out"
  (
    cd "$fixture"
    LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
      scripts/check_architecture_drift_freshness.sh "$report_file" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "tracked architecture drift report is fresh"
}

run_stale_report_fails_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local report_file="$fixture/docs/releases/architecture-drift-2099-01.md"
  (
    cd "$fixture"
    LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
      scripts/generate_architecture_drift_report.sh "$report_file"
    git add "$report_file"
    git commit -qm "seed stale architecture drift report"
  )

  printf '\nmanual drift\n' >>"$report_file"
  (
    cd "$fixture"
    git add "$report_file"
    git commit -qm "record stale tracked architecture drift report"
  )

  local output_file="$fixture/stale.out"
  if (
    cd "$fixture" &&
      LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
        scripts/check_architecture_drift_freshness.sh "$report_file" >"$output_file" 2>&1
  ); then
    echo "expected freshness check to fail when the tracked report drifts from generated output" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "stale tracked architecture drift report"
}

run_untracked_report_fails_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local report_file="$fixture/docs/releases/architecture-drift-2099-01.md"
  local output_file="$fixture/untracked.out"
  if (
    cd "$fixture" &&
      LOONGCLAW_ARCH_REPORT_MONTH="2099-01" \
        scripts/check_architecture_drift_freshness.sh "$report_file" >"$output_file" 2>&1
  ); then
    echo "expected freshness check to fail when the report path is not tracked by git" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "must already be tracked by git"
}

run_fresh_report_passes_test
run_fresh_report_with_adjacent_baseline_passes_test
run_stale_report_fails_test
run_untracked_report_fails_test

echo "check_architecture_drift_freshness.sh checks passed"
