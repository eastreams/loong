#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/install.sh"
. "$REPO_ROOT/scripts/release_artifact_lib.sh"
PACKAGE_NAME="loong"
PRIMARY_BIN_NAME="loong"
LEGACY_BIN_NAME="loong"

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

assert_installed_binary() {
  local install_dir="$1"
  local expected_output="$2"
  local primary_output
  local legacy_path

  [[ -x "$install_dir/$PRIMARY_BIN_NAME" ]]
  legacy_path="$install_dir/$LEGACY_BIN_NAME"
  [[ ! -f "$legacy_path" ]]
  [[ ! -L "$legacy_path" ]]

  primary_output="$("$install_dir/$PRIMARY_BIN_NAME")"

  if [[ "$primary_output" != "$expected_output" ]]; then
    echo "expected primary binary output '$expected_output' but got '$primary_output'" >&2
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
  archive_name="$(release_archive_name "$PACKAGE_NAME" "$tag" "$target")"
  checksum_name="$(release_archive_checksum_name "$PACKAGE_NAME" "$tag" "$target")"
  binary_name="$(release_binary_name_for_target "$PRIMARY_BIN_NAME" "$target")"
  release_dir="$fixture/releases/download/$tag"
  staging_dir="$fixture/staging/$target"
  mkdir -p "$release_dir" "$staging_dir"

  cat >"$staging_dir/$binary_name" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "onboard" ]]; then
  shift
  selected_provider="${LOONG_WEB_SEARCH_PROVIDER:-${LOONG_WEB_SEARCH_PROVIDER:-}}"
  while [[ "$#" -gt 0 ]]; do
    case "${1:-}" in
      --web-search-provider)
        shift
        selected_provider="${1:-}"
        ;;
    esac
    shift || true
  done
  {
    printf 'onboard\n'
    printf 'web_search_provider=%s\n' "${selected_provider:-}"
  } >> "${ONBOARD_MARKER:?}"
  exit "${ONBOARD_EXIT_CODE:-0}"
fi
EOF
  printf "printf '%%s\\n' %q\n" "$binary_label" >>"$staging_dir/$binary_name"
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
if [[ "$url" == "https://api.github.com/repos/eastreams/loong/releases/latest" ]]; then
  exit 22
fi

cat >&2 <<ERR
unexpected curl request: $url
ERR
exit 1
EOF
  chmod +x "$fixture/fake-bin/curl"
}

make_install_curl_stub_bin() {
  local fixture="$1"
  local duckduckgo_mode="${2:?duckduckgo_mode is required}"
  local tavily_result="${3:?tavily_result is required}"
  local real_curl
  real_curl="$(command -v curl)"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/curl" <<EOF
#!/usr/bin/env bash
set -euo pipefail

url="\${@: -1}"
case "\$url" in
  https://html.duckduckgo.com/html/*)
    if [[ "$duckduckgo_mode" == "success" ]]; then
      exit 0
    fi
    exit 22
    ;;
  https://api.tavily.com/search)
    if [[ "$tavily_result" == "__FAIL__" ]]; then
      exit 7
    fi
    printf '%s' "$tavily_result"
    exit 0
    ;;
esac

exec "$real_curl" "\$@"
EOF
  chmod +x "$fixture/fake-bin/curl"
}

make_uname_stub_bin() {
  local fixture="$1"
  local stub_platform="$2"
  local stub_arch="$3"
  local stub_operating_system="${4:-GNU/Linux}"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/uname" <<EOF2
#!/usr/bin/env bash
set -euo pipefail

case "\${1:-}" in
  -s) printf '%s\n' "$stub_platform" ;;
  -m) printf '%s\n' "$stub_arch" ;;
  -o) printf '%s\n' "$stub_operating_system" ;;
  *)
    echo "unexpected uname invocation: \$*" >&2
    exit 1
    ;;
esac
EOF2
  chmod +x "$fixture/fake-bin/uname"
}

make_getconf_stub_bin() {
  local fixture="$1"
  local response="$2"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/getconf" <<EOF2
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
EOF2
  chmod +x "$fixture/fake-bin/getconf"
}

make_ldd_stub_bin() {
  local fixture="$1"
  local response="$2"
  mkdir -p "$fixture/fake-bin"
  cat >"$fixture/fake-bin/ldd" <<EOF2
#!/usr/bin/env bash
set -euo pipefail

if [[ "$response" == "__FAIL__" ]]; then
  exit 1
fi

printf '%s\n' "$response"
EOF2
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
  local fixture install_dir output_file
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-gnu.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.39"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-gnu"
  assert_installed_binary "$install_dir" "gnu-binary"
}


