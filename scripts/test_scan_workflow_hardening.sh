#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq -- "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

cd "$REPO_ROOT"

assert_contains ".github/workflows/security.yml" "cancel-in-progress: true"
assert_contains ".github/workflows/security.yml" "workflow_dispatch:"
assert_contains ".github/workflows/security.yml" "merge_group:"
assert_contains ".github/workflows/security.yml" "timeout-minutes: 20"
assert_contains ".github/workflows/security.yml" "contents: read"
assert_contains ".github/workflows/security.yml" "run_advisory_checks: \${{ steps.classify.outputs.run_advisory_checks }}"
assert_contains ".github/workflows/security.yml" "if: \${{ needs.changes.outputs.run_advisory_checks == 'true' }}"
assert_contains ".github/workflows/security.yml" "scripts/workflow_change_router.mjs"
assert_contains ".github/workflows/security.yml" "cargo deny check advisories bans licenses sources"
assert_contains ".github/workflows/security.yml" 'core.info(`Security changed paths: ${touchedPathSummary || "<full-validation>"}`)'

assert_contains ".github/workflows/codeql.yml" "cancel-in-progress: true"
assert_contains ".github/workflows/codeql.yml" "merge_group:"
assert_contains ".github/workflows/codeql.yml" "run_analysis: \${{ steps.classify.outputs.run_analysis }}"
assert_contains ".github/workflows/codeql.yml" "if: \${{ needs.changes.outputs.run_analysis == 'true' }}"
assert_contains ".github/workflows/codeql.yml" "scripts/workflow_change_router.mjs"
assert_contains ".github/workflows/codeql.yml" 'core.info(`CodeQL changed paths: ${touchedPathSummary || "<full-validation>"}`)'

echo "scan workflow hardening checks passed"
