#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

FILTERS=(
  "integration::cli_"
  "integration::architecture::"
  "integration::gateway_api_health::"
  "integration::gateway_read_models::"
  "integration::runtime_snapshot_cli::"
  "integration::status_cli::"
  "integration::work_unit_cli::"
)

build_output_json="$(mktemp -t loong-daemon-smoke-build.XXXXXX.json)"
trap 'rm -f "$build_output_json"' EXIT

./scripts/cargo-local-toolchain.sh test --locked -p loong --features test-support --test daemon_smoke --no-run --message-format=json >"$build_output_json"

test_binary="$(python3 - <<'PY' "$build_output_json"
import json
import sys
from pathlib import Path

artifact_path = None
for line in Path(sys.argv[1]).read_text(errors='ignore').splitlines():
    try:
        payload = json.loads(line)
    except Exception:
        continue
    if payload.get('reason') != 'compiler-artifact':
        continue
    target = payload.get('target', {})
    if target.get('name') == 'daemon_smoke' and payload.get('executable'):
        artifact_path = payload['executable']

if artifact_path is None:
    raise SystemExit('failed to locate daemon smoke test binary')
print(artifact_path)
PY
)"

if [[ -z "${LOONG_HOME:-}" ]]; then
  mkdir -p "$REPO_ROOT/target/test-loong-home"
  export LOONG_HOME="$REPO_ROOT/target/test-loong-home"
fi

for filter in "${FILTERS[@]}"; do
  echo "[daemon-smoke] $test_binary $filter"
  "$test_binary" "$filter"
done