run_termux_arm64_installs_android_release_test() {
  local fixture install_dir output_file
  fixture="$(make_release_fixture "v0.1.2" "aarch64-linux-android" "termux-binary")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/termux-android.out"
  make_uname_stub_bin "$fixture" "Linux" "aarch64" "Android"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      TERMUX_VERSION="0.119.0" \
      PREFIX="/data/data/com.termux/files/usr" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "aarch64-linux-android"
  assert_installed_binary "$install_dir" "termux-binary"
}

run_linux_guest_with_termux_env_is_not_termux_test() {
  if (
    source_install_functions
    TERMUX_VERSION="0.119.0"
    PREFIX="/data/data/com.termux/files/usr"
    uname() {
      case "${1:-}" in
        -s) printf 'Linux\n' ;;
        -m) printf 'x86_64\n' ;;
        -o) printf 'GNU/Linux\n' ;;
        *)
          echo "unexpected uname invocation: $*" >&2
          return 1
          ;;
      esac
    }
    is_termux_environment
  ); then
    echo "expected GNU/Linux shells with inherited Termux env to stay on Linux artifacts" >&2
    exit 1
  fi
}

run_linux_guest_with_termux_env_prefers_linux_release_test() {
  local fixture install_dir output_file
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-termux-env.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64" "GNU/Linux"
  make_getconf_stub_bin "$fixture" "glibc 2.39"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      TERMUX_VERSION="0.119.0" \
      PREFIX="/data/data/com.termux/files/usr" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-gnu"
  assert_installed_binary "$install_dir" "gnu-binary"
}

run_termux_x86_64_rejects_android_release_test() {
  local fixture output_file
  fixture="$(mktemp -d)"
  trap 'rm -rf "$fixture"' RETURN
  output_file="$fixture/termux-android-x86_64.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64" "Android"

  if (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      TERMUX_VERSION="0.119.0" \
      PREFIX="/data/data/com.termux/files/usr" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$fixture/install" >"$output_file" 2>&1
  ); then
    echo "expected install.sh to reject unsupported Android x86_64 hosts" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "unsupported Android architecture"
}

run_linux_x86_64_falls_back_to_musl_when_glibc_is_too_old_test() {
  local fixture install_dir output_file
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-musl-old-glibc.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.36"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-musl"
  assert_installed_binary "$install_dir" "musl-binary"
}

run_linux_x86_64_falls_back_to_musl_when_glibc_detection_fails_test() {
  local fixture install_dir output_file
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
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-musl"
  assert_installed_binary "$install_dir" "musl-binary"
}

run_linux_x86_64_explicit_musl_override_test() {
  local fixture install_dir output_file
  fixture="$(make_linux_dual_libc_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/linux-musl-override.out"
  make_uname_stub_bin "$fixture" "Linux" "x86_64"
  make_getconf_stub_bin "$fixture" "glibc 2.39"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      LOONG_INSTALL_TARGET_LIBC="musl" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-musl"
  assert_installed_binary "$install_dir" "musl-binary"
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
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
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
  local fixture install_dir output_file
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
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_contains "$output_file" "x86_64-unknown-linux-gnu"
  assert_installed_binary "$install_dir" "gnu-binary"
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
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
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
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
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
  make_install_curl_stub_bin "$fixture" "success" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$output_file" "Installed loong to"
  assert_contains "$output_file" "Running guided onboarding"
  assert_contains "$marker" "onboard"
}

