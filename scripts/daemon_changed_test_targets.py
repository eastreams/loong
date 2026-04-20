#!/usr/bin/env python3
"""Resolve daemon test targets for changed daemon test files.

Examples:
  python3 scripts/daemon_changed_test_targets.py crates/daemon/tests/integration/onboard_cli.rs
  git diff --name-only | python3 scripts/daemon_changed_test_targets.py --format json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parent.parent
INTEGRATION_PREFIX = "crates/daemon/tests/integration/"
TARGET_ORDER = [
    "daemon_smoke",
    "daemon_cli",
    "daemon_gateway",
    "daemon_onboard",
    "daemon_channels",
    "daemon_runtime",
    "integration",
]
BROAD_DAEMON_TEST_PATHS = {
    "crates/daemon/Cargo.toml",
    "crates/daemon/tests/integration.rs",
    "crates/daemon/tests/integration/mod.rs",
    "crates/daemon/tests/support.rs",
}
TARGET_PREFIXES = {
    "daemon_smoke": (
        "crates/daemon/tests/daemon_smoke.rs",
        "crates/daemon/tests/integration/architecture.rs",
        "crates/daemon/tests/integration/cli_tests.rs",
        "crates/daemon/tests/integration/gateway_api_health.rs",
        "crates/daemon/tests/integration/gateway_read_models.rs",
        "crates/daemon/tests/integration/runtime_snapshot_cli.rs",
        "crates/daemon/tests/integration/status_cli.rs",
        "crates/daemon/tests/integration/work_unit_cli.rs",
    ),
    "daemon_cli": (
        "crates/daemon/tests/daemon_cli.rs",
        "crates/daemon/tests/integration/ask_cli.rs",
        "crates/daemon/tests/integration/chat_cli.rs",
        "crates/daemon/tests/integration/cli_tests.rs",
        "crates/daemon/tests/integration/latest_selector_process_support.rs",
        "crates/daemon/tests/integration/mcp.rs",
        "crates/daemon/tests/integration/memory_context_benchmark_cli.rs",
        "crates/daemon/tests/integration/personalize_cli.rs",
        "crates/daemon/tests/integration/plugins_cli.rs",
        "crates/daemon/tests/integration/session_search_cli.rs",
        "crates/daemon/tests/integration/sessions_cli.rs",
        "crates/daemon/tests/integration/skills_cli.rs",
        "crates/daemon/tests/integration/status_cli.rs",
        "crates/daemon/tests/integration/tasks_cli.rs",
    ),
    "daemon_gateway": (
        "crates/daemon/tests/daemon_gateway.rs",
        "crates/daemon/tests/integration/architecture.rs",
        "crates/daemon/tests/integration/gateway_api_acp.rs",
        "crates/daemon/tests/integration/gateway_api_events.rs",
        "crates/daemon/tests/integration/gateway_api_health.rs",
        "crates/daemon/tests/integration/gateway_api_turn.rs",
        "crates/daemon/tests/integration/gateway_owner_state.rs",
        "crates/daemon/tests/integration/gateway_read_models.rs",
        "crates/daemon/tests/integration/logging.rs",
        "crates/daemon/tests/integration/managed_bridge_fixtures.rs",
    ),
    "daemon_onboard": (
        "crates/daemon/tests/daemon_onboard.rs",
        "crates/daemon/tests/integration/import_cli.rs",
        "crates/daemon/tests/integration/managed_bridge_fixtures.rs",
        "crates/daemon/tests/integration/managed_bridge_parity.rs",
        "crates/daemon/tests/integration/migrate_cli.rs",
        "crates/daemon/tests/integration/migration.rs",
        "crates/daemon/tests/integration/onboard_cli.rs",
    ),
    "daemon_channels": (
        "crates/daemon/tests/daemon_channels.rs",
        "crates/daemon/tests/integration/doctor_feishu.rs",
        "crates/daemon/tests/integration/feishu_cli.rs",
        "crates/daemon/tests/integration/multi_channel_serve_cli.rs",
    ),
    "daemon_runtime": (
        "crates/daemon/tests/daemon_runtime.rs",
        "crates/daemon/tests/integration/acp.rs",
        "crates/daemon/tests/integration/programmatic.rs",
        "crates/daemon/tests/integration/runtime_capability_cli.rs",
        "crates/daemon/tests/integration/runtime_experiment_cli.rs",
        "crates/daemon/tests/integration/runtime_restore_cli.rs",
        "crates/daemon/tests/integration/runtime_snapshot_cli.rs",
        "crates/daemon/tests/integration/runtime_trajectory_cli.rs",
        "crates/daemon/tests/integration/spec_runtime.rs",
        "crates/daemon/tests/integration/spec_runtime_bridge/",
        "crates/daemon/tests/integration/trajectory_export_cli.rs",
        "crates/daemon/tests/integration/work_unit_cli.rs",
    ),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "paths",
        nargs="*",
        help="Changed file paths relative to the repo root. Reads stdin when omitted.",
    )
    parser.add_argument(
        "--format",
        choices=("names", "json"),
        default="names",
        help="Output format.",
    )
    return parser.parse_args()


def normalize_input_paths(raw_paths: Iterable[str]) -> list[str]:
    normalized: list[str] = []
    seen: set[str] = set()

    for raw_path in raw_paths:
        stripped_path = raw_path.strip()
        if not stripped_path:
            continue

        candidate_path = Path(stripped_path)
        if candidate_path.is_absolute():
            resolved_path = candidate_path.resolve(strict=False)
            try:
                relative_path = resolved_path.relative_to(REPO_ROOT)
            except ValueError:
                relative_path = resolved_path
            candidate_path = relative_path

        normalized_path = candidate_path.as_posix()
        if normalized_path.startswith("./"):
            normalized_path = normalized_path[2:]
        if normalized_path in seen:
            continue

        seen.add(normalized_path)
        normalized.append(normalized_path)

    return normalized


def load_changed_paths(args: argparse.Namespace) -> list[str]:
    if args.paths:
        return normalize_input_paths(args.paths)

    if sys.stdin.isatty():
        return []

    stdin_lines = sys.stdin.read().splitlines()
    return normalize_input_paths(stdin_lines)


def path_matches_any_prefix(path: str, prefixes: tuple[str, ...]) -> bool:
    for prefix in prefixes:
        if path.startswith(prefix):
            return True
    return False


def ordered_targets(selected_targets: set[str]) -> list[str]:
    ordered: list[str] = []
    for target in TARGET_ORDER:
        if target not in selected_targets:
            continue
        ordered.append(target)
    return ordered


def resolve_targets(changed_paths: list[str]) -> list[str]:
    selected_targets: set[str] = set()

    for changed_path in changed_paths:
        if changed_path in BROAD_DAEMON_TEST_PATHS:
            return TARGET_ORDER.copy()

        matched_known_target = False
        for target_name, prefixes in TARGET_PREFIXES.items():
            if not path_matches_any_prefix(changed_path, prefixes):
                continue
            selected_targets.add(target_name)
            matched_known_target = True

        if matched_known_target:
            continue

        is_unmapped_integration_path = changed_path.startswith(INTEGRATION_PREFIX)
        if is_unmapped_integration_path:
            selected_targets.add("integration")

    return ordered_targets(selected_targets)


def main() -> int:
    args = parse_args()
    changed_paths = load_changed_paths(args)
    targets = resolve_targets(changed_paths)

    if args.format == "names":
        for target in targets:
            print(target)
        return 0

    payload = {
        "input_paths": changed_paths,
        "targets": targets,
    }
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
