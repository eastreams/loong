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
  chmod +x \
    "$fixture/scripts/architecture_budget_lib.sh" \
    "$fixture/scripts/check_architecture_boundaries.sh"

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
    "$fixture/crates/app/src/conversation" \
    "$fixture/crates/loong-runtime/src/tool_plane"

  cp "$REPO_ROOT/crates/spec/Cargo.toml" "$fixture/crates/spec/Cargo.toml"
  cp "$REPO_ROOT/crates/app/src/provider/mod.rs" "$fixture/crates/app/src/provider/mod.rs"
  cp "$REPO_ROOT/crates/app/src/memory/mod.rs" "$fixture/crates/app/src/memory/mod.rs"
  cp "$REPO_ROOT/crates/app/src/conversation/runtime.rs" "$fixture/crates/app/src/conversation/runtime.rs"
  cp "$REPO_ROOT/crates/app/src/conversation/turn_engine.rs" \
    "$fixture/crates/app/src/conversation/turn_engine.rs"
  cp "$REPO_ROOT/crates/app/src/conversation/turn_coordinator.rs" \
    "$fixture/crates/app/src/conversation/turn_coordinator.rs"
  cp "$REPO_ROOT/crates/app/src/context.rs" "$fixture/crates/app/src/context.rs"
  cp "$REPO_ROOT/crates/loong-runtime/src/lib.rs" "$fixture/crates/loong-runtime/src/lib.rs"
  cp "$REPO_ROOT/crates/loong-runtime/src/tool_plane/invocation.rs" \
    "$fixture/crates/loong-runtime/src/tool_plane/invocation.rs"
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
      LOONG_ARCH_STRICT=true scripts/check_architecture_boundaries.sh >"$output_file" 2>&1
  ); then
    echo "expected architecture boundary check to fail when a tracked hotspot file is missing" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "missing hotspot file"
  assert_contains "$output_file" "crates/spec/src/spec_runtime.rs"
}

run_check_fails_on_missing_boundary_file_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  rm "$fixture/crates/app/src/context.rs"

  local output_file="$fixture/check-boundary.out"
  if (
    cd "$fixture" &&
      LOONG_ARCH_STRICT=true scripts/check_architecture_boundaries.sh >"$output_file" 2>&1
  ); then
    echo "expected architecture boundary check to fail when a tracked boundary file is missing" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "missing boundary file"
  assert_contains "$output_file" "crates/app/src/context.rs"
}

run_boundary_scan_rejects_legacy_typed_execution_types_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  cat <<'EOF_SIGNATURE' >>"$fixture/crates/app/src/context.rs"
pub struct AppContext;
type ForbiddenLegacyEvidence = CapabilityToken;
EOF_SIGNATURE

  local output_file="$fixture/boundary-hits.out"
  (
    cd "$fixture" &&
      architecture_typed_execution_legacy_type_hits >"$output_file"
  )

  assert_contains "$output_file" "context.rs"
  assert_contains "$output_file" "AppContext"
  assert_contains "$output_file" "CapabilityToken"
}

run_boundary_scan_rejects_runtime_protocol_contract_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  cat <<'EOF_CONTRACT' >>"$fixture/crates/loong-runtime/src/lib.rs"
pub struct RuntimeOneshotRequest;
EOF_CONTRACT

  local output_file="$fixture/runtime-boundary-hits.out"
  (
    cd "$fixture" &&
      architecture_runtime_root_transitional_contract_hits >"$output_file"
  )

  assert_contains "$output_file" "loong-runtime/src/lib.rs"
  assert_contains "$output_file" "RuntimeOneshotRequest"
}

run_boundary_scan_rejects_external_action_execution_audit_writer_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  cat <<'EOF_CALLER' >>"$fixture/crates/app/src/context.rs"
fn invalid_audit_owner(kernel: &Kernel, grant: &Granted<Action>) {
    kernel.record_granted_action_execution(grant, ActionExecutionEvent::Started);
}
EOF_CALLER

  local output_file="$fixture/action-execution-owner-hits.out"
  (
    cd "$fixture" &&
      architecture_typed_action_execution_audit_owner_hits >"$output_file"
  )

  assert_contains "$output_file" "context.rs"
  assert_contains "$output_file" "record_granted_action_execution"
}

run_hotspot_metadata_helpers_test
run_hotspot_pressure_helpers_test
run_check_fails_on_missing_hotspot_test
run_check_fails_on_missing_boundary_file_test
run_boundary_scan_rejects_legacy_typed_execution_types_test
run_boundary_scan_rejects_runtime_protocol_contract_test
run_boundary_scan_rejects_external_action_execution_audit_writer_test

echo "architecture budget script checks passed"
