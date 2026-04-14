#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

assert_guard_condition() {
  local workflow_path="$1"
  local _job_name="$2"
  local expected_ref="$3"
  local expected_condition

  expected_condition="if: \${{ github.event.pull_request.head.repo.full_name != github.repository || github.event.pull_request.head.ref != '${expected_ref}' }}"

  if ! grep -Fq -- "$expected_condition" "$workflow_path"; then
    echo "workflow $workflow_path is missing expected guard condition: $expected_condition" >&2
    cat "$workflow_path" >&2
    exit 1
  fi
}

cd "$REPO_ROOT"

assert_guard_condition ".github/workflows/enforce-dev-to-main.yml" "block-non-dev-source" "dev"
assert_guard_condition ".github/workflows/enforce-main-to-release.yml" "block-non-main-source" "main"

echo "promotion guard workflow checks passed"
