#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

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

smoke_filters=()
while IFS= read -r line; do
  smoke_filters+=("$line")
done < <(python3 - <<'PY' "$test_binary"
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
)

if [[ -z "${LOONG_HOME:-}" ]]; then
  mkdir -p "$REPO_ROOT/target/test-loong-home"
  export LOONG_HOME="$REPO_ROOT/target/test-loong-home"
fi

for filter in "${smoke_filters[@]}"; do
  echo "[daemon-smoke] $test_binary $filter"
  "$test_binary" "$filter"
done
