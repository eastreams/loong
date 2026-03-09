#!/usr/bin/env bash
set -euo pipefail

# Doc governance checks — validates mirror consistency and dead links.
# Referenced by: task check:docs (Taskfile.yml)

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ERRORS=0
PUBLIC_GITHUB_REPO="${LOONGCLAW_PUBLIC_REPO:-loongclaw-ai/loongclaw}"
PUBLIC_GITHUB_BASE="https://github.com/${PUBLIC_GITHUB_REPO}"

# --- 1. CLAUDE.md / AGENTS.md mirror check ---
if ! diff -q "$REPO_ROOT/CLAUDE.md" "$REPO_ROOT/AGENTS.md" > /dev/null 2>&1; then
    echo "FAIL: CLAUDE.md and AGENTS.md are not mirrored"
    ERRORS=$((ERRORS + 1))
else
    echo "OK: CLAUDE.md == AGENTS.md"
fi

# --- 2. Dead internal links in docs/ ---
DEAD_LINK_FILE=$(mktemp)
RELEASE_TAGS_FILE=$(mktemp)
trap 'rm -f "$DEAD_LINK_FILE" "$RELEASE_TAGS_FILE"' EXIT

find "$REPO_ROOT/docs" "$REPO_ROOT/CLAUDE.md" "$REPO_ROOT/AGENTS.md" -name '*.md' 2>/dev/null | while IFS= read -r md_file; do
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

# --- 3. Release docs map to released versions ---
RELEASE_VERSIONS="$(grep -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' "$REPO_ROOT/CHANGELOG.md" | sed -E 's/^## \[([0-9]+\.[0-9]+\.[0-9]+)\]$/\1/' || true)"
if [ -z "$RELEASE_VERSIONS" ]; then
    echo "OK: No released versions found in CHANGELOG.md"
else
    while IFS= read -r version; do
        [ -z "$version" ] && continue
        tag="v${version}"
        doc_path="$REPO_ROOT/docs/releases/${tag}.md"

        if [ ! -f "$doc_path" ]; then
            echo "FAIL: missing release doc for ${tag}: docs/releases/${tag}.md"
            ERRORS=$((ERRORS + 1))
            continue
        fi
        if ! grep -Fxq "# Release ${tag}" "$doc_path"; then
            echo "FAIL: ${doc_path} missing heading '# Release ${tag}'"
            ERRORS=$((ERRORS + 1))
        fi
        required_sections=(
            "## Summary"
            "## Process"
            "## Artifacts"
            "## Verification"
            "## Known Issues"
            "## Rollback"
            "## Detail Links"
        )
        for section in "${required_sections[@]}"; do
            if ! grep -Fxq "$section" "$doc_path"; then
                echo "FAIL: ${doc_path} missing section '$section'"
                ERRORS=$((ERRORS + 1))
            fi
        done

        if ! grep -Fq "| Asset |" "$doc_path"; then
            echo "FAIL: ${doc_path} missing artifacts table header '| Asset |'"
            ERRORS=$((ERRORS + 1))
        fi

        if ! grep -Fq "Trace ID:" "$doc_path"; then
            echo "FAIL: ${doc_path} missing trace summary field 'Trace ID:'"
            ERRORS=$((ERRORS + 1))
        fi
        if ! grep -Fq "Trace path:" "$doc_path"; then
            echo "FAIL: ${doc_path} missing trace summary field 'Trace path:'"
            ERRORS=$((ERRORS + 1))
        fi

        DETAIL_LINKS_CONTENT="$(awk '/^## Detail Links$/{flag=1; next} /^## /{flag=0} flag {print}' "$doc_path")"
        DETAIL_LINK_COUNT="$(printf '%s\n' "$DETAIL_LINKS_CONTENT" | grep -Eo '\[[^]]+\]\([^)]+\)' | wc -l | tr -d ' ')"
        if [ "$DETAIL_LINK_COUNT" -lt 3 ]; then
            echo "FAIL: ${doc_path} needs at least three markdown links under '## Detail Links'"
            ERRORS=$((ERRORS + 1))
        fi

        debug_doc_path="$REPO_ROOT/.docs/releases/${tag}-debug.md"
        if [ ! -f "$debug_doc_path" ]; then
            echo "FAIL: missing local debug doc for ${tag}: .docs/releases/${tag}-debug.md"
            ERRORS=$((ERRORS + 1))
        else
            if ! grep -Fq "Trace path:" "$debug_doc_path"; then
                echo "FAIL: ${debug_doc_path} missing trace field 'Trace path:'"
                ERRORS=$((ERRORS + 1))
            fi
        fi

        echo "$tag" >> "$RELEASE_TAGS_FILE"
    done <<< "$RELEASE_VERSIONS"
