#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SYNC_SCRIPT="$REPO_ROOT/scripts/sync_github_labels.py"
TAXONOMY_FILE="$REPO_ROOT/.github/label_taxonomy.json"

assert_not_contains() {
  local file="$1"
  local needle="$2"
  if grep -Fq "$needle" "$file"; then
    echo "did not expect to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

[[ -f "$SYNC_SCRIPT" ]] || {
  echo "expected sync script at $SYNC_SCRIPT" >&2
  exit 1
}

[[ -f "$TAXONOMY_FILE" ]] || {
  echo "expected taxonomy file at $TAXONOMY_FILE" >&2
  exit 1
}

python3 "$SYNC_SCRIPT" --check

python3 - "$TAXONOMY_FILE" <<'PY'
import json
import sys
from pathlib import Path

taxonomy = json.loads(Path(sys.argv[1]).read_text())
managed_names = []
for group in ("surfaces", "domains", "general_labels", "size_labels"):
    managed_names.extend(entry["name"] for entry in taxonomy[group])

bad_names = [
    name for name in managed_names
    if name.startswith("area:") or name.startswith("domain:") or name == "rust"
]
if bad_names:
    print(f"managed label names must be unprefixed and rust-free, found: {bad_names}", file=sys.stderr)
    sys.exit(1)
PY

assert_not_contains "$REPO_ROOT/.github/labeler.yml" "\"rust\":"
assert_not_contains "$REPO_ROOT/docs/references/github-collaboration.md" "area:"
assert_not_contains "$REPO_ROOT/docs/references/github-collaboration.md" "domain:"
assert_contains "$REPO_ROOT/.github/ISSUE_TEMPLATE/bug_report.yml" "label: Surface"
assert_contains "$REPO_ROOT/.github/ISSUE_TEMPLATE/feature_request.yml" "label: Surface"
assert_contains "$REPO_ROOT/.github/ISSUE_TEMPLATE/docs_improvement.yml" "label: Surface"
assert_contains "$REPO_ROOT/.github/workflows/labeler.yml" "const legacyLabels = ["

echo "sync_github_labels checks passed"
