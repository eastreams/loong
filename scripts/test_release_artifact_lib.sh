#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$REPO_ROOT/scripts/release_artifact_lib.sh"

assert_equals() {
  local expected="$1"
  local actual="$2"
  if [[ "$actual" != "$expected" ]]; then
    echo "expected '$expected' but got '$actual'" >&2
    exit 1
  fi
}

run_release_artifact_lib_tests() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  cat >"$tmp_dir/CHANGELOG.md" <<'EOF'
# Changelog

## [0.1.2] - 2026-03-09

## [0.1.1] - 2026-03-09

## [0.1.0] - 2026-03-09
EOF

  cat >"$tmp_dir/v0.1.2.md" <<'EOF'
# Release v0.1.2

## Summary
- Generated at: 2026-03-09T05:39:43Z
- Trace ID: `020e2a67`
- Trace path: `.docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67`
EOF

  local versions
  versions="$(release_versions_from_changelog "$tmp_dir/CHANGELOG.md")"
  assert_equals $'0.1.2\n0.1.1\n0.1.0' "$versions"
  assert_equals "v0.1.2" "$(release_tag_from_version "0.1.2")"
  assert_equals "2026-03-09T05:39:43Z" "$(release_doc_generated_at "$tmp_dir/v0.1.2.md")"
  assert_equals "020e2a67" "$(release_doc_backticked_field "$tmp_dir/v0.1.2.md" "Trace ID")"
  assert_equals \
    ".docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67" \
    "$(release_doc_backticked_field "$tmp_dir/v0.1.2.md" "Trace path")"
  assert_equals ".docs/releases/v0.1.2-debug.md" "$(release_debug_doc_relpath "v0.1.2")"
  assert_equals \
    "loongclaw-v0.1.2-x86_64-unknown-linux-gnu.tar.gz" \
    "$(release_archive_name "loongclaw" "v0.1.2" "x86_64-unknown-linux-gnu")"
  assert_equals \
    "loongclaw-v0.1.2-aarch64-apple-darwin.tar.gz" \
    "$(release_archive_name "loongclaw" "v0.1.2" "aarch64-apple-darwin")"
  assert_equals \
    "loongclaw-v0.1.2-aarch64-unknown-linux-gnu.tar.gz" \
    "$(release_archive_name "loongclaw" "v0.1.2" "aarch64-unknown-linux-gnu")"
  assert_equals \
    "loongclaw-v0.1.2-x86_64-unknown-linux-musl.tar.gz" \
    "$(release_archive_name "loongclaw" "v0.1.2" "x86_64-unknown-linux-musl")"
  assert_equals \
    "loongclaw-v0.1.2-x86_64-pc-windows-msvc.zip" \
    "$(release_archive_name "loongclaw" "v0.1.2" "x86_64-pc-windows-msvc")"
  assert_equals \
    "loongclaw-v0.1.2-x86_64-pc-windows-msvc.zip.sha256" \
    "$(release_archive_checksum_name "loongclaw" "v0.1.2" "x86_64-pc-windows-msvc")"
  assert_equals \
    "loongclaw-v0.1.2-x86_64-unknown-linux-musl.tar.gz.sha256" \
    "$(release_archive_checksum_name "loongclaw" "v0.1.2" "x86_64-unknown-linux-musl")"
  assert_equals \
    "x86_64-unknown-linux-gnu" \
    "$(release_target_for_platform "Linux" "x86_64")"
  assert_equals \
    "aarch64-unknown-linux-gnu" \
    "$(release_target_for_platform "Linux" "arm64")"
  assert_equals \
    $'gnu\nmusl' \
    "$(release_supported_linux_libcs_for_arch "x86_64")"
  assert_equals \
    "gnu" \
    "$(release_supported_linux_libcs_for_arch "aarch64")"
  assert_equals \
    "x86_64-unknown-linux-musl" \
    "$(release_linux_target_for_arch_and_libc "x86_64" "musl")"
  assert_equals \
    "x86_64-unknown-linux-gnu" \
    "$(release_linux_target_for_arch_and_libc "x86_64" "gnu")"
  assert_equals \
    "aarch64-unknown-linux-gnu" \
    "$(release_linux_target_for_arch_and_libc "aarch64" "gnu")"
  assert_equals \
    "2.39" \
    "$(release_gnu_glibc_floor_for_target "x86_64-unknown-linux-gnu")"
  assert_equals \
    "2.17" \
    "$(release_gnu_glibc_floor_for_target "aarch64-unknown-linux-gnu")"
  assert_equals \
    "x86_64-apple-darwin" \
    "$(release_target_for_platform "Darwin" "x86_64")"
  assert_equals \
    "aarch64-apple-darwin" \
    "$(release_target_for_platform "Darwin" "arm64")"
  assert_equals \
    "x86_64-pc-windows-msvc" \
    "$(release_target_for_platform "Windows_NT" "AMD64")"

  version_is_greater "0.1.2" "0.1.1"
  if version_is_greater "0.1.1" "0.1.2"; then
    echo "expected version_is_greater to reject smaller version" >&2
    exit 1
  fi
  version_is_greater "0.1.0-alpha.2" "0.1.0-alpha.1"
  if version_is_greater "0.1.0-alpha.1" "0.1.0-alpha.2"; then
    echo "expected version_is_greater to reject smaller prerelease version" >&2
    exit 1
  fi
  version_is_greater "0.1.0" "0.1.0-alpha.9"
  version_is_greater "0.2.0-alpha.1" "0.1.9"

  if release_target_for_platform "Linux" "ppc64le" >/dev/null 2>&1; then
    echo "expected release_target_for_platform to reject unsupported host arch" >&2
    exit 1
  fi

  if release_supported_linux_libcs_for_arch "ppc64le" >/dev/null 2>&1; then
    echo "expected release_supported_linux_libcs_for_arch to reject unsupported host arch" >&2
    exit 1
  fi

  release_trace_path_matches_contract \
    "v0.1.2" \
    "020e2a67" \
    ".docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67"

  if release_trace_path_matches_contract \
    "v0.1.2" \
    "020e2a67" \
    ".docs/traces/20260309T053941Z-v0.1.2-020e2a67"; then
    echo "expected release_trace_path_matches_contract to reject missing post-release marker" >&2
    exit 1
  fi

  if release_trace_path_matches_contract \
    "v0.1.2" \
    "WRONG999" \
    ".docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67"; then
    echo "expected release_trace_path_matches_contract to reject mismatched trace id" >&2
    exit 1
  fi

  if release_trace_path_matches_contract \
    "v0.1.2" \
    "020e2a67" \
    ".docs/traces/../outside/20260309T053941Z-post-release-v0.1.2-020e2a67"; then
    echo "expected release_trace_path_matches_contract to reject traversal segments" >&2
    exit 1
  fi

  if release_trace_path_matches_contract \
    "v0.1.2" \
    "020e2a67" \
    ".docs/traces/./20260309T053941Z-post-release-v0.1.2-020e2a67"; then
    echo "expected release_trace_path_matches_contract to reject dot path segments" >&2
    exit 1
  fi

  mkdir -p "$tmp_dir/.docs/traces" "$tmp_dir/outside-target"
  ln -s "$tmp_dir/outside-target" "$tmp_dir/.docs/traces/out"

  if (
    cd "$tmp_dir" &&
      release_trace_path_matches_contract \
        "v0.1.2" \
        "020e2a67" \
        ".docs/traces/out/20260309T053941Z-post-release-v0.1.2-020e2a67"
  ); then
    echo "expected release_trace_path_matches_contract to reject symlink-backed trace path prefixes" >&2
    exit 1
  fi

  cat >"$tmp_dir/CHANGELOG.md" <<'EOF'
# Changelog

## [0.1.0-alpha.2] - 2026-03-17

## [0.1.0-alpha.1] - 2026-03-17
EOF

  versions="$(release_versions_from_changelog "$tmp_dir/CHANGELOG.md")"
  assert_equals $'0.1.0-alpha.2\n0.1.0-alpha.1' "$versions"
  assert_equals "v0.1.0-alpha.1" "$(release_tag_from_version "0.1.0-alpha.1")"
  assert_equals ".docs/releases/v0.1.0-alpha.1-debug.md" "$(release_debug_doc_relpath "v0.1.0-alpha.1")"
}

run_release_artifact_lib_tests

echo "release_artifact_lib.sh checks passed"
