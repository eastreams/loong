#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/check_glibc_floor.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

make_readelf_stub_bin() {
  local fixture="$1"
  local readelf_output="$2"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/readelf" <<EOF
#!/usr/bin/env bash
set -euo pipefail

if [[ "\${1:-}" != "--version-info" || "\${2:-}" != "--wide" ]]; then
  echo "unexpected readelf invocation: \$*" >&2
  exit 1
fi

cat <<'OUT'
$readelf_output
OUT
EOF
  chmod +x "$fixture/fake-bin/readelf"
}

run_accepts_supported_glibc_floor_test() {
  local fixture output_file bin_path
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/check.out"
  bin_path="$fixture/loongclaw"
  : >"$bin_path"
  make_readelf_stub_bin "$fixture" $'Version needs section \'.gnu.version_r\' contains 1 entry:\n  0x0010:   Name: GLIBC_2.17  Flags: none  Version: 4\n  0x0020:   Name: GLIBC_2.4  Flags: none  Version: 3'

  PATH="$fixture/fake-bin:$PATH" \
    bash "$SCRIPT_UNDER_TEST" "$bin_path" "2.17" >"$output_file" 2>&1

  assert_contains "$output_file" "Max required GLIBC: 2.17"
}

run_rejects_newer_glibc_requirement_test() {
  local fixture output_file bin_path
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/check.out"
  bin_path="$fixture/loongclaw"
  : >"$bin_path"
  make_readelf_stub_bin "$fixture" $'Version needs section \'.gnu.version_r\' contains 1 entry:\n  0x0010:   Name: GLIBC_2.18  Flags: none  Version: 4'

  if PATH="$fixture/fake-bin:$PATH" \
    bash "$SCRIPT_UNDER_TEST" "$bin_path" "2.17" >"$output_file" 2>&1; then
    echo "expected check_glibc_floor.sh to fail for unsupported GLIBC floor" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "Binary requires GLIBC_2.18"
}

run_rejects_missing_glibc_versions_test() {
  local fixture output_file bin_path
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/check.out"
  bin_path="$fixture/loongclaw"
  : >"$bin_path"
  make_readelf_stub_bin "$fixture" "no version references found"

  if PATH="$fixture/fake-bin:$PATH" \
    bash "$SCRIPT_UNDER_TEST" "$bin_path" "2.17" >"$output_file" 2>&1; then
    echo "expected check_glibc_floor.sh to fail when no GLIBC symbols are present" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "failed to detect required GLIBC version"
}

run_accepts_supported_glibc_floor_test
run_rejects_newer_glibc_requirement_test
run_rejects_missing_glibc_versions_test

echo "check_glibc_floor.sh checks passed"
