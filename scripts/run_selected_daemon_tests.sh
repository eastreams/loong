#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

all_features=0
include_lib_bins=0
selected_targets=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all-features)
      all_features=1
      shift
      ;;
    --include-lib-bins)
      include_lib_bins=1
      shift
      ;;
    *)
      selected_targets+=("$1")
      shift
      ;;
  esac
done

if [[ "${#selected_targets[@]}" -eq 0 ]]; then
  echo "usage: scripts/run_selected_daemon_tests.sh [--all-features] [--include-lib-bins] <target> [<target> ...]" >&2
  exit 1
fi

build_output_json="$(mktemp -t loong-daemon-tests-build.XXXXXX.json)"
trap 'rm -f "$build_output_json"' EXIT

feature_args=(--features test-support)
if [[ "$all_features" -eq 1 ]]; then
  feature_args=(--all-features)
fi

target_args=()
for target_name in "${selected_targets[@]}"; do
  target_args+=(--test "$target_name")
done

build_args=(test --locked -p loong "${feature_args[@]}")
if [[ "$include_lib_bins" -eq 1 ]]; then
  build_args+=(--lib --bins)
fi
build_args+=("${target_args[@]}" --no-run --message-format=json)

./scripts/cargo-local-toolchain.sh "${build_args[@]}" >"$build_output_json"

binary_payload_json="$(python3 - <<'PY' "$build_output_json" "${selected_targets[@]}"
import json
import sys
from pathlib import Path

build_output_path = Path(sys.argv[1])
selected_targets = sys.argv[2:]
artifact_paths = {}
lib_bin_paths = []
seen_lib_bin_paths = set()

for line in build_output_path.read_text(errors="ignore").splitlines():
    try:
        payload = json.loads(line)
    except Exception:
        continue
    if payload.get("reason") != "compiler-artifact":
        continue
    target = payload.get("target", {})
    target_name = target.get("name")
    target_kinds = target.get("kind", [])
    executable = payload.get("executable")

    if executable and ("lib" in target_kinds or "bin" in target_kinds):
        if executable not in seen_lib_bin_paths:
            seen_lib_bin_paths.add(executable)
            lib_bin_paths.append(executable)

    if target_name in selected_targets and executable:
        artifact_paths[target_name] = executable

missing_targets = [name for name in selected_targets if name not in artifact_paths]
if missing_targets:
    raise SystemExit(f"failed to locate daemon test binaries for: {', '.join(missing_targets)}")

print(json.dumps({
    "targets": artifact_paths,
    "lib_bins": lib_bin_paths,
}))
PY
)"

derive_smoke_filters() {
  local binary_path="$1"
  python3 - <<'PY' "$binary_path"
import subprocess
import sys

binary_path = sys.argv[1]
list_output = subprocess.check_output([binary_path, "--list"], text=True)
filters = []
seen = set()

for raw_line in list_output.splitlines():
    line = raw_line.strip()
    if not line.endswith(": test"):
        continue

    test_name = line[:-6]
    if not test_name.startswith("integration::"):
        continue

    parts = test_name.split("::")
    if len(parts) >= 3:
        filter_name = "::".join(parts[:2]) + "::"
    else:
        filter_name = test_name

    if filter_name in seen:
        continue

    seen.add(filter_name)
    filters.append(filter_name)

if not filters:
    raise SystemExit("failed to derive daemon smoke filters from test binary")

for filter_name in filters:
    print(filter_name)
PY
}

run_target_binary() {
  local target_name="$1"
  local binary_path="$2"

  if [[ "$target_name" == "daemon_smoke" ]]; then
    local smoke_filters=()
    while IFS= read -r line; do
      smoke_filters+=("$line")
    done < <(derive_smoke_filters "$binary_path")

    for filter_name in "${smoke_filters[@]}"; do
      echo "[daemon-smoke] $binary_path $filter_name"
      "$binary_path" "$filter_name"
    done
    return
  fi

  echo "[daemon-test] $binary_path"
  "$binary_path"
}

run_lib_or_bin_binary() {
  local binary_path="$1"
  echo "[loong-test] $binary_path"
  "$binary_path"
}

if [[ -z "${LOONG_HOME:-}" ]]; then
  mkdir -p "$REPO_ROOT/target/test-loong-home"
  export LOONG_HOME="$REPO_ROOT/target/test-loong-home"
fi

if [[ "$include_lib_bins" -eq 1 ]]; then
  while IFS= read -r binary_path; do
    [[ -n "$binary_path" ]] || continue
    run_lib_or_bin_binary "$binary_path"
  done < <(python3 - <<'PY' "$binary_payload_json"
import json
import sys

payload = json.loads(sys.argv[1])
for binary_path in payload["lib_bins"]:
    print(binary_path)
PY
)
fi

while IFS= read -r target_name; do
  binary_path="$(python3 - <<'PY' "$binary_payload_json" "$target_name"
import json
import sys

artifact_paths = json.loads(sys.argv[1])["targets"]
target_name = sys.argv[2]
print(artifact_paths[target_name])
PY
)"
  run_target_binary "$target_name" "$binary_path"
done < <(printf '%s\n' "${selected_targets[@]}")
