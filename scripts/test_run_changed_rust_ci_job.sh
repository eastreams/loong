#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_SOURCE="$REPO_ROOT/scripts/run_changed_rust_ci_job.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq -- "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_not_contains() {
  local file="$1"
  local needle="$2"
  if grep -Fq -- "$needle" "$file"; then
    echo "did not expect to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

make_fixture_repo() {
  local fixture_root="$1"
  mkdir -p "$fixture_root/scripts" "$fixture_root/stub"
  cp "$SCRIPT_SOURCE" "$fixture_root/scripts/run_changed_rust_ci_job.sh"
  chmod +x "$fixture_root/scripts/run_changed_rust_ci_job.sh"

  cat >"$fixture_root/scripts/test_changed_rust_packages.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'changed %s\n' "$*" >>"$FAKE_INVOCATION_LOG"
EOF
  chmod +x "$fixture_root/scripts/test_changed_rust_packages.sh"

  cat >"$fixture_root/stub/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'cargo %s\n' "$*" >>"$FAKE_INVOCATION_LOG"
EOF
  chmod +x "$fixture_root/stub/cargo"
}

run_pull_request_fast_lane_test() {
  local tmp_dir fixture_root invocation_log
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  fixture_root="$tmp_dir/repo"
  invocation_log="$tmp_dir/invocations.log"

  : >"$invocation_log"
  make_fixture_repo "$fixture_root"

  (
    cd "$fixture_root"
    PATH="$fixture_root/stub:$PATH" \
      FAKE_INVOCATION_LOG="$invocation_log" \
      GITHUB_EVENT_NAME=pull_request \
      TOUCHED_PATHS_JSON='["crates/daemon/src/gateway/openai_compat.rs","Taskfile.yml"]' \
      ./scripts/run_changed_rust_ci_job.sh --all-features
  )

  assert_contains "$invocation_log" "changed --all-features crates/daemon/src/gateway/openai_compat.rs Taskfile.yml"
  assert_not_contains "$invocation_log" "cargo test --workspace"
}

run_non_pr_full_workspace_test() {
  local tmp_dir fixture_root invocation_log
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  fixture_root="$tmp_dir/repo"
  invocation_log="$tmp_dir/invocations.log"

  : >"$invocation_log"
  make_fixture_repo "$fixture_root"

  (
    cd "$fixture_root"
    PATH="$fixture_root/stub:$PATH" \
      FAKE_INVOCATION_LOG="$invocation_log" \
      GITHUB_EVENT_NAME=push \
      ./scripts/run_changed_rust_ci_job.sh
  )

  assert_contains "$invocation_log" "cargo test --workspace --locked"
  assert_not_contains "$invocation_log" "changed "
}

run_pull_request_fast_lane_test
run_non_pr_full_workspace_test

echo "run_changed_rust_ci_job.sh harness checks passed"
