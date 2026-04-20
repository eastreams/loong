#!/usr/bin/env python3
"""Resolve changed workspace Rust packages and their reverse-dependency closure.

Examples:
  python3 scripts/rust_changed_packages.py --format names crates/app/src/lib.rs
  python3 scripts/rust_changed_packages.py --format cargo-args Cargo.lock
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import deque
from pathlib import Path
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKSPACE_WIDE_PATHS = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "Cross.toml",
    "clippy.toml",
    "rustfmt.toml",
}
WORKSPACE_WIDE_PREFIXES = (".cargo/", "patches/")
EXAMPLE_PACKAGE_HINTS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("examples/benchmarks/", ("loong-bench", "loong")),
    ("examples/spec/", ("loong-spec", "loong-bench", "loong")),
    ("examples/plugins", ("loong-kernel", "loong-spec", "loong")),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "paths",
        nargs="*",
        help="Changed file paths relative to the repo root. Reads stdin when omitted.",
    )
    parser.add_argument(
        "--format",
        choices=("names", "cargo-args", "json"),
        default="names",
        help="Output format.",
    )
    parser.add_argument(
        "--no-dependents",
        action="store_true",
        help="Only return directly affected workspace packages.",
    )
    return parser.parse_args()


def load_workspace_metadata() -> dict:
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO_ROOT,
        text=True,
    )
    return json.loads(output)


def normalize_input_paths(raw_paths: Iterable[str]) -> list[str]:
    normalized: list[str] = []
    seen: set[str] = set()
    for raw in raw_paths:
        stripped = raw.strip()
        if not stripped:
            continue
        candidate = Path(stripped)
        if candidate.is_absolute():
            resolved = candidate.resolve(strict=False)
            try:
                candidate = resolved.relative_to(REPO_ROOT)
            except ValueError:
                candidate = resolved
        normalized_path = candidate.as_posix()
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

    return normalize_input_paths(sys.stdin.read().splitlines())


def build_workspace_graph(metadata: dict) -> tuple[list[str], dict[str, dict], dict[Path, str]]:
    packages = metadata["packages"]
    package_by_id = {package["id"]: package for package in packages}
    workspace_order = [package_by_id[member_id]["name"] for member_id in metadata["workspace_members"]]

    package_info: dict[str, dict] = {}
    package_root_to_name: dict[Path, str] = {}
    for package in packages:
        name = package["name"]
        root = Path(package["manifest_path"]).resolve().parent
        package_info[name] = {"root": root, "deps": set(), "rdeps": set()}
        package_root_to_name[root] = name

    for package in packages:
        name = package["name"]
        for dependency in package["dependencies"]:
            dependency_path = dependency.get("path")
            if dependency_path is None:
                continue
            dependency_root = Path(dependency_path).resolve()
            dependency_name = package_root_to_name.get(dependency_root)
            if dependency_name is None:
                continue
            package_info[name]["deps"].add(dependency_name)
            package_info[dependency_name]["rdeps"].add(name)

    return workspace_order, package_info, package_root_to_name


def path_affects_workspace(path: str) -> bool:
    if path in WORKSPACE_WIDE_PATHS:
        return True
    return any(path.startswith(prefix) for prefix in WORKSPACE_WIDE_PREFIXES)


def directly_affected_packages(
    changed_paths: list[str],
    workspace_order: list[str],
    package_info: dict[str, dict],
) -> tuple[set[str], bool]:
    if any(path_affects_workspace(path) for path in changed_paths):
        return set(workspace_order), True

    direct: set[str] = set()
    for path in changed_paths:
        for example_prefix, hinted_packages in EXAMPLE_PACKAGE_HINTS:
            if path.startswith(example_prefix):
                direct.update(hinted_packages)

        absolute_path = (REPO_ROOT / path).resolve(strict=False)
        for package_name in workspace_order:
            package_root = package_info[package_name]["root"]
            try:
                absolute_path.relative_to(package_root)
            except ValueError:
                continue
            direct.add(package_name)
            break

    return direct, False


def reverse_dependency_closure(direct: set[str], package_info: dict[str, dict]) -> set[str]:
    closure = set(direct)
    queue = deque(direct)
    while queue:
        current = queue.popleft()
        for dependent in package_info[current]["rdeps"]:
            if dependent in closure:
                continue
            closure.add(dependent)
            queue.append(dependent)
    return closure


def ordered_subset(package_names: set[str], workspace_order: list[str]) -> list[str]:
    return [package_name for package_name in workspace_order if package_name in package_names]


def main() -> int:
    args = parse_args()
    changed_paths = load_changed_paths(args)
    metadata = load_workspace_metadata()
    workspace_order, package_info, _ = build_workspace_graph(metadata)
    direct, all_selected = directly_affected_packages(changed_paths, workspace_order, package_info)
    selected = direct if args.no_dependents else reverse_dependency_closure(direct, package_info)

    ordered_direct = ordered_subset(direct, workspace_order)
    ordered_selected = ordered_subset(selected, workspace_order)

    if args.format == "names":
        for package_name in ordered_selected:
            print(package_name)
        return 0

    if args.format == "cargo-args":
        print(" ".join(f"-p {package_name}" for package_name in ordered_selected))
        return 0

    print(
        json.dumps(
            {
                "input_paths": changed_paths,
                "direct": ordered_direct,
                "selected": ordered_selected,
                "all_selected": all_selected,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
