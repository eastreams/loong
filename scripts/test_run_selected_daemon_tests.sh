#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_SOURCE="$REPO_ROOT/scripts/run_selected_daemon_tests.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq -- "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

make_fake_cargo() {
  local stub_dir="$1"
  cat >"$stub_dir/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf 'cargo %s\n' "$*" >>"$FAKE_INVOCATION_LOG"

build_dir="${FAKE_BUILD_DIR}"
mkdir -p "$build_dir"

targets=()
previous=""
for arg in "$@"; do
  if [[ "$previous" == "--test" ]]; then
    targets+=("$arg")
  fi
  previous="$arg"
done

if [[ " $* " == *" --no-run "* ]]; then
  if [[ " $* " == *" --lib "* ]]; then
    binary_path="$build_dir/loong_daemon_lib"
    cat >"$binary_path" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'binary loong_daemon_lib %s\n' "$*" >>"$FAKE_INVOCATION_LOG"
STUB
    chmod +x "$binary_path"
    python3 - <<PY
import json
print(json.dumps({
    "reason": "compiler-artifact",
    "target": {"name": "loong_daemon", "kind": ["lib"]},
    "executable": "$binary_path",
}))
PY
  fi

  if [[ " $* " == *" --bins "* ]]; then
    binary_path="$build_dir/loong_bin"
    cat >"$binary_path" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'binary loong_bin %s\n' "$*" >>"$FAKE_INVOCATION_LOG"
STUB
    chmod +x "$binary_path"
    python3 - <<PY
import json
print(json.dumps({
    "reason": "compiler-artifact",
    "target": {"name": "loong", "kind": ["bin"]},
    "executable": "$binary_path",
}))
PY
  fi

  for target in "${targets[@]}"; do
    binary_path="$build_dir/$target"
    cat >"$binary_path" <<STUB
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == "--list" ]]; then
  if [[ "$target" == "daemon_smoke" ]]; then
    cat <<'LIST'
integration::alpha::test_one: test
integration::alpha::test_two: test
integration::beta::test_three: test
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
  chmod +x "$stub_dir/cargo"
}

run_batch_compile_and_execution_test() {
  local tmp_dir fixture_root stub_dir invocation_log build_dir output_file
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN
  fixture_root="$tmp_dir/repo"
  stub_dir="$fixture_root/stub"
  invocation_log="$tmp_dir/invocations.log"
  build_dir="$tmp_dir/build"
  output_file="$tmp_dir/output.txt"
  mkdir -p "$fixture_root/scripts" "$stub_dir"

  cp "$SCRIPT_SOURCE" "$fixture_root/scripts/run_selected_daemon_tests.sh"
  chmod +x "$fixture_root/scripts/run_selected_daemon_tests.sh"

  : >"$invocation_log"
  make_fake_cargo "$stub_dir"

  cat >"$stub_dir/python" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
python3 "$@" | python3 -c 'import sys; sys.stdout.write(sys.stdin.read().replace("\n", "\r\n"))'
EOF
  chmod +x "$stub_dir/python"

  cat >"$fixture_root/scripts/cargo-local-toolchain.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
exec cargo "$@"
EOF
  chmod +x "$fixture_root/scripts/cargo-local-toolchain.sh"

  (
    cd "$fixture_root"
    PATH="$stub_dir:$PATH" \
      PYTHON_BIN=python \
      FAKE_INVOCATION_LOG="$invocation_log" \
      FAKE_BUILD_DIR="$build_dir" \
      ./scripts/run_selected_daemon_tests.sh --all-features --include-lib-bins daemon_smoke daemon_feishu >"$output_file" 2>&1
  )

  assert_contains "$invocation_log" "cargo test --locked -p loong --all-features --lib --bins --test daemon_smoke --test daemon_feishu --no-run --message-format=json"
  assert_contains "$invocation_log" "binary loong_daemon_lib "
  assert_contains "$invocation_log" "binary loong_bin "
  assert_contains "$invocation_log" "binary daemon_smoke integration::alpha::"
  assert_contains "$invocation_log" "binary daemon_smoke integration::beta::"
  assert_contains "$invocation_log" "binary daemon_feishu "
}

run_batch_compile_and_execution_test

echo "run_selected_daemon_tests.sh harness checks passed"
