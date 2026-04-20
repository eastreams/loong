#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ERRORS=0

fail() {
    local message="$1"
    echo "FAIL: ${message}"
    ERRORS=$((ERRORS + 1))
}

check_file_exists() {
    local file="$1"

    if [ -f "$file" ]; then
        return
    fi

    fail "missing required governance doc or source file: ${file#"$REPO_ROOT/"}"
}

check_absent_fixed_string() {
    local file="$1"
    local needle="$2"
    local message="$3"

    if grep -Fq "$needle" "$file"; then
        fail "$message"
    fi
}

check_present_fixed_string() {
    local file="$1"
    local needle="$2"
    local message="$3"

    if grep -Fq "$needle" "$file"; then
        return
    fi

    fail "$message"
}

count_audit_event_kinds() {
    local file="$1"

    awk '
        /pub enum AuditEventKind \{/ {
            in_enum = 1
            next
        }

        in_enum && /^}/ {
            in_enum = 0
        }

        in_enum && /^[[:space:]]{4}[A-Za-z0-9_]+[[:space:]]*(\{|,)/ {
            count += 1
        }

        END {
            print count + 0
        }
    ' "$file"
}

read_documented_audit_event_kind_count() {
    local file="$1"

    sed -n -E 's/^- ([0-9]+) event kinds with atomic sequencing$/\1/p' "$file" | head -n 1
}

ROADMAP_FILE="$REPO_ROOT/docs/ROADMAP.md"
SECURITY_FILE="$REPO_ROOT/docs/SECURITY.md"
QUALITY_SCORE_FILE="$REPO_ROOT/docs/QUALITY_SCORE.md"
LAYERED_DESIGN_FILE="$REPO_ROOT/docs/design-docs/layered-kernel-design.md"
DESIGN_INDEX_FILE="$REPO_ROOT/docs/design-docs/index.md"
AUDIT_TYPES_FILE="$REPO_ROOT/crates/contracts/src/audit_types.rs"

check_file_exists "$ROADMAP_FILE"
check_file_exists "$SECURITY_FILE"
check_file_exists "$QUALITY_SCORE_FILE"
check_file_exists "$LAYERED_DESIGN_FILE"
check_file_exists "$DESIGN_INDEX_FILE"
check_file_exists "$AUDIT_TYPES_FILE"

check_absent_fixed_string \
    "$ROADMAP_FILE" \
    'kernel-level tool-call policy gate (`PolicyEngine::check_tool_call`)' \
    "docs/ROADMAP.md still describes PolicyEngine::check_tool_call as the live tool-policy seam"

check_absent_fixed_string \
    "$LAYERED_DESIGN_FILE" \
    'must call `PolicyEngine::check_tool_call` before dispatch' \
    "docs/design-docs/layered-kernel-design.md still describes PolicyEngine::check_tool_call as the live tool-policy seam"

check_present_fixed_string \
    "$SECURITY_FILE" \
    'PolicyExtensionChain' \
    "docs/SECURITY.md must describe the live PolicyExtensionChain tool-policy seam"

check_present_fixed_string \
    "$LAYERED_DESIGN_FILE" \
    'PolicyExtensionChain' \
    "docs/design-docs/layered-kernel-design.md must describe the policy extension chain as the live request-policy seam"

check_absent_fixed_string \
    "$QUALITY_SCORE_FILE" \
    'file.read`/`file.write` bypass policy check' \
    "docs/QUALITY_SCORE.md still claims file tools bypass policy checks"

check_absent_fixed_string \
    "$QUALITY_SCORE_FILE" \
    'Audit events in-memory only' \
    "docs/QUALITY_SCORE.md still claims observability is in-memory only"

check_absent_fixed_string \
    "$DESIGN_INDEX_FILE" \
    'audit events exist, in-memory only' \
    "docs/design-docs/index.md still claims the audit system is in-memory only"

implemented_audit_event_kinds="$(count_audit_event_kinds "$AUDIT_TYPES_FILE")"
documented_audit_event_kinds="$(read_documented_audit_event_kind_count "$SECURITY_FILE")"

if [ -z "$documented_audit_event_kinds" ]; then
    fail "docs/SECURITY.md must declare the number of audit event kinds"
elif [ "$implemented_audit_event_kinds" != "$documented_audit_event_kinds" ]; then
    fail \
        "docs/SECURITY.md says there are ${documented_audit_event_kinds} audit event kinds, but crates/contracts/src/audit_types.rs defines ${implemented_audit_event_kinds}"
fi

if [ "$ERRORS" -gt 0 ]; then
    echo ""
    echo "FAILED: $ERRORS governance doc consistency error(s)"
    exit 1
fi

echo "Governance doc consistency checks passed."
