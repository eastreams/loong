#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="$(realpath "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"

assert_contains() {
    local file="$1"
    local needle="$2"

    if grep -Fq "$needle" "$file"; then
        return
    fi

    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
}

make_fixture_repo() {
    local fixture
    fixture="$(mktemp -d)"

    mkdir -p \
        "$fixture/scripts" \
        "$fixture/docs/design-docs" \
        "$fixture/crates/contracts/src"

    cp "$REPO_ROOT/scripts/check_governance_docs_consistency.sh" \
        "$fixture/scripts/check_governance_docs_consistency.sh"
    cp "$REPO_ROOT/docs/ROADMAP.md" "$fixture/docs/ROADMAP.md"
    cp "$REPO_ROOT/docs/SECURITY.md" "$fixture/docs/SECURITY.md"
    cp "$REPO_ROOT/docs/QUALITY_SCORE.md" "$fixture/docs/QUALITY_SCORE.md"
    cp "$REPO_ROOT/docs/design-docs/layered-kernel-design.md" \
        "$fixture/docs/design-docs/layered-kernel-design.md"
    cp "$REPO_ROOT/docs/design-docs/index.md" \
        "$fixture/docs/design-docs/index.md"
    cp "$REPO_ROOT/crates/contracts/src/audit_types.rs" \
        "$fixture/crates/contracts/src/audit_types.rs"

    chmod +x "$fixture/scripts/check_governance_docs_consistency.sh"

    printf '%s\n' "$fixture"
}

run_happy_path_test() {
    local fixture
    local output_file

    fixture="$(make_fixture_repo)"
    output_file="$fixture/happy.out"
    trap 'rm -rf "$fixture"' RETURN

    (
        cd "$fixture"
        scripts/check_governance_docs_consistency.sh >"$output_file"
    )

    assert_contains "$output_file" "Governance doc consistency checks passed."
}

run_stale_policy_hook_test() {
    local fixture
    local output_file

    fixture="$(make_fixture_repo)"
    output_file="$fixture/stale-policy.out"
    trap 'rm -rf "$fixture"' RETURN

    cat <<'EOF_APPEND' >> "$fixture/docs/ROADMAP.md"
- kernel-level tool-call policy gate (`PolicyEngine::check_tool_call`) with explicit deny/approval-required outcomes before tool dispatch
EOF_APPEND

    if (
        cd "$fixture"
        scripts/check_governance_docs_consistency.sh >"$output_file" 2>&1
    ); then
        echo "expected governance doc consistency check to fail on stale policy-hook wording" >&2
        cat "$output_file" >&2
        exit 1
    fi

    assert_contains "$output_file" "docs/ROADMAP.md still describes PolicyEngine::check_tool_call as the live tool-policy seam"
}

run_audit_event_count_mismatch_test() {
    local fixture
    local output_file

    fixture="$(make_fixture_repo)"
    output_file="$fixture/audit-count.out"
    trap 'rm -rf "$fixture"' RETURN

    python3 - "$fixture/docs/SECURITY.md" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
updated = text.replace("- 10 event kinds with atomic sequencing", "- 7 event kinds with atomic sequencing")
path.write_text(updated)
PY

    if (
        cd "$fixture"
        scripts/check_governance_docs_consistency.sh >"$output_file" 2>&1
    ); then
        echo "expected governance doc consistency check to fail on audit event count mismatch" >&2
        cat "$output_file" >&2
        exit 1
    fi

    assert_contains "$output_file" "docs/SECURITY.md says there are 7 audit event kinds"
}

run_happy_path_test
run_stale_policy_hook_test
run_audit_event_count_mismatch_test

echo "governance doc consistency script checks passed"