run_release_install_removes_stale_legacy_binary_test() {
  local fixture install_dir output_file stale_legacy_binary
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-remove-legacy.out"
  stale_legacy_binary="$install_dir/$LEGACY_BIN_NAME"

  mkdir -p "$install_dir"
  printf 'stale legacy binary\n' >"$stale_legacy_binary"
  chmod +x "$stale_legacy_binary"

  (
    cd "$REPO_ROOT"
    LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$output_file" "Removed legacy loong compatibility command from"
}

run_release_install_keeps_legacy_directory_collision_test() {
  local fixture install_dir output_file stale_legacy_directory
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-keep-legacy-directory.out"
  stale_legacy_directory="$install_dir/$LEGACY_BIN_NAME"

  mkdir -p "$stale_legacy_directory"

  (
    cd "$REPO_ROOT"
    LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  [[ -d "$stale_legacy_directory" ]]
  assert_not_contains "$output_file" "Removed legacy loong compatibility command from"
}

run_release_install_adds_path_to_bashrc_and_prints_source_hint_test() {
  local fixture
  local home_dir
  local install_dir
  local output_file
  local bashrc_file
  local expected_path_line
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  home_dir="$fixture/home"
  install_dir="$fixture/install"
  output_file="$fixture/install-bashrc.out"
  bashrc_file="$home_dir/.bashrc"
  expected_path_line="export PATH=\"$install_dir:\$PATH\""
  mkdir -p "$home_dir"

  (
    cd "$REPO_ROOT"
    HOME="$home_dir" \
      SHELL="/bin/bash" \
      PATH="/usr/bin:/bin" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$bashrc_file" "# Added by Loong installer"
  assert_contains "$bashrc_file" "$expected_path_line"
  assert_contains "$output_file" "Added $install_dir to PATH in $bashrc_file"
  assert_contains "$output_file" "source \"$bashrc_file\""
}

run_release_install_skips_source_hint_when_path_is_already_available_test() {
  local fixture
  local home_dir
  local install_dir
  local output_file
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  home_dir="$fixture/home"
  install_dir="$fixture/install"
  output_file="$fixture/install-path-present.out"
  mkdir -p "$home_dir"

  (
    cd "$REPO_ROOT"
    HOME="$home_dir" \
      SHELL="/bin/bash" \
      PATH="$install_dir:/usr/bin:/bin" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_not_contains "$output_file" "source \"$home_dir/.bashrc\""
  assert_not_contains "$output_file" "Add to PATH if needed:"
}

run_release_install_keeps_source_hint_when_rc_already_has_path_entry_test() {
  local fixture
  local home_dir
  local install_dir
  local output_file
  local bashrc_file
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  home_dir="$fixture/home"
  install_dir="$fixture/install"
  output_file="$fixture/install-rc-already-set.out"
  bashrc_file="$home_dir/.bashrc"
  mkdir -p "$home_dir"
  printf 'export PATH="%s:$PATH"\n' "$install_dir" >"$bashrc_file"

  (
    cd "$REPO_ROOT"
    HOME="$home_dir" \
      SHELL="/bin/bash" \
      PATH="/usr/bin:/bin" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$output_file" "PATH entry already present in $bashrc_file"
  assert_contains "$output_file" "source \"$bashrc_file\""
}

run_release_install_unsupported_shell_uses_manual_path_hint_only_test() {
  local fixture
  local home_dir
  local install_dir
  local output_file
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  home_dir="$fixture/home"
  install_dir="$fixture/install"
  output_file="$fixture/install-fish-shell.out"
  mkdir -p "$home_dir"

  (
    cd "$REPO_ROOT"
    HOME="$home_dir" \
      SHELL="/usr/bin/fish" \
      PATH="/usr/bin:/bin" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$output_file" "Add to PATH if needed:"
  assert_not_contains "$output_file" 'source "$HOME/.profile"'
}

run_release_override_install_and_onboard_failure_preserves_install_test() {
  local fixture
  local install_dir
  local output_file
  local marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-onboard-failure.out"
  marker="$fixture/onboard-failure.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "success" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      ONBOARD_EXIT_CODE="130" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$marker" "onboard"
  assert_contains "$output_file" "Onboarding exited with code 130"
  assert_contains "$output_file" "You can run 'loong onboard' later to complete setup"
  assert_contains "$output_file" "Done. Try:"
}

run_release_override_install_and_onboard_detects_duckduckgo_default_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-ddg.out"
  marker="$fixture/onboard-web-search-ddg.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "success" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LANG="C.UTF-8" \
      TZ="UTC" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_contains "$output_file" "Onboarding web search default: DuckDuckGo (detected)"
  assert_contains "$marker" "web_search_provider=duckduckgo"
}

run_release_install_honors_legacy_release_base_env_test() {
  local fixture
  local install_dir
  local output_file
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-legacy-release-base.out"

  (
    cd "$REPO_ROOT"
    LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
}

run_release_override_install_and_onboard_honors_legacy_web_search_provider_env_test() {
  local fixture
  local install_dir
  local output_file
  local marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-legacy-env.out"
  marker="$fixture/onboard-web-search-legacy-env.log"
  : >"$marker"

  (
    cd "$REPO_ROOT"
    ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      LOONG_WEB_SEARCH_PROVIDER="brave" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_installed_binary "$install_dir" "fixture-binary"
  assert_contains "$output_file" "Onboarding web search default: Brave Search (preconfigured)"
  assert_contains "$marker" "web_search_provider=brave"
}

run_release_override_install_and_onboard_prefers_tavily_for_domestic_hosts_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-tavily.out"
  marker="$fixture/onboard-web-search-tavily.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "__FAIL__" "401"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LANG="zh_CN.UTF-8" \
      TZ="Asia/Shanghai" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_contains "$output_file" "Onboarding web search default: Tavily (detected)"
  assert_contains "$marker" "web_search_provider=tavily"
}

run_release_override_install_and_onboard_prefers_unique_ready_credential_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-perplexity.out"
  marker="$fixture/onboard-web-search-perplexity.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "success" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LANG="C.UTF-8" \
      TZ="UTC" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="perplexity-test-token" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_contains "$output_file" "Onboarding web search default: Perplexity Search (detected credential)"
  assert_contains "$marker" "web_search_provider=perplexity"
}

run_release_override_install_and_onboard_prefers_unique_ready_firecrawl_credential_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-firecrawl.out"
  marker="$fixture/onboard-web-search-firecrawl.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "success" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LANG="C.UTF-8" \
      TZ="UTC" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="firecrawl-test-token" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_contains "$output_file" "Onboarding web search default: Firecrawl Search (detected credential)"
  assert_contains "$marker" "web_search_provider=firecrawl"
}

run_release_override_install_and_onboard_preserves_signal_source_when_firecrawl_and_exa_both_exist_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-firecrawl-exa-ambiguous.out"
  marker="$fixture/onboard-web-search-firecrawl-exa-ambiguous.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "success" "__FAIL__"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LANG="C.UTF-8" \
      TZ="UTC" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="" \
      PERPLEXITY_API_KEY="" \
      EXA_API_KEY="exa-test-token" \
      FIRECRAWL_API_KEY="firecrawl-test-token" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_contains "$output_file" "Onboarding web search default: DuckDuckGo (detected)"
  assert_contains "$marker" "web_search_provider=duckduckgo"
}

run_release_override_install_and_onboard_preserves_signal_source_when_multiple_credentials_exist_test() {
  local fixture install_dir output_file marker
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/install-web-search-tavily-multi-credential.out"
  marker="$fixture/onboard-web-search-tavily-multi-credential.log"
  : >"$marker"
  make_install_curl_stub_bin "$fixture" "success" "401"

  (
    cd "$REPO_ROOT"
    PATH="$fixture/fake-bin:$PATH" \
      LANG="zh_CN.UTF-8" \
      TZ="Asia/Shanghai" \
      ONBOARD_MARKER="$marker" \
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
      BRAVE_API_KEY="" \
      TAVILY_API_KEY="tavily-test-token" \
      PERPLEXITY_API_KEY="perplexity-test-token" \
      EXA_API_KEY="" \
      FIRECRAWL_API_KEY="" \
      JINA_API_KEY="" \
      JINA_AUTH_TOKEN="" \
      LOONG_WEB_SEARCH_PROVIDER="" \
      bash "$SCRIPT_UNDER_TEST" --version v0.1.2 --prefix "$install_dir" --onboard >"$output_file" 2>&1
  )

  assert_contains "$output_file" "Onboarding web search default: Tavily (detected)"
  assert_contains "$marker" "web_search_provider=tavily"
}

run_checksum_mismatch_fails_test() {
  local fixture install_dir output_file tag target checksum_name
  fixture="$(make_release_fixture "v0.1.2")"
  trap 'rm -rf "$fixture"' RETURN
  install_dir="$fixture/install"
  output_file="$fixture/checksum.out"
  tag="v0.1.2"
  target="$(host_target)"
  checksum_name="$(release_archive_checksum_name "$PACKAGE_NAME" "$tag" "$target")"
  printf 'deadbeef  wrong-archive\n' >"$fixture/releases/download/$tag/$checksum_name"

  if (
    cd "$REPO_ROOT"
    LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
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

  assert_contains "$output_file" "no GitHub release is published for eastreams/loong yet"
  assert_contains "$output_file" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  assert_contains "$output_file" 'source "$HOME/.cargo/env"'
  assert_contains "$output_file" "git clone https://github.com/eastreams/loong.git"
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
      LOONG_INSTALL_RELEASE_BASE_URL="file://$fixture/releases" \
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
run_release_install_removes_stale_legacy_binary_test
run_release_install_keeps_legacy_directory_collision_test
run_release_install_adds_path_to_bashrc_and_prints_source_hint_test
run_release_install_skips_source_hint_when_path_is_already_available_test
run_release_install_keeps_source_hint_when_rc_already_has_path_entry_test
run_release_install_unsupported_shell_uses_manual_path_hint_only_test
run_release_override_install_and_onboard_failure_preserves_install_test
run_release_override_install_and_onboard_detects_duckduckgo_default_test
run_release_install_honors_legacy_release_base_env_test
run_release_override_install_and_onboard_honors_legacy_web_search_provider_env_test
run_release_override_install_and_onboard_prefers_tavily_for_domestic_hosts_test
run_release_override_install_and_onboard_prefers_unique_ready_credential_test
run_release_override_install_and_onboard_prefers_unique_ready_firecrawl_credential_test
run_release_override_install_and_onboard_preserves_signal_source_when_firecrawl_and_exa_both_exist_test
run_release_override_install_and_onboard_preserves_signal_source_when_multiple_credentials_exist_test
run_checksum_mismatch_fails_test
run_missing_release_guidance_test
run_linux_x86_64_prefers_gnu_when_glibc_is_supported_test
run_termux_arm64_installs_android_release_test
run_linux_guest_with_termux_env_is_not_termux_test
run_linux_guest_with_termux_env_prefers_linux_release_test
run_termux_x86_64_rejects_android_release_test
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
