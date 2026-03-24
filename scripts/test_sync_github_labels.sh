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

assert_not_contains_regex() {
  local file="$1"
  local pattern="$2"
  if grep -Eq "$pattern" "$file"; then
    echo "did not expect to match /$pattern/ in $file" >&2
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

python3 - "$SYNC_SCRIPT" <<'PY'
import contextlib
import importlib.util
import io
import sys
import tempfile
from pathlib import Path

script_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("sync_github_labels", script_path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

tmpdir = Path(tempfile.mkdtemp(prefix="sync-github-labels-"))
missing_target = tmpdir / "missing-target.yml"
stderr = io.StringIO()
with contextlib.redirect_stderr(stderr):
    result = module.check_targets({missing_target: "expected\n"})

output = stderr.getvalue()
if result != 1:
    print(f"expected missing target check to return 1, got {result}", file=sys.stderr)
    sys.exit(1)
if str(missing_target) not in output:
    print(f"expected missing target path in stderr, got: {output!r}", file=sys.stderr)
    sys.exit(1)
if "out of date" not in output:
    print(f"expected mismatch summary in stderr, got: {output!r}", file=sys.stderr)
    sys.exit(1)
PY

python3 - "$SYNC_SCRIPT" <<'PY'
import importlib.util
import sys
from pathlib import Path

script_path = Path(sys.argv[1])
repo_root = script_path.parents[1]
spec = importlib.util.spec_from_file_location("sync_github_labels", script_path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

taxonomy = module.load_taxonomy(repo_root)
failures = module.check_semantic_regression_cases(taxonomy)

if failures:
    for failure in failures:
        print(failure, file=sys.stderr)
    sys.exit(1)
PY

assert_not_contains_regex "$REPO_ROOT/.github/labeler.yml" '(^|[[:space:]])"?rust"?[[:space:]]*:'
assert_not_contains "$REPO_ROOT/docs/references/github-collaboration.md" "area:"
assert_not_contains "$REPO_ROOT/docs/references/github-collaboration.md" "domain:"
assert_not_contains "$REPO_ROOT/docs/plans/2026-03-19-github-label-taxonomy-implementation-plan.md" "For Claude:"
assert_contains "$REPO_ROOT/docs/references/github-collaboration.md" "## CI and Promotion Gates"
assert_contains "$REPO_ROOT/docs/references/github-collaboration.md" 'release` and `release/*` branches are optional release-hardening lanes'
assert_contains "$REPO_ROOT/docs/references/github-collaboration.md" "enforce-dev-to-main"
assert_contains "$REPO_ROOT/.github/ISSUE_TEMPLATE/bug_report.yml" "label: Surface"
assert_contains "$REPO_ROOT/.github/ISSUE_TEMPLATE/feature_request.yml" "label: Surface"
assert_contains "$REPO_ROOT/.github/ISSUE_TEMPLATE/docs_improvement.yml" "label: Surface"
assert_contains "$REPO_ROOT/.github/workflows/labeler.yml" "const legacyLabels = ["

echo "sync_github_labels checks passed"
