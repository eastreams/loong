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

assert_not_contains() {
  local file="$1"
  local needle="$2"
  if grep -Fq -- "$needle" "$file"; then
    echo "did not expect to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

WORKFLOW_PATH="$REPO_ROOT/.github/workflows/ci.yml"

assert_contains "$WORKFLOW_PATH" "run_rust_jobs: \${{ steps.classify.outputs.run_rust_jobs }}"
assert_contains "$WORKFLOW_PATH" "run_docs_site: \${{ steps.classify.outputs.run_docs_site }}"
assert_contains "$WORKFLOW_PATH" "touched_paths_json: \${{ steps.classify.outputs.touched_paths_json }}"
assert_contains "$WORKFLOW_PATH" "merge_group:"
assert_contains "$WORKFLOW_PATH" "checks_requested"
assert_contains "$WORKFLOW_PATH" "scripts/workflow_change_router.mjs"
assert_contains "$WORKFLOW_PATH" "if: \${{ needs.changes.outputs.run_rust_jobs == 'true' }}"
assert_contains "$WORKFLOW_PATH" "if: \${{ needs.changes.outputs.run_docs_site == 'true' }}"
assert_contains "$WORKFLOW_PATH" "TOUCHED_PATHS_JSON: \${{ needs.changes.outputs.touched_paths_json }}"
assert_contains "$WORKFLOW_PATH" "check_optional_result()"
assert_contains "$WORKFLOW_PATH" 'check_result changes "$CHANGES_RESULT"'
assert_contains "$WORKFLOW_PATH" "Expected success or skipped"
assert_contains "$WORKFLOW_PATH" 'core.info(`CI changed paths: ${touchedPathSummary || "<full-validation>"}`)'
assert_contains "$WORKFLOW_PATH" "./scripts/run_changed_rust_ci_job.sh"
assert_contains "$WORKFLOW_PATH" "./scripts/run_changed_rust_ci_job.sh --all-features"
assert_not_contains "$WORKFLOW_PATH" 'matrix.os }}" == "ubuntu-latest"'

echo "ci workflow change routing checks passed"
