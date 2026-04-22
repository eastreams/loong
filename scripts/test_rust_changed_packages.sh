#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

python3 - <<'PY'
import json
import subprocess


def resolve(*paths: str) -> dict:
    output = subprocess.check_output(
        ["python3", "scripts/rust_changed_packages.py", "--format", "json", *paths],
        text=True,
    )
    return json.loads(output)


def assert_equal(actual, expected, label: str) -> None:
    if actual != expected:
        raise SystemExit(f"{label}: expected {expected!r}, got {actual!r}")

app = resolve("crates/app/src/lib.rs")
assert_equal(app["direct"], ["loong-app"], "app direct")
assert_equal(app["selected"], ["loong-app", "loong"], "app closure")

bench = resolve("crates/bench/src/lib.rs")
assert_equal(bench["direct"], ["loong-bench"], "bench direct")
assert_equal(bench["selected"], ["loong-bench", "loong"], "bench closure")

protocol = resolve("crates/protocol/src/lib.rs")
assert_equal(protocol["direct"], ["loong-protocol"], "protocol direct")
assert_equal(
    protocol["selected"],
    ["loong-protocol", "loong-bridge-runtime", "loong-spec", "loong-bench", "loong"],
    "protocol closure",
)

contracts = resolve("crates/contracts/src/lib.rs")
assert_equal(contracts["direct"], ["loong-contracts"], "contracts direct")
assert_equal(
    contracts["selected"],
    [
        "loong-contracts",
        "loong-kernel",
        "loong-bridge-runtime",
        "loong-spec",
        "loong-bench",
        "loong-app",
        "loong",
    ],
    "contracts closure",
)

lockfile = resolve("Cargo.lock")
assert_equal(
    lockfile["selected"],
    [
        "loong-contracts",
        "loong-kernel",
        "loong-protocol",
        "loong-bridge-runtime",
        "loong-spec",
        "loong-bench",
        "loong-app",
        "loong",
    ],
    "lockfile selects all packages",
)
if not lockfile["all_selected"]:
    raise SystemExit("lockfile should force all packages")

cargo_config = resolve(".cargo/config.toml")
assert_equal(cargo_config["selected"], lockfile["selected"], ".cargo config selects all packages")
if not cargo_config["all_selected"]:
    raise SystemExit(".cargo/config.toml should force all packages")

ignored = resolve("docs/RELIABILITY.md")
assert_equal(ignored["selected"], [], "docs file should not select packages")

print("rust_changed_packages checks passed")
PY
