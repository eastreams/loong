#!/usr/bin/env bash
set -euo pipefail

# Resolve rustup-managed toolchain binaries up front so cargo builds/tests use
# the concrete rustc/rustdoc executables instead of the rustup proxy shims.
# This avoids mid-build rustup channel/target sync attempts on machines where
# the proxy may try to reconcile partially installed targets.
#
# For `cargo test`, also seed an isolated writable `LOONG_HOME` under
# `target/test-loong-home` when the caller has not set one explicitly. That
# keeps default audit/runtime-state paths out of the real home directory during
# local verification.

resolve_tool() {
  local tool="$1"
  if ! command -v rustup >/dev/null 2>&1; then
    return 1
  fi

  local resolved
  resolved="$(rustup which "$tool" 2>/dev/null || true)"
  if [[ -z "$resolved" || ! -x "$resolved" ]]; then
    return 1
  fi

  printf '%s\n' "$resolved"
}

is_test_invocation() {
  local arg
  for arg in "$@"; do
    case "$arg" in
      +*|-*)
        continue
        ;;
      test)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  done
  return 1
}

if [[ -z "${RUSTC:-}" ]]; then
  if resolved_rustc="$(resolve_tool rustc)"; then
    export RUSTC="$resolved_rustc"
  fi
fi

if [[ -z "${RUSTDOC:-}" ]]; then
  if resolved_rustdoc="$(resolve_tool rustdoc)"; then
    export RUSTDOC="$resolved_rustdoc"
  fi
fi

if [[ -z "${LOONG_HOME:-}" ]] && is_test_invocation "$@"; then
  repo_root="$(pwd)"
  loong_test_home="${repo_root}/target/test-loong-home"
  mkdir -p "$loong_test_home"
  export LOONG_HOME="$loong_test_home"
fi

exec cargo "$@"
