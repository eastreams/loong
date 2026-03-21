#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/install.sh"
. "$REPO_ROOT/scripts/release_artifact_lib.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

sha256_file() {
  local file_path="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file_path" | awk '{print $1}'
    return 0
  fi
  sha256sum "$file_path" | awk '{print $1}'
}

host_target() {
  release_target_for_platform "$(uname -s)" "$(uname -m)"
}

make_release_fixture() {
  local fixture
  fixture="$(mktemp -d)"
  write_release_fixture_asset \
    "$fixture" \
    "${1:-v0.1.2}" \
    "${2:-$(host_target)}" \
    "${3:-fixture-binary}"
  printf '%s\n' "$fixture"
}

write_release_fixture_asset() {
  local fixture tag target binary_label archive_name checksum_name binary_name archive_path checksum_path release_dir staging_dir
  fixture="${1:?fixture is required}"
  tag="${2:-v0.1.2}"
  target="${3:-$(host_target)}"
  binary_label="${4:-fixture-binary}"
  archive_name="$(release_archive_name "loongclaw" "$tag" "$target")"
  checksum_name="$(release_archive_checksum_name "loongclaw" "$tag" "$target")"
  binary_name="$(release_binary_name_for_target "loongclaw" "$target")"
  release_dir="$fixture/releases/download/$tag"
  staging_dir="$fixture/staging/$target"
  mkdir -p "$release_dir" "$staging_dir"

  cat >"$staging_dir/$binary_name" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == "onboard" ]]; then
  printf 'onboard\n' >> "\${ONBOARD_MARKER:?}"
fi
printf '%s\n' "$binary_label"
EOF
  chmod +x "$staging_dir/$binary_name"

  archive_path="$release_dir/$archive_name"
  case "$archive_name" in
    *.tar.gz)
      tar -C "$staging_dir" -czf "$archive_path" "$binary_name"
      ;;
    *.zip)
      (cd "$staging_dir" && zip -q "$archive_path" "$binary_name")
      ;;
    *)
      echo "unsupported archive format in fixture: $archive_name" >&2
      exit 1
      ;;
  esac

  checksum_path="$release_dir/$checksum_name"
  printf '%s  %s\n' "$(sha256_file "$archive_path")" "$archive_name" >"$checksum_path"
}

make_linux_dual_libc_fixture() {
  local fixture tag
  fixture="$(mktemp -d)"
  tag="${1:-v0.1.2}"
  write_release_fixture_asset "$fixture" "$tag" "x86_64-unknown-linux-gnu" "gnu-binary"
  write_release_fixture_asset "$fixture" "$tag" "x86_64-unknown-linux-musl" "musl-binary"
  printf '%s\n' "$fixture"
}

make_latest_release_stub_bin() {
  local fixture="$1"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

url="${@: -1}"
if [[ "$url" == "https://api.github.com/repos/loongclaw-ai/loongclaw/releases/latest" ]]; then
  exit 22
fi

cat >&2 <<ERR
unexpected curl request: $url
ERR
exit 1
EOF
  chmod +x "$fixture/fake-bin/curl"
}

make_uname_stub_bin() {
  local fixture="$1"
  local stub_platform="$2"
  local stub_arch="$3"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/uname" <<EOF
#!/usr/bin/env bash
set -euo pipefail

case "\${1:-}" in
  -s) printf '%s\n' "$stub_platform" ;;
  -m) printf '%s\n' "$stub_arch" ;;
  *)
    echo "unexpected uname invocation: \$*" >&2
    exit 1
    ;;
esac
EOF
  chmod +x "$fixture/fake-bin/uname"
}

make_getconf_stub_bin() {
  local fixture="$1"
  local response="$2"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/getconf" <<EOF
#!/usr/bin/env bash
set -euo pipefail

if [[ "\${1:-}" != "GNU_LIBC_VERSION" ]]; then
  echo "unexpected getconf invocation: \$*" >&2
  exit 1
fi

if [[ "$response" == "__FAIL__" ]]; then
  exit 1
fi

printf '%s\n' "$response"
EOF
  chmod +x "$fixture/fake-bin/getconf"
}

make_ldd_stub_bin() {
  local fixture="$1"
  local response="$2"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/ldd" <<EOF
#!/usr/bin/env bash
set -euo pipefail

if [[ "$response" == "__FAIL__" ]]; then
  exit 1
fi

  printf '%s\n' "$response"
EOF
  chmod +x "$fixture/fake-bin/ldd"
}

make_sort_stub_bin() {
  local fixture="$1"
  local mode="$2"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/sort" <<EOF
#!/usr/bin/env bash
set -euo pipefail

if [[ "$mode" == "__FAIL_VERSION__" ]]; then
  for arg in "\$@"; do
    if [[ "\$arg" == "-V" || "\$arg" == "--version-sort" ]]; then
      echo "sort: unrecognized option '\$arg'" >&2
      exit 1
    fi
  done
fi

exec /usr/bin/sort "\$@"
EOF
  chmod +x "$fixture/fake-bin/sort"
}

