#!/usr/bin/env bash
set -euo pipefail

# Doc governance checks — validates mirror consistency, dead links, and stale plans.
# Referenced by: task check:docs (Taskfile.yml)

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ERRORS=0

# --- 1. CLAUDE.md / AGENTS.md mirror check ---
if ! diff -q "$REPO_ROOT/CLAUDE.md" "$REPO_ROOT/AGENTS.md" > /dev/null 2>&1; then
    echo "FAIL: CLAUDE.md and AGENTS.md are not mirrored"
    ERRORS=$((ERRORS + 1))
else
    echo "OK: CLAUDE.md == AGENTS.md"
fi

# --- 2. Dead internal links in docs/ ---
DEAD_LINK_FILE=$(mktemp)
trap 'rm -f "$DEAD_LINK_FILE"' EXIT

find "$REPO_ROOT/docs" "$REPO_ROOT/CLAUDE.md" "$REPO_ROOT/AGENTS.md" "$REPO_ROOT/ARCHITECTURE.md" -name '*.md' 2>/dev/null | while IFS= read -r md_file; do
    dir="$(dirname "$md_file")"
    # Extract markdown links: [text](path) — skip http/https/mailto/anchor-only links
    grep -oE '\]\([^)]+\)' "$md_file" 2>/dev/null | \
        sed 's/^\]//' | sed 's/)$//' | sed 's/^(//' | \
        grep -v '^http' | grep -v '^mailto' | grep -v '^#' | \
        sed 's/#.*//' | \
    while IFS= read -r link; do
        [ -z "$link" ] && continue
        target="$dir/$link"
        if [ ! -e "$target" ]; then
            echo "DEAD LINK: $md_file -> $link"
            echo "1" >> "$DEAD_LINK_FILE"
        fi
    done || true
done || true

DEAD_LINKS=$(wc -l < "$DEAD_LINK_FILE" 2>/dev/null | tr -d ' ')
if [ "$DEAD_LINKS" -gt 0 ]; then
    ERRORS=$((ERRORS + DEAD_LINKS))
else
    echo "OK: No dead internal links"
fi

# --- 3. Stale plans check (active plans older than 30 days) ---
ACTIVE_DIR="$REPO_ROOT/docs/exec-plans/active"
STALE=0
if [ -d "$ACTIVE_DIR" ]; then
    while IFS= read -r plan; do
        [ -z "$plan" ] && continue
        age_days=$(( ( $(date +%s) - $(stat -f %m "$plan" 2>/dev/null || stat -c %Y "$plan" 2>/dev/null || echo 0) ) / 86400 ))
        if [ "$age_days" -gt 30 ]; then
            echo "STALE PLAN (${age_days}d): $plan"
            STALE=$((STALE + 1))
        fi
    done < <(find "$ACTIVE_DIR" -name '*.md' 2>/dev/null)
fi

if [ "$STALE" -gt 0 ]; then
    echo "WARN: $STALE stale plan(s) in docs/exec-plans/active/"
else
    echo "OK: No stale active plans"
fi

# --- Summary ---
if [ "$ERRORS" -gt 0 ]; then
    echo ""
    echo "FAILED: $ERRORS doc governance error(s)"
    exit 1
fi

echo ""
echo "All doc governance checks passed."
