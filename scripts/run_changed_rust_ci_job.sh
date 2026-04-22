#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

resolve_python_bin() {
  if [[ -n "${PYTHON_BIN:-}" ]]; then
    if command -v "$PYTHON_BIN" >/dev/null 2>&1; then
      printf '%s\n' "$PYTHON_BIN"
      return 0
    fi
    echo "configured PYTHON_BIN '$PYTHON_BIN' was not found in PATH" >&2
    exit 1
  fi

  if command -v python3 >/dev/null 2>&1; then
    printf '%s\n' "python3"
    return 0
  fi

  if command -v python >/dev/null 2>&1; then
    printf '%s\n' "python"
    return 0
  fi

  echo "python3 or python is required" >&2
  exit 1
}

PYTHON_BIN="$(resolve_python_bin)"

all_features=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --all-features)
      all_features=1
      shift
      ;;
    *)
      echo "usage: scripts/run_changed_rust_ci_job.sh [--all-features]" >&2
      exit 1
      ;;
  esac
done

if [[ "${GITHUB_EVENT_NAME:-}" != "pull_request" ]]; then
  cargo_args=(test --workspace --locked)
  if [[ "$all_features" -eq 1 ]]; then
    cargo_args=(test --workspace --all-features --locked)
  fi
  cargo "${cargo_args[@]}"
  exit 0
fi

if [[ -z "${TOUCHED_PATHS_JSON:-}" ]]; then
  echo "TOUCHED_PATHS_JSON is required for pull_request fast-lane runs" >&2
  exit 1
fi

changed_args=()
if [[ "$all_features" -eq 1 ]]; then
  changed_args+=(--all-features)
fi

changed_paths=()
while IFS= read -r line; do
  line="${line%$'\r'}"
  changed_paths+=("$line")
done < <("$PYTHON_BIN" - <<'PY'
import json
import os

for path in json.loads(os.environ["TOUCHED_PATHS_JSON"]):
    print(path)
PY
)

changed_args+=("${changed_paths[@]}")
./scripts/test_changed_rust_packages.sh "${changed_args[@]}"
