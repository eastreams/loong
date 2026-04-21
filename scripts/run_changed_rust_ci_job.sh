#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

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
  changed_paths+=("$line")
done < <(python3 - <<'PY'
import json
import os

for path in json.loads(os.environ["TOUCHED_PATHS_JSON"]):
    print(path)
PY
)

changed_args+=("${changed_paths[@]}")
./scripts/test_changed_rust_packages.sh "${changed_args[@]}"
