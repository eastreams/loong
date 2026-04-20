#!/usr/bin/env bash
set -euo pipefail

# Validate that the crate dependency graph matches the documented architecture.
#
# Architecture contract (from CLAUDE.md):
#   contracts (leaf — zero internal deps)
#   ├── kernel → contracts
#   ├── protocol (independent leaf)
#   ├── bridge-runtime → contracts, kernel, protocol
#   ├── app → contracts, kernel
#   ├── spec → contracts, kernel, protocol, bridge-runtime
#   ├── bench → contracts, kernel, spec
#   └── daemon (binary) → all of the above

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

violations=0

# Extract workspace-internal dependency edges from cargo metadata.
# Output: "from_crate -> to_crate" lines for loong-* packages only.
PREFIX="loong-"
edges="$(cargo metadata --format-version 1 2>/dev/null \
  | python3 -c '
import json, sys
PREFIX = "loong-"
meta = json.load(sys.stdin)
ws_ids = {p["id"] for p in meta["packages"] if p["name"].startswith(PREFIX)}
ws_names = {p["id"]: p["name"][len(PREFIX):] for p in meta["packages"] if p["id"] in ws_ids}
for node in meta["resolve"]["nodes"]:
    if node["id"] not in ws_ids:
        continue
    src = ws_names[node["id"]]
    for dep in node["deps"]:
        if dep["pkg"] in ws_ids:
            dst = ws_names[dep["pkg"]]
            print(f"{src} -> {dst}")
' | sort -u)"

# Allowed edges (from architecture contract).
allowed=(
  "kernel -> contracts"
  "bridge-runtime -> contracts"
  "bridge-runtime -> kernel"
  "bridge-runtime -> protocol"
  "app -> contracts"
  "app -> kernel"
  "spec -> bridge-runtime"
  "spec -> contracts"
  "spec -> kernel"
  "spec -> protocol"
  "bench -> contracts"
  "bench -> kernel"
  "bench -> spec"
  "daemon -> contracts"
  "daemon -> kernel"
  "daemon -> protocol"
  "daemon -> app"
  "daemon -> bridge-runtime"
  "daemon -> spec"
  "daemon -> bench"
)

is_allowed() {
  local edge="$1"
  for a in "${allowed[@]}"; do
    if [[ "$edge" == "$a" ]]; then
      return 0
    fi
  done
  return 1
}

echo "[dep-graph] workspace edges:"
while IFS= read -r edge; do
  [[ -z "$edge" ]] && continue
  if is_allowed "$edge"; then
    echo "  [ok] $edge"
  else
    echo "  [VIOLATION] $edge"
    violations=$((violations + 1))
  fi
done <<< "$edges"

if (( violations > 0 )); then
  echo "[dep-graph] FAILED: $violations disallowed dependency edge(s)" >&2
  exit 1
fi

echo "[dep-graph] PASSED: all workspace edges match architecture contract"
