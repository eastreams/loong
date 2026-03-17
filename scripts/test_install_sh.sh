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
  local fixture tag target archive_name checksum_name binary_name archive_path checksum_path release_dir
  fixture="$(mktemp -d)"
  tag="${1:-v0.1.2}"
  target="$(host_target)"
  archive_name="$(release_archive_name "loongclaw" "$tag" "$target")"
  checksum_name="$(release_archive_checksum_name "loongclaw" "$tag" "$target")"
  binary_name="$(release_binary_name_for_target "loongclaw" "$target")"
  release_dir="$fixture/releases/download/$tag"
  mkdir -p "$release_dir" "$fixture/staging"

  cat >"$fixture/staging/$binary_name" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "onboard" ]]; then
  printf 'onboard\n' >> "${ONBOARD_MARKER:?}"
fi
printf 'fixture-binary\n'
EOF
  chmod +x "$fixture/staging/$binary_name"

  archive_path="$release_dir/$archive_name"
  case "$archive_name" in
    *.tar.gz)
      tar -C "$fixture/staging" -czf "$archive_path" "$binary_name"
      ;;
    *.zip)
      (cd "$fixture/staging" && zip -q "$archive_path" "$binary_name")
      ;;
    *)
      echo "unsupported archive format in fixture: $archive_name" >&2
      exit 1
      ;;
  esac

  checksum_path="$release_dir/$checksum_name"
  printf '%s  %s\n' "$(sha256_file "$archive_path")" "$archive_name" >"$checksum_path"

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

run_release_override_install_and_onboard_test
run_checksum_mismatch_fails_test
run_missing_release_guidance_test

echo "install.sh smoke checks passed"
