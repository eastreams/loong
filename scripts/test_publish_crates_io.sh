#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/publish_crates_io.sh"
PACKAGE_CHAIN=(
  loong-contracts
  loong-protocol
  loong-kernel
  loong-spec
  loong-bench
  loong-app
  loong
)

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

assert_lines_exact() {
  local file="$1"
  shift

  local expected_lines=("$@")
  local actual_lines=()
  local expected_count
  local actual_count
  local index
  local expected_line
  local actual_line
  local line

  while IFS= read -r line; do
    actual_lines+=("$line")
  done < "$file"

  expected_count="${#expected_lines[@]}"
  actual_count="${#actual_lines[@]}"

  if [[ "$actual_count" -ne "$expected_count" ]]; then
    echo "expected $expected_count lines in $file but found $actual_count" >&2
    cat "$file" >&2
    exit 1
  fi

  for index in "${!expected_lines[@]}"; do
    expected_line="${expected_lines[$index]}"
    actual_line="${actual_lines[$index]}"

    if [[ "$actual_line" != "$expected_line" ]]; then
      echo "expected line $((index + 1)) to be '$expected_line' but found '$actual_line'" >&2
      cat "$file" >&2
      exit 1
    fi
  done
}

assert_publish_sequence() {
  local file="$1"
  local mode="$2"
  shift 2

  local expected_lines=()
  local pkg
  local line

  for pkg in "$@"; do
    line="publish -p $pkg --locked"
    if [[ "$mode" == "dry-run" ]]; then
      line="publish --dry-run -p $pkg --locked"
    fi
    expected_lines+=("$line")
  done

  assert_lines_exact "$file" "${expected_lines[@]}"
}

make_fake_cargo() {
  local stub_dir="$1"
  local invocation_log="$2"
  cat >"$stub_dir/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >>"$FAKE_CARGO_INVOCATION_LOG"
EOF
  chmod +x "$stub_dir/cargo"
}

run_default_dry_run_test() {
  local tmp_dir stub_dir invocation_log output_file
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  stub_dir="$tmp_dir/stub"
  mkdir -p "$stub_dir"
  invocation_log="$tmp_dir/invocations.log"
  output_file="$tmp_dir/output.txt"

  : >"$invocation_log"
  make_fake_cargo "$stub_dir" "$invocation_log"

  PATH="$stub_dir:$PATH" \
    FAKE_CARGO_INVOCATION_LOG="$invocation_log" \
    bash "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1

  assert_publish_sequence "$invocation_log" "dry-run" "${PACKAGE_CHAIN[@]}"
  assert_contains "$output_file" "mode: dry-run"
}

run_publish_mode_test() {
  local tmp_dir stub_dir invocation_log output_file
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  stub_dir="$tmp_dir/stub"
  mkdir -p "$stub_dir"
  invocation_log="$tmp_dir/invocations.log"
  output_file="$tmp_dir/output.txt"

  : >"$invocation_log"
  make_fake_cargo "$stub_dir" "$invocation_log"

  PATH="$stub_dir:$PATH" \
    FAKE_CARGO_INVOCATION_LOG="$invocation_log" \
    bash "$SCRIPT_UNDER_TEST" --publish >"$output_file" 2>&1

  assert_publish_sequence "$invocation_log" "publish" "${PACKAGE_CHAIN[@]}"
  assert_contains "$output_file" "mode: publish"
}

run_resume_from_test() {
  local tmp_dir stub_dir invocation_log output_file
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  stub_dir="$tmp_dir/stub"
  mkdir -p "$stub_dir"
  invocation_log="$tmp_dir/invocations.log"
  output_file="$tmp_dir/output.txt"

  : >"$invocation_log"
  make_fake_cargo "$stub_dir" "$invocation_log"

  PATH="$stub_dir:$PATH" \
    FAKE_CARGO_INVOCATION_LOG="$invocation_log" \
    bash "$SCRIPT_UNDER_TEST" --from loong-spec >"$output_file" 2>&1

  assert_publish_sequence \
    "$invocation_log" \
    "dry-run" \
    loong-spec \
    loong-bench \
    loong-app \
    loong
  assert_contains "$output_file" "starting from: loong-spec"
}

run_default_dry_run_test
run_publish_mode_test
run_resume_from_test

echo "publish_crates_io.sh harness checks passed"