fi

# --- 4. Trace index linkage checks ---
if [ -s "$RELEASE_TAGS_FILE" ]; then
    TRACE_INDEX="$REPO_ROOT/.docs/traces/index.jsonl"
    TRACE_LATEST="$REPO_ROOT/.docs/traces/latest"
    if [ ! -f "$TRACE_INDEX" ]; then
        echo "FAIL: missing trace index for released versions: .docs/traces/index.jsonl"
        ERRORS=$((ERRORS + 1))
    fi
    if [ ! -f "$TRACE_LATEST" ]; then
        echo "FAIL: missing latest trace pointer: .docs/traces/latest"
        ERRORS=$((ERRORS + 1))
    fi

    while IFS= read -r tag; do
        [ -z "$tag" ] && continue
        by_tag_latest="$REPO_ROOT/.docs/traces/by-tag/${tag}/latest"
        if [ ! -f "$by_tag_latest" ]; then
            echo "FAIL: missing by-tag latest pointer for ${tag}: .docs/traces/by-tag/${tag}/latest"
            ERRORS=$((ERRORS + 1))
        fi

        if [ -f "$TRACE_INDEX" ]; then
            if ! grep -F "\"tag\":\"${tag}\"" "$TRACE_INDEX" | grep -F "\"command\":\"post-release\"" | grep -F "\"status\":\"success\"" > /dev/null; then
                echo "FAIL: trace index missing successful post-release record for ${tag}"
                ERRORS=$((ERRORS + 1))
            fi
        fi
    done < "$RELEASE_TAGS_FILE"
fi

# --- 5. Canonical public repository link checks ---
check_canonical_github_links() {
    local file="$1"
    local found=0
    while IFS= read -r url; do
        found=1
        case "$url" in
            "${PUBLIC_GITHUB_BASE}"/*) ;;
            *)
                echo "FAIL: ${file} contains non-canonical GitHub URL: ${url}"
                ERRORS=$((ERRORS + 1))
                ;;
        esac
    done < <(grep -Eo 'https://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+[^)[:space:]]*' "$file" 2>/dev/null || true)

    if [ "$found" -eq 0 ]; then
        echo "FAIL: ${file} contains no GitHub links; expected links under ${PUBLIC_GITHUB_BASE}"
        ERRORS=$((ERRORS + 1))
    fi
}

for release_doc in "$REPO_ROOT/docs/releases/TEMPLATE.md" "$REPO_ROOT"/docs/releases/v*.md; do
    [ -f "$release_doc" ] || continue
    if grep -Fq "github.com/<org>/<repo>" "$release_doc"; then
        echo "FAIL: ${release_doc} still contains placeholder github.com/<org>/<repo>"
        ERRORS=$((ERRORS + 1))
    fi
    check_canonical_github_links "$release_doc"
done

ISSUE_TEMPLATE_CONFIG="$REPO_ROOT/.github/ISSUE_TEMPLATE/config.yml"
EXPECTED_ADVISORY_URL="${PUBLIC_GITHUB_BASE}/security/advisories/new"
if [ ! -f "$ISSUE_TEMPLATE_CONFIG" ]; then
    echo "FAIL: missing issue template config: .github/ISSUE_TEMPLATE/config.yml"
    ERRORS=$((ERRORS + 1))
elif ! grep -Fq "${EXPECTED_ADVISORY_URL}" "$ISSUE_TEMPLATE_CONFIG"; then
    echo "FAIL: ${ISSUE_TEMPLATE_CONFIG} must reference canonical advisory URL ${EXPECTED_ADVISORY_URL}"
    ERRORS=$((ERRORS + 1))
fi

# --- Summary ---
if [ "$ERRORS" -gt 0 ]; then
    echo ""
    echo "FAILED: $ERRORS doc governance error(s)"
    exit 1
fi

echo ""
echo "All doc governance checks passed."
