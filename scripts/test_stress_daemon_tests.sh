#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/stress_daemon_tests.sh"

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

make_fake_cargo() {
  local stub_dir="$1"
  local behavior_file="$2"
  local invocation_log="$3"
  cat >"$stub_dir/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

behavior_file="$FAKE_CARGO_BEHAVIOR_FILE"
invocation_log="$FAKE_CARGO_INVOCATION_LOG"
mode="default"
for arg in "$@"; do
  case "$arg" in
    --test-threads=*)
      mode="${arg#--test-threads=}"
      ;;
  esac
done
trap_mode="${LOONG_WASM_SIGNALS_BASED_TRAPS:-auto}"
printf 'trap=%s mode=%s args=%s\n' "$trap_mode" "$mode" "$*" >>"$invocation_log"
if grep -Fxq "trap=${trap_mode} mode=${mode}" "$behavior_file"; then
  echo "simulated failure trap=${trap_mode} mode=${mode}" >&2
  exit 1
fi
echo "simulated success trap=${trap_mode} mode=${mode}"
EOF
  chmod +x "$stub_dir/cargo"
}

run_fail_fast_test() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  local stub_dir="$tmp_dir/stub"
  mkdir -p "$stub_dir"
  local behavior_file="$tmp_dir/failures.txt"
  local invocation_log="$tmp_dir/invocations.log"
  local log_dir="$tmp_dir/logs"
  local output_file="$tmp_dir/output.txt"

  cat >"$behavior_file" <<'EOF'
trap=auto mode=2
EOF
  : >"$invocation_log"
  make_fake_cargo "$stub_dir" "$behavior_file" "$invocation_log"

  local rc=0
  if PATH="$stub_dir:$PATH" \
    FAKE_CARGO_BEHAVIOR_FILE="$behavior_file" \
    FAKE_CARGO_INVOCATION_LOG="$invocation_log" \
    "$SCRIPT_UNDER_TEST" 1 "default,2,1" "$log_dir" >"$output_file" 2>&1; then
    rc=0
  else
    rc=$?
  fi

  if [[ "$rc" -eq 0 ]]; then
    echo "expected fail-fast run to fail" >&2
    cat "$output_file" >&2
    exit 1
  fi

  local summary_file="$log_dir/summary.txt"
  [[ -f "$summary_file" ]] || {
    echo "expected summary file $summary_file" >&2
    exit 1
  }

  assert_contains "$summary_file" "[stress] overall status=FAIL"
  assert_contains "$summary_file" "[stress] mode-result traps=auto mode=default status=PASS"
  assert_contains "$summary_file" "[stress] mode-result traps=auto mode=2 status=FAIL"
  assert_not_contains "$summary_file" "mode=1"

  assert_contains "$invocation_log" "trap=auto mode=default"
  assert_contains "$invocation_log" "trap=auto mode=2"
  assert_not_contains "$invocation_log" "trap=auto mode=1"
}

run_continue_on_failure_test() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  local stub_dir="$tmp_dir/stub"
  mkdir -p "$stub_dir"
  local behavior_file="$tmp_dir/failures.txt"
  local invocation_log="$tmp_dir/invocations.log"
  local log_dir="$tmp_dir/logs"
  local output_file="$tmp_dir/output.txt"

  cat >"$behavior_file" <<'EOF'
trap=true mode=2
EOF
  : >"$invocation_log"
  make_fake_cargo "$stub_dir" "$behavior_file" "$invocation_log"

  local rc=0
  if PATH="$stub_dir:$PATH" \
    FAKE_CARGO_BEHAVIOR_FILE="$behavior_file" \
    FAKE_CARGO_INVOCATION_LOG="$invocation_log" \
    LOONG_STRESS_CONTINUE_ON_FAILURE=true \
    "$SCRIPT_UNDER_TEST" 1 "default,2,1" "$log_dir" "auto,true" >"$output_file" 2>&1; then
    rc=0
  else
    rc=$?
  fi

  if [[ "$rc" -eq 0 ]]; then
    echo "expected continue-on-failure run to exit non-zero after collecting failures" >&2
    cat "$output_file" >&2
    exit 1
  fi

  local summary_file="$log_dir/summary.txt"
  [[ -f "$summary_file" ]] || {
    echo "expected summary file $summary_file" >&2
    exit 1
  }

  assert_contains "$summary_file" "[stress] overall status=FAIL"
  assert_contains "$summary_file" "[stress] mode-result traps=true mode=2 status=FAIL"
  assert_contains "$summary_file" "[stress] mode-result traps=true mode=1 status=PASS"
  assert_contains "$summary_file" "[stress] mode-result traps=auto mode=1 status=PASS"

  assert_contains "$invocation_log" "trap=true mode=2"
  assert_contains "$invocation_log" "trap=true mode=1"
}

run_all_pass_test() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  local stub_dir="$tmp_dir/stub"
  mkdir -p "$stub_dir"
  local behavior_file="$tmp_dir/failures.txt"
  local invocation_log="$tmp_dir/invocations.log"
  local log_dir="$tmp_dir/logs"
  local output_file="$tmp_dir/output.txt"

  : >"$behavior_file"
  : >"$invocation_log"
  make_fake_cargo "$stub_dir" "$behavior_file" "$invocation_log"

  PATH="$stub_dir:$PATH" \
    FAKE_CARGO_BEHAVIOR_FILE="$behavior_file" \
    FAKE_CARGO_INVOCATION_LOG="$invocation_log" \
    "$SCRIPT_UNDER_TEST" 1 "default,1" "$log_dir" "auto" >"$output_file" 2>&1

  local summary_file="$log_dir/summary.txt"
  [[ -f "$summary_file" ]] || {
    echo "expected summary file $summary_file" >&2
    exit 1
  }

  assert_contains "$summary_file" "[stress] overall status=PASS"
  assert_contains "$summary_file" "[stress] mode-result traps=auto mode=default status=PASS"
  assert_contains "$summary_file" "[stress] mode-result traps=auto mode=1 status=PASS"
}

run_fail_fast_test
run_continue_on_failure_test
run_all_pass_test

echo "stress_daemon_tests.sh harness checks passed"
