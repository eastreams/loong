#!/usr/bin/env bash
set -euo pipefail

# Validate that the crate dependency graph matches the documented architecture
# contract across every workspace crate.
#
# Repository-visible contract:
#   contracts (leaf — zero internal deps)
#   ├── loong-core → contracts
#   ├── loong-access → contracts, loong-core
#   ├── kernel → contracts, loong-access, loong-core, loong-plugin-sdk
#   ├── loong-tools → contracts, loong-core, kernel
#   ├── protocol (independent leaf)
#   ├── bridge-runtime → contracts, kernel, protocol
#   ├── app → contracts, loong-core, kernel, loong-runtime, loong-tools
#   ├── spec → contracts, loong-core, kernel, protocol, bridge-runtime
#   ├── bench → kernel, spec
#   └── daemon (binary) → app, bench, contracts, loong-core, kernel, protocol, spec, bridge-runtime
#
# Additive spine:
#   loong-core
#   ├── loong-runtime → contracts, loong-core, kernel
#   ├── loong-app-protocol → loong-core
#   ├── loong-cli → loong-app-protocol
#   └── loong-plugin-sdk → loong-core

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

violations=0

# Extract workspace-internal dependency edges from cargo metadata without
# resolving external dependencies. The architecture check only cares about path
# deps between workspace packages, so downloading registry crates is noise.
# Output: "<crate-alias> -> <crate-alias>" lines for every checked workspace package.
edges="$(cargo metadata --format-version 1 --no-deps \
  | python3 -c '
import json, sys
meta = json.load(sys.stdin)
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
    "loong-access": "access",
    "loong-tools": "tools",
}
workspace_member_ids = set(meta["workspace_members"])
workspace_packages = {
    p["name"]: p
    for p in meta["packages"]
    if p["id"] in workspace_member_ids
}
unknown_packages = sorted(set(workspace_packages) - set(ALIASES))
if unknown_packages:
    print(
        "[dep-graph] missing aliases for workspace package(s): "
        + ", ".join(unknown_packages),
        file=sys.stderr,
    )
    sys.exit(2)
ws_packages = workspace_packages
for package_name, package in ws_packages.items():
    src = ALIASES[package_name]
    for dep in package["dependencies"]:
        if dep["name"] in ws_packages:
            dst = ALIASES[dep["name"]]
            print(f"{src} -> {dst}")
' | sort -u)"

# Allowed edges (from architecture contract).
allowed=(
  "access -> contracts"
  "access -> core"
  "kernel -> access"
  "kernel -> plugin-sdk"
  "kernel -> core"
  "kernel -> contracts"
  "bridge-runtime -> contracts"
  "bridge-runtime -> kernel"
  "bridge-runtime -> protocol"
  "app -> contracts"
  "app -> core"
  "app -> kernel"
  "app -> runtime"
  "app -> tools"
  "spec -> bridge-runtime"
  "spec -> contracts"
  "spec -> core"
  "spec -> kernel"
  "spec -> protocol"
  "bench -> kernel"
  "bench -> spec"
  "daemon -> contracts"
  "daemon -> core"
  "daemon -> kernel"
  "daemon -> protocol"
  "daemon -> app"
  "daemon -> bridge-runtime"
  "daemon -> spec"
  "daemon -> bench"
  "core -> contracts"
  "runtime -> contracts"
  "runtime -> core"
  "runtime -> kernel"
  "app-protocol -> core"
  "cli -> app-protocol"
  "plugin-sdk -> core"
  "tools -> contracts"
  "tools -> core"
  "tools -> kernel"
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
