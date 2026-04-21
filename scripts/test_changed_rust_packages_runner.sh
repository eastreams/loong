#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_SOURCE="$REPO_ROOT/scripts/test_changed_rust_packages.sh"
DAEMON_RUNNER_SOURCE="$REPO_ROOT/scripts/run_selected_daemon_tests.sh"

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
  local selected_packages_json="$2"
  local daemon_targets="$3"

  mkdir -p "$fixture_root/scripts"
  cp "$SCRIPT_SOURCE" "$fixture_root/scripts/test_changed_rust_packages.sh"
  cp "$DAEMON_RUNNER_SOURCE" "$fixture_root/scripts/run_selected_daemon_tests.sh"
  chmod +x "$fixture_root/scripts/test_changed_rust_packages.sh"
  chmod +x "$fixture_root/scripts/run_selected_daemon_tests.sh"

  cat >"$fixture_root/scripts/rust_changed_packages.py" <<EOF
#!/usr/bin/env python3
import json
import sys

packages = $selected_packages_json
if "--format" in sys.argv:
    format_index = sys.argv.index("--format")
    format_value = sys.argv[format_index + 1]
else:
    format_value = "names"

if format_value == "json":
    print(json.dumps({"selected": packages}))
else:
    for package in packages:
        print(package)
EOF
  chmod +x "$fixture_root/scripts/rust_changed_packages.py"

  cat >"$fixture_root/scripts/daemon_changed_test_targets.py" <<EOF
#!/usr/bin/env python3
print("""$daemon_targets""".strip())
EOF
  chmod +x "$fixture_root/scripts/daemon_changed_test_targets.py"

  cat >"$fixture_root/scripts/cargo-local-toolchain.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf 'cargo %s\n' "$*" >>"$FAKE_INVOCATION_LOG"

build_dir="${FAKE_BUILD_DIR}"
mkdir -p "$build_dir"

if [[ " $* " == *" --no-run "* ]]; then
  targets=()
  previous=""
  for arg in "$@"; do
    if [[ "$previous" == "--test" ]]; then
      targets+=("$arg")
    fi
    previous="$arg"
  done

  for target in "${targets[@]}"; do
    binary_path="$build_dir/$target"
    cat >"$binary_path" <<STUB
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == "--list" ]]; then
  if [[ "$target" == "daemon_smoke" ]]; then
    cat <<'LIST'
integration::alpha::case_one: test
integration::alpha::case_two: test
integration::beta::case_one: test
LIST
  fi
  exit 0
fi
printf 'binary $target %s\n' "\$*" >>"$FAKE_INVOCATION_LOG"
STUB
    chmod +x "$binary_path"
    python3 - <<PY
import json
print(json.dumps({
    "reason": "compiler-artifact",
    "target": {"name": "$target"},
    "executable": "$binary_path",
}))
PY
  done
fi
EOF
  chmod +x "$fixture_root/scripts/cargo-local-toolchain.sh"

}

run_explicit_paths_all_features_test() {
  local tmp_dir fixture_root invocation_log output_file
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  fixture_root="$tmp_dir/repo"
  invocation_log="$tmp_dir/invocations.log"
  output_file="$tmp_dir/output.txt"
  build_dir="$tmp_dir/build"

  : >"$invocation_log"
  make_fixture_repo "$fixture_root" '["loong"]' $'daemon_smoke\ndaemon_feishu'

  (
    cd "$fixture_root"
    FAKE_INVOCATION_LOG="$invocation_log" \
      FAKE_BUILD_DIR="$build_dir" \
      ./scripts/test_changed_rust_packages.sh --all-features crates/daemon/src/gateway/openai_compat.rs >"$output_file" 2>&1
  )

  assert_contains "$output_file" "[test:changed] files:"
  assert_contains "$output_file" "crates/daemon/src/gateway/openai_compat.rs"
  assert_contains "$output_file" "[test:changed] daemon targets: daemon_smoke daemon_feishu"

  assert_contains "$invocation_log" "cargo test --locked -p loong --all-features --lib --bins --test daemon_smoke --test daemon_feishu --no-run --message-format=json"
  assert_contains "$invocation_log" "binary daemon_smoke integration::alpha::"
  assert_contains "$invocation_log" "binary daemon_smoke integration::beta::"
  assert_contains "$invocation_log" "binary daemon_feishu "
}

run_explicit_paths_mixed_package_test() {
  local tmp_dir fixture_root invocation_log output_file
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  fixture_root="$tmp_dir/repo"
  invocation_log="$tmp_dir/invocations.log"
  output_file="$tmp_dir/output.txt"
  build_dir="$tmp_dir/build"

  : >"$invocation_log"
  make_fixture_repo "$fixture_root" '["loong-app", "loong"]' 'daemon_feishu'

  (
    cd "$fixture_root"
    FAKE_INVOCATION_LOG="$invocation_log" \
      FAKE_BUILD_DIR="$build_dir" \
      ./scripts/test_changed_rust_packages.sh crates/app/src/lib.rs >"$output_file" 2>&1
  )

  assert_contains "$output_file" "[test:changed] packages: loong-app loong"
  assert_contains "$invocation_log" "cargo test --locked -p loong-app --lib --tests"
  assert_contains "$invocation_log" "cargo test --locked -p loong --features test-support --lib --bins --test daemon_feishu --no-run --message-format=json"
  assert_contains "$invocation_log" "binary daemon_feishu "
  assert_not_contains "$invocation_log" "--all-features"
}

run_explicit_paths_all_features_test
run_explicit_paths_mixed_package_test

echo "test_changed_rust_packages.sh harness checks passed"
