#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID_NDK_LINUX_SHA256="601246087a682d1944e1e16dd85bc6e49560fe8b6d61255be2829178c8ed15d9"

. "$REPO_ROOT/scripts/release_artifact_lib.sh"

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
    echo "expected not to find '$needle' in $file" >&2
    exit 1
  fi
}

assert_android_ndk_sha256_hardening() {
  local workflow_path="$1"

  assert_contains "$workflow_path" "ANDROID_NDK_LINUX_SHA256: \"$ANDROID_NDK_LINUX_SHA256\""
  assert_not_contains "$workflow_path" "ANDROID_NDK_LINUX_SHA1:"
  assert_contains "$workflow_path" 'shasum -a 256 --check -'
  assert_not_contains "$workflow_path" 'sha1sum --check -'
}

assert_android_release_target_parity() {
  local expected_target

  expected_target="$(release_target_for_platform "Android" "aarch64")"

  assert_contains ".github/workflows/ci.yml" "targets: ${expected_target}"
  assert_contains ".github/workflows/ci.yml" "--target ${expected_target}"
  assert_not_contains ".github/workflows/ci.yml" "x86_64-linux-android"
  assert_contains ".github/workflows/release.yml" "target: ${expected_target}"
  assert_not_contains ".github/workflows/release.yml" "target: x86_64-linux-android"
}

cd "$REPO_ROOT"

assert_android_ndk_sha256_hardening ".github/workflows/ci.yml"
assert_android_ndk_sha256_hardening ".github/workflows/release.yml"
assert_android_release_target_parity

echo "release workflow hardening checks passed"
