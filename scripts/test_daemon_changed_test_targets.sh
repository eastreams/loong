#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

python3 - <<'PY'
import json
import subprocess


def resolve(*paths: str) -> list[str]:
    command = [
        "python3",
        "scripts/daemon_changed_test_targets.py",
        "--format",
        "json",
        *paths,
    ]
    output = subprocess.check_output(command, text=True)
    payload = json.loads(output)
    return payload["targets"]


def assert_equal(actual, expected, label: str) -> None:
    if actual != expected:
        raise SystemExit(f"{label}: expected {expected!r}, got {actual!r}")


assert_equal(resolve("docs/RELIABILITY.md"), [], "docs changes do not select daemon shards")
assert_equal(
    resolve("crates/daemon/tests/integration/onboard_cli.rs"),
    ["daemon_onboard"],
    "onboard module maps to onboard shard",
)
assert_equal(
    resolve("crates/daemon/tests/integration/status_cli.rs"),
    ["daemon_smoke", "daemon_cli"],
    "status module maps to smoke and cli shards",
)
assert_equal(
    resolve("crates/daemon/tests/integration/managed_bridge_fixtures.rs"),
    ["daemon_cli", "daemon_gateway", "daemon_onboard"],
    "managed bridge fixtures map to all dependent shards",
)
assert_equal(
    resolve("crates/daemon/tests/integration/spec_runtime_bridge/http_json.rs"),
    ["daemon_runtime"],
    "spec runtime bridge maps to runtime shard",
)
assert_equal(
    resolve("crates/daemon/tests/integration/future_new_suite.rs"),
    ["integration"],
    "unmapped integration files fall back to the full integration target",
)
assert_equal(
    resolve("crates/daemon/tests/daemon_smoke.rs"),
    ["daemon_smoke"],
    "daemon smoke entrypoint maps to the smoke shard itself",
)
assert_equal(
    resolve("crates/daemon/tests/integration/root_cli.rs"),
    ["daemon_smoke", "daemon_cli"],
    "root cli surface tests map to smoke and cli shards",
)
assert_equal(
    resolve("crates/daemon/tests/integration/channel_surfaces.rs"),
    ["daemon_cli"],
    "channel surface root tests map to the cli shard",
)
assert_equal(
    resolve("crates/daemon/tests/integration/memory_surfaces.rs"),
    ["daemon_cli"],
    "memory surface root tests map to the cli shard",
)
assert_equal(
    resolve("crates/daemon/tests/support/mod.rs"),
    [
        "daemon_smoke",
        "daemon_cli",
        "daemon_gateway",
        "daemon_onboard",
        "daemon_channels",
        "daemon_runtime",
        "integration",
    ],
    "support changes fan out to all daemon targets",
)
assert_equal(
    resolve("crates/daemon/Cargo.toml"),
    [
        "daemon_smoke",
        "daemon_cli",
        "daemon_gateway",
        "daemon_onboard",
        "daemon_channels",
        "daemon_runtime",
        "integration",
    ],
    "daemon manifest changes fan out to all daemon targets",
)

print("daemon_changed_test_targets checks passed")
PY
