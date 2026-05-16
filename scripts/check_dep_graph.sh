#!/usr/bin/env bash
set -euo pipefail

# Validate that the crate dependency graph matches the documented architecture
# contract and the additive local-only Phase 2 spine.
#
# Repository-visible contract:
#   contracts (leaf — zero internal deps)
#   ├── kernel → contracts
#   ├── protocol (independent leaf)
#   ├── bridge-runtime → contracts, kernel, protocol
#   ├── app → contracts, kernel
#   ├── spec → contracts, kernel, protocol, bridge-runtime
#   ├── bench → kernel, spec
#   └── daemon (binary) → app, bench, contracts, kernel, spec, bridge-runtime
#
# Additive local-only Phase 2 spine:
#   loong-core (leaf)
#   ├── loong-runtime → loong-core
#   ├── loong-app-protocol → loong-runtime
#   ├── loong-cli → loong-app-protocol
#   └── loong-plugin-sdk → loong-core
#
# Narrow Phase 3 migration allowance:
#   daemon -> loong-app-protocol for the single migrated `turn run` path

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

violations=0

# Extract workspace-internal dependency edges from cargo metadata.
# Output: "<crate-alias> -> <crate-alias>" lines for every checked workspace package.
edges="$(cargo metadata --format-version 1 2>/dev/null \
  | python3 -c '
import json, sys
meta = json.load(sys.stdin)
workspace_root = meta["workspace_root"].rstrip("/")
ALIASES = {
    "loong-contracts": "contracts",
    "loong-kernel": "kernel",
    "loong-protocol": "protocol",
    "loong-bridge-runtime": "bridge-runtime",
    "loong-app": "app",
    "loong-spec": "spec",
    "loong-bench": "bench",
    "loong": "daemon",
    "loong-core": "core",
    "loong-runtime": "runtime",
    "loong-app-protocol": "app-protocol",
    "loong-cli": "cli",
    "loong-plugin-sdk": "plugin-sdk",
}
ws_ids = {
    p["id"]: ALIASES[p["name"]]
    for p in meta["packages"]
    if p["manifest_path"].startswith(workspace_root + "/")
    and p["name"] in ALIASES
}
for node in meta["resolve"]["nodes"]:
    if node["id"] not in ws_ids:
        continue
    src = ws_ids[node["id"]]
    for dep in node["deps"]:
        if dep["pkg"] in ws_ids:
            dst = ws_ids[dep["pkg"]]
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
  "bench -> kernel"
  "bench -> spec"
  "daemon -> contracts"
  "daemon -> kernel"
  "daemon -> protocol"
  "daemon -> app"
  "daemon -> bridge-runtime"
  "daemon -> spec"
  "daemon -> bench"
  "daemon -> app-protocol"
  "runtime -> core"
  "app-protocol -> runtime"
  "cli -> app-protocol"
  "plugin-sdk -> core"
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
