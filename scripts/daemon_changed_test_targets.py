#!/usr/bin/env python3
"""Resolve daemon test targets for changed daemon test files.

Examples:
  python3 scripts/daemon_changed_test_targets.py crates/daemon/tests/integration/onboard_cli.rs
  git diff --name-only | python3 scripts/daemon_changed_test_targets.py --format json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib

REPO_ROOT = Path(__file__).resolve().parent.parent
DAEMON_CRATE_DIR = REPO_ROOT / "crates/daemon"
DAEMON_TESTS_DIR = DAEMON_CRATE_DIR / "tests"
INTEGRATION_DIR = DAEMON_TESTS_DIR / "integration"
CARGO_TOML_PATH = DAEMON_CRATE_DIR / "Cargo.toml"
INTEGRATION_TARGET_NAME = "integration"
INTEGRATION_PREFIX = "crates/daemon/tests/integration/"
SUPPORT_PREFIX = "crates/daemon/tests/support/"
PATH_ATTRIBUTE_RE = re.compile(r'^#\[path = "([^"]+)"\]$')
MODULE_DECLARATION_RE = re.compile(r"^mod ([A-Za-z0-9_]+);$")
INTEGRATION_BLOCK_RE = re.compile(r"^mod integration \{$")


@dataclass(frozen=True)
class ShardTarget:
    name: str
    source_path: str
    dependency_prefixes: tuple[str, ...]

    def matches_path(self, changed_path: str) -> bool:
        for prefix in self.dependency_prefixes:
            if prefix.endswith("/"):
                if changed_path.startswith(prefix):
                    return True
                continue

            if changed_path == prefix:
                return True

        return False


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


def repo_relative_path(path: Path) -> str:
    resolved_path = path.resolve(strict=False)
    relative_path = resolved_path.relative_to(REPO_ROOT)
    return relative_path.as_posix()


def add_unique_prefix(prefixes: list[str], seen: set[str], prefix: str) -> None:
    if prefix in seen:
        return

    seen.add(prefix)
    prefixes.append(prefix)


def resolve_module_dependency_prefixes(base_dir: Path, module_name: str, path_attribute: str | None) -> list[str]:
    if path_attribute is not None:
        candidate_path = base_dir / path_attribute
        relative_path = repo_relative_path(candidate_path)
        return [relative_path]

    file_candidate = base_dir / f"{module_name}.rs"
    if file_candidate.exists():
        relative_path = repo_relative_path(file_candidate)
        return [relative_path]

    directory_candidate = base_dir / module_name
    directory_module = directory_candidate / "mod.rs"
    if directory_module.exists():
        relative_path = repo_relative_path(directory_candidate)
        directory_prefix = f"{relative_path}/"
        return [directory_prefix]

    return []


def discover_shard_dependency_prefixes(shard_source_path: Path) -> tuple[str, ...]:
    prefixes: list[str] = []
    seen_prefixes: set[str] = set()
    shard_source_relative = repo_relative_path(shard_source_path)
    add_unique_prefix(prefixes, seen_prefixes, shard_source_relative)

    pending_path_attribute: str | None = None
    inside_integration_block = False
    integration_block_balance = 0

    shard_source_lines = shard_source_path.read_text().splitlines()
    for line in shard_source_lines:
        stripped_line = line.strip()

        if not inside_integration_block and INTEGRATION_BLOCK_RE.match(stripped_line):
            inside_integration_block = True
            opening_braces = stripped_line.count("{")
            closing_braces = stripped_line.count("}")
            integration_block_balance = opening_braces - closing_braces
            continue

        path_match = PATH_ATTRIBUTE_RE.match(stripped_line)
        if path_match is not None:
            pending_path_attribute = path_match.group(1)
            continue

        module_match = MODULE_DECLARATION_RE.match(stripped_line)
        if module_match is not None:
            module_name = module_match.group(1)
            if inside_integration_block:
                module_base_dir = INTEGRATION_DIR
            else:
                module_base_dir = DAEMON_TESTS_DIR

            dependency_prefixes = resolve_module_dependency_prefixes(
                module_base_dir,
                module_name,
                pending_path_attribute,
            )
            for dependency_prefix in dependency_prefixes:
                add_unique_prefix(prefixes, seen_prefixes, dependency_prefix)
            pending_path_attribute = None

        if inside_integration_block:
            opening_braces = stripped_line.count("{")
            closing_braces = stripped_line.count("}")
            integration_block_balance += opening_braces - closing_braces
            if integration_block_balance == 0:
                inside_integration_block = False

    return tuple(prefixes)


def load_shard_targets() -> list[ShardTarget]:
    cargo_toml_bytes = CARGO_TOML_PATH.read_bytes()
    cargo_manifest = tomllib.loads(cargo_toml_bytes.decode())
    test_entries = cargo_manifest.get("test", [])

    shard_targets: list[ShardTarget] = []
    for test_entry in test_entries:
        required_features = test_entry.get("required-features", [])
        if "test-support" not in required_features:
            continue

        target_name = test_entry.get("name")
        source_path = test_entry.get("path")
        if not target_name or not source_path:
            continue

        shard_source_path = DAEMON_CRATE_DIR / source_path
        shard_source_relative = repo_relative_path(shard_source_path)
        dependency_prefixes = discover_shard_dependency_prefixes(shard_source_path)
        shard_target = ShardTarget(
            name=target_name,
            source_path=shard_source_relative,
            dependency_prefixes=dependency_prefixes,
        )
        shard_targets.append(shard_target)

    return shard_targets


def ordered_target_names(shard_targets: list[ShardTarget]) -> list[str]:
    ordered_names: list[str] = []
    for shard_target in shard_targets:
        ordered_names.append(shard_target.name)
    ordered_names.append(INTEGRATION_TARGET_NAME)
    return ordered_names


def is_broad_daemon_test_path(changed_path: str) -> bool:
    broad_exact_paths = {
        repo_relative_path(CARGO_TOML_PATH),
        repo_relative_path(DAEMON_TESTS_DIR / "integration.rs"),
        repo_relative_path(INTEGRATION_DIR / "mod.rs"),
    }
    if changed_path in broad_exact_paths:
        return True

    if changed_path.startswith(SUPPORT_PREFIX):
        return True

    return False


def ordered_subset(selected_targets: set[str], ordered_names: list[str]) -> list[str]:
    ordered_selection: list[str] = []
    for target_name in ordered_names:
        if target_name not in selected_targets:
            continue
        ordered_selection.append(target_name)
    return ordered_selection


def resolve_targets(changed_paths: list[str]) -> list[str]:
    shard_targets = load_shard_targets()
    ordered_names = ordered_target_names(shard_targets)
    selected_targets: set[str] = set()

    for changed_path in changed_paths:
        if is_broad_daemon_test_path(changed_path):
            return ordered_names

        matched_shard = False
        for shard_target in shard_targets:
            if not shard_target.matches_path(changed_path):
                continue
            selected_targets.add(shard_target.name)
            matched_shard = True

        if matched_shard:
            continue

        if changed_path.startswith(INTEGRATION_PREFIX):
            selected_targets.add(INTEGRATION_TARGET_NAME)

    return ordered_subset(selected_targets, ordered_names)


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