source_install_functions() {
  local shim
  shim="$(mktemp)"
  awk '
    /^prefix="\$\{HOME\}\/\.local\/bin"/ { skip = 1; next }
    skip && !/^require_command\(\) \{/ { next }
    /^require_command\(\) \{/ { skip = 0 }
    /^if \[\[ "\$\{install_source\}" -eq 1 \]\]; then$/ { exit }
    { print }
  ' "$SCRIPT_UNDER_TEST" >"$shim"
  # shellcheck disable=SC1090
  . "$shim"
  rm -f "$shim"
}

run_linux_x86_64_prefers_gnu_when_glibc_is_supported_test() {
  local fixture install_dir output_file installed_output
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-gnu.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.39"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-gnu"
  installed_output="$("$install_dir/loongclaw")"
  if [[ "$installed_output" != "gnu-binary" ]]; then
    echo "expected GNU artifact to be installed but got '$installed_output'" >&2
    exit 1
  fi
}

run_linux_x86_64_falls_back_to_musl_when_glibc_is_too_old_test() {
  local fixture install_dir output_file installed_output
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-musl-old-glibc.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.36"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-musl"
  installed_output="$("$install_dir/loongclaw")"
  if [[ "$installed_output" != "musl-binary" ]]; then
    echo "expected musl fallback artifact to be installed but got '$installed_output'" >&2
    exit 1
  fi
}

run_linux_x86_64_falls_back_to_musl_when_glibc_detection_fails_test() {
  local fixture install_dir output_file installed_output
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-musl-no-glibc.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "__FAIL__"
  make_ldd_stub_bin "$fixture" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-musl"
  installed_output="$("$install_dir/loongclaw")"
  if [[ "$installed_output" != "musl-binary" ]]; then
    echo "expected musl fallback artifact to be installed but got '$installed_output'" >&2
    exit 1
  fi
}

run_linux_x86_64_explicit_musl_override_test() {
  local fixture install_dir output_file installed_output
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-musl-override.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.39"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      LOONGCLAW_INSTALL_TARGET_LIBC="musl" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-musl"
  installed_output="$("$install_dir/loongclaw")"
  if [[ "$installed_output" != "musl-binary" ]]; then
    echo "expected musl override artifact to be installed but got '$installed_output'" >&2
    exit 1
  fi
}

run_linux_x86_64_explicit_gnu_override_rejects_old_glibc_test() {
  local fixture output_file
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/linux-gnu-override-old-glibc.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.36"

  if (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --target-libc gnu --prefix "$fixture/install" >"$output_file" 2>&1
  ); then
    echo "expected install.sh to reject a GNU override on an unsupported glibc host" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "requires glibc"
}

run_version_at_least_falls_back_when_sort_version_is_unavailable_test() {
  local fixture
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' RETURN
  make_sort_stub_bin "$fixture" "__FAIL_VERSION__"

  if ! (
    PATH="$fixture/fake-bin:$PATH"
    source_install_functions
    version_at_least "2.39" "2.39"
  ); then
    echo "expected version_at_least to succeed even when sort -V is unavailable" >&2
    exit 1
  fi
}

run_version_at_least_rejects_older_version_with_sort_version_test() {
  if (
    source_install_functions
    version_at_least "2.16" "2.17"
  ); then
    echo "expected version_at_least to reject older versions when sort -V is available" >&2
    exit 1
  fi
}

run_detect_host_glibc_version_rejects_musl_ldd_output_test() {
  if (
    source_install_functions
    getconf() { return 1; }
    ldd() { printf 'musl libc (x86_64) Version 1.2.5\n'; }
    detect_host_glibc_version >/dev/null
  ); then
    echo "expected detect_host_glibc_version to reject musl ldd output" >&2
    exit 1
  fi
}

run_release_target_for_install_rejects_arm64_old_glibc_without_musl_test() {
  local output_file
  output_file="$(mktemp)"
  trap 'rm -f "$output_file"' RETURN

  if (
    source_install_functions
    detect_host_glibc_version() { printf '2.16\n'; }
    release_target_for_install "Linux" "aarch64" "auto" >"$output_file" 2>&1
  ); then
    echo "expected release_target_for_install to reject GNU-only arm64 installs on an unsupported glibc host" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "no musl release artifact is published for aarch64"
}

run_linux_x86_64_prefers_gnu_when_sort_version_is_unavailable_test() {
  local fixture install_dir output_file installed_output
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-gnu-no-sort-v.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.39"
  make_sort_stub_bin "$fixture" "__FAIL_VERSION__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-gnu"
  installed_output="$("$install_dir/loongclaw")"
  if [[ "$installed_output" != "gnu-binary" ]]; then
    echo "expected GNU artifact to be installed without sort -V support but got '$installed_output'" >&2
    exit 1
  fi
}

run_linux_x86_64_explicit_gnu_override_rejects_musl_ldd_output_test() {
  local fixture output_file
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/linux-gnu-override-musl-ldd.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "__FAIL__"
  make_ldd_stub_bin "$fixture" "musl libc (x86_64) Version 1.2.5"

  if (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --target-libc gnu --prefix "$fixture/install" >"$output_file" 2>&1
  ); then
    echo "expected install.sh to reject a GNU override when only musl ldd output is available" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "explicit GNU install requires detectable glibc"
}

run_linux_arm64_auto_rejects_old_glibc_without_musl_artifact_test() {
  local fixture output_file
  fixture="$(make_release_fixture "v0.1.2" "aarch64-unknown-linux-gnu" "arm64-gnu-binary")"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/linux-arm64-old-glibc.out"
  make_uname_stub_bin "$fixture" "Linux" "aarch64"
  make_getconf_stub_bin "$fixture" "glibc 2.16"

  if (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$fixture/install" >"$output_file" 2>&1
  ); then
    echo "expected install.sh to reject GNU-only arm64 installs on an unsupported glibc host" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "no musl release artifact is published for aarch64"
}

run_release_override_install_and_onboard_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install.out"
  marker="$fixture/onboard.log"
  : >"$marker"

  (
    cd "$REPO_ROOT"
    ONBOARD_MARKER="$marker" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  [[ -x "$install_dir/loongclaw" ]]
  assert_contains "$output_file" "Installed loongclaw"
  assert_contains "$output_file" "Running guided onboarding"
  assert_contains "$marker" "onboard"
}

run_checksum_mismatch_fails_test() {
  local fixture install_dir output_file tag target checksum_name
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/checksum.out"
  tag="v0.1.2"
  target="$(host_target)"
  checksum_name="$(release_archive_checksum_name "loongclaw" "$tag" "$target")"
  printf 'deadbeef  wrong-archive\n' >"$fixture/releases/download/$tag/$checksum_name"

  if (
    cd "$REPO_ROOT"
    LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version "$tag" --prefix "$install_dir" >"$output_file" 2>&1
  ); then
    echo "expected install.sh to fail on checksum mismatch" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "checksum verification failed"
}

run_missing_release_guidance_test() {
  local fixture output_file
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/missing-release.out"
  make_latest_release_stub_bin "$fixture"

  if (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      bash "$SCRIPT_UNDER_TEST" --prefix "$fixture/install" >"$output_file" 2>&1
  ); then
    echo "expected install.sh to fail when no latest GitHub release exists" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "no GitHub release is published for loongclaw-ai/loongclaw yet"
  assert_contains "$output_file" "git clone https://github.com/loongclaw-ai/loongclaw.git"
  assert_contains "$output_file" "bash scripts/install.sh --source --onboard"
}

run_standalone_linux_arm64_install_rejects_missing_glibc_test() {
  local fixture install_dir output_file standalone_script
  fixture="$(make_release_fixture "v0.1.2" "aarch64-unknown-linux-gnu")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/standalone-install.out"
  standalone_script="$fixture/install.sh"
  cp "$SCRIPT_UNDER_TEST" "$standalone_script"
  chmod +x "$standalone_script"
  make_uname_stub_bin "$fixture" "Linux" "aarch64"
  make_getconf_stub_bin "$fixture" "__FAIL__"
  make_ldd_stub_bin "$fixture" "__FAIL__"

  if (
    cd "$fixture"
    PATH="$fixture/fake-bin:$PATH" \
      LOONGCLAW_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$standalone_script" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  ); then
    echo "expected standalone install.sh to reject GNU-only arm64 installs without detectable glibc" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "could not detect a compatible glibc on the host"
  assert_contains "$output_file" "no musl release artifact is published for aarch64"
}

run_release_override_install_and_onboard_test
run_checksum_mismatch_fails_test
run_missing_release_guidance_test
run_linux_x86_64_prefers_gnu_when_glibc_is_supported_test
run_version_at_least_falls_back_when_sort_version_is_unavailable_test
run_version_at_least_rejects_older_version_with_sort_version_test
run_detect_host_glibc_version_rejects_musl_ldd_output_test
run_release_target_for_install_rejects_arm64_old_glibc_without_musl_test
run_linux_x86_64_prefers_gnu_when_sort_version_is_unavailable_test
run_linux_x86_64_falls_back_to_musl_when_glibc_is_too_old_test
run_linux_x86_64_falls_back_to_musl_when_glibc_detection_fails_test
run_linux_x86_64_explicit_musl_override_test
run_linux_x86_64_explicit_gnu_override_rejects_old_glibc_test
run_linux_x86_64_explicit_gnu_override_rejects_musl_ldd_output_test
run_linux_arm64_auto_rejects_old_glibc_without_musl_artifact_test
run_standalone_linux_arm64_install_rejects_missing_glibc_test

echo "install.sh smoke checks passed"
