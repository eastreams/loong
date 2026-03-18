#!/usr/bin/env bash
set -euo pipefail

# Doc governance checks — validates mirror consistency and dead links.
# Referenced by: task check:docs (Taskfile.yml)

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
. "$REPO_ROOT/scripts/release_artifact_lib.sh"
ERRORS=0
WARNINGS=0
PUBLIC_GITHUB_REPO="${LOONGCLAW_PUBLIC_REPO:-loongclaw-ai/loongclaw}"
PUBLIC_GITHUB_BASE="https://github.com/${PUBLIC_GITHUB_REPO}"

if [ -n "${LOONGCLAW_RELEASE_DOCS_STRICT:-}" ]; then
    case "${LOONGCLAW_RELEASE_DOCS_STRICT}" in
        1|true|TRUE|yes|YES) STRICT_RELEASE_DOCS=1 ;;
        0|false|FALSE|no|NO) STRICT_RELEASE_DOCS=0 ;;
        *)
            echo "FAIL: invalid LOONGCLAW_RELEASE_DOCS_STRICT value '${LOONGCLAW_RELEASE_DOCS_STRICT}' (expected 0/1)"
            exit 1
            ;;
    esac
elif [ "${CI:-}" = "true" ] || [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    STRICT_RELEASE_DOCS=1
else
    STRICT_RELEASE_DOCS=0
fi

artifact_gate_fail_or_warn() {
    local message="$1"
    if [ "$STRICT_RELEASE_DOCS" -eq 1 ]; then
        echo "FAIL: ${message}"
        ERRORS=$((ERRORS + 1))
    else
        echo "WARN: ${message}"
        WARNINGS=$((WARNINGS + 1))
    fi
}

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
RELEASE_TRACE_EXPECTATIONS_FILE=$(mktemp)
LATEST_RELEASE_VERSION=""
LATEST_RELEASE_TRACE_PATH=""
trap 'rm -f "$DEAD_LINK_FILE" "$RELEASE_TAGS_FILE" "$RELEASE_TRACE_EXPECTATIONS_FILE"' EXIT

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
RELEASE_VERSIONS="$(release_versions_from_changelog "$REPO_ROOT/CHANGELOG.md" || true)"
if [ -z "$RELEASE_VERSIONS" ]; then
    echo "OK: No released versions found in CHANGELOG.md"
else
    while IFS= read -r version; do
        [ -z "$version" ] && continue
        tag="$(release_tag_from_version "$version")"
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
            "## Highlights"
            "## Process"
            "## Artifacts"
            "## Verification"
            "## Refactor Budget"
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
        trace_id_field="$(release_doc_backticked_field "$doc_path" "Trace ID")"
        if [ -z "$trace_id_field" ]; then
            echo "FAIL: ${doc_path} missing exact backticked 'Trace ID' value"
            ERRORS=$((ERRORS + 1))
        fi
        if ! grep -Fq "Trace path:" "$doc_path"; then
            echo "FAIL: ${doc_path} missing trace summary field 'Trace path:'"
            ERRORS=$((ERRORS + 1))
        fi
        trace_path_field="$(release_doc_backticked_field "$doc_path" "Trace path")"
        if [ -z "$trace_path_field" ]; then
            echo "FAIL: ${doc_path} missing exact backticked 'Trace path' value"
            ERRORS=$((ERRORS + 1))
        fi
        if [ -n "$trace_path_field" ] && [ -n "$trace_id_field" ]; then
            if ! release_trace_path_matches_contract "$tag" "$trace_id_field" "$trace_path_field"; then
                trace_path_basename="$(basename "$trace_path_field" 2>/dev/null || true)"
                expected_trace_suffix="$(release_trace_expected_suffix "$tag" "$trace_id_field")"
                if [[ "$trace_path_field" != .docs/traces/* ]]; then
                    echo "FAIL: ${doc_path} Trace path must stay under .docs/traces/: ${trace_path_field}"
                elif ! release_trace_path_segments_safe "$trace_path_field"; then
                    echo "FAIL: ${doc_path} Trace path must stay under .docs/traces/ without '.' or '..' segments or symlink prefixes: ${trace_path_field}"
                elif ! release_trace_path_symlink_prefixes_safe "$trace_path_field"; then
                    echo "FAIL: ${doc_path} Trace path must stay under .docs/traces/ without '.' or '..' segments or symlink prefixes: ${trace_path_field}"
                elif [[ "$trace_path_basename" != *"-post-release-"* ]]; then
                    echo "FAIL: ${doc_path} Trace path basename must include -post-release-"
                else
                    echo "FAIL: ${doc_path} trace identity must align with Trace path suffix ${expected_trace_suffix}"
                fi
                ERRORS=$((ERRORS + 1))
            fi
        fi
        if ! grep -Fq "Refactor budget item:" "$doc_path"; then
            echo "FAIL: ${doc_path} missing explicit process field 'Refactor budget item:'"
            ERRORS=$((ERRORS + 1))
        fi

        DETAIL_LINKS_CONTENT="$(awk '/^## Detail Links$/{flag=1; next} /^## /{flag=0} flag {print}' "$doc_path")"
        DETAIL_LINK_COUNT="$(printf '%s\n' "$DETAIL_LINKS_CONTENT" | grep -Eo '\[[^]]+\]\([^)]+\)' | wc -l | tr -d ' ')"
        if [ "$DETAIL_LINK_COUNT" -lt 3 ]; then
            echo "FAIL: ${doc_path} needs at least three markdown links under '## Detail Links'"
            ERRORS=$((ERRORS + 1))
        fi
        trace_detail_path="$(sed -n -E 's/^- Trace directory: `([^`]+)`$/\1/p' "$doc_path" | head -n 1)"
        if [ -z "$trace_detail_path" ]; then
            echo "FAIL: ${doc_path} missing detail link field 'Trace directory:'"
            ERRORS=$((ERRORS + 1))
        elif [ -n "$trace_path_field" ] && [ "$trace_detail_path" != "$trace_path_field" ]; then
            echo "FAIL: ${doc_path} trace detail link must match Trace path (${trace_path_field})"
            ERRORS=$((ERRORS + 1))
        fi
        expected_debug_doc_rel="$(release_debug_doc_relpath "$tag")"
        debug_detail_path="$(sed -n -E 's/^- Local debug log: `([^`]+)`$/\1/p' "$doc_path" | head -n 1)"
        if [ -z "$debug_detail_path" ]; then
            echo "FAIL: ${doc_path} missing detail link field 'Local debug log:'"
            ERRORS=$((ERRORS + 1))
        elif [ "$debug_detail_path" != "$expected_debug_doc_rel" ]; then
            echo "FAIL: ${doc_path} local debug log detail link must be ${expected_debug_doc_rel}"
            ERRORS=$((ERRORS + 1))
        fi

        debug_doc_path="$REPO_ROOT/.docs/releases/${tag}-debug.md"
        if [ ! -f "$debug_doc_path" ]; then
            artifact_gate_fail_or_warn "missing local debug doc for ${tag}: .docs/releases/${tag}-debug.md"
        else
            if ! grep -Fq "Trace ID: \`${trace_id_field}\`" "$debug_doc_path"; then
                echo "FAIL: ${debug_doc_path} debug doc trace id must match release doc (${trace_id_field})"
                ERRORS=$((ERRORS + 1))
            fi
            if ! grep -Fq "Trace path: \`${trace_path_field}\`" "$debug_doc_path"; then
                echo "FAIL: ${debug_doc_path} debug doc trace path must match release doc (${trace_path_field})"
                ERRORS=$((ERRORS + 1))
            fi
        fi

        printf '%s|%s|%s|%s\n' "$tag" "$trace_id_field" "$trace_path_field" "$doc_path" >> "$RELEASE_TRACE_EXPECTATIONS_FILE"
        if [ -z "$LATEST_RELEASE_VERSION" ] || version_is_greater "$version" "$LATEST_RELEASE_VERSION"; then
            LATEST_RELEASE_VERSION="$version"
            LATEST_RELEASE_TRACE_PATH="$trace_path_field"
        fi
        echo "$tag" >> "$RELEASE_TAGS_FILE"
    done <<< "$RELEASE_VERSIONS"
fi

# --- 4. Trace index linkage checks ---
if [ -s "$RELEASE_TAGS_FILE" ]; then
    TRACE_INDEX="$REPO_ROOT/.docs/traces/index.jsonl"
    TRACE_LATEST="$REPO_ROOT/.docs/traces/latest"
    if [ ! -f "$TRACE_INDEX" ]; then
        artifact_gate_fail_or_warn "missing trace index for released versions: .docs/traces/index.jsonl"
    fi
    if [ ! -f "$TRACE_LATEST" ]; then
        artifact_gate_fail_or_warn "missing latest trace pointer: .docs/traces/latest"
    elif [ -n "$LATEST_RELEASE_TRACE_PATH" ]; then
        trace_latest_value="$(cat "$TRACE_LATEST")"
        if [ "$trace_latest_value" != "$LATEST_RELEASE_TRACE_PATH" ]; then
            echo "FAIL: .docs/traces/latest trace latest pointer must match highest released Trace path (${LATEST_RELEASE_TRACE_PATH})"
            ERRORS=$((ERRORS + 1))
        fi
    fi

    while IFS='|' read -r tag trace_id trace_path source_release_doc; do
        [ -z "$tag" ] && continue
        by_tag_latest="$REPO_ROOT/.docs/traces/by-tag/${tag}/latest"
        if [ ! -f "$by_tag_latest" ]; then
            artifact_gate_fail_or_warn "missing by-tag latest pointer for ${tag}: .docs/traces/by-tag/${tag}/latest"
        else
            by_tag_latest_value="$(cat "$by_tag_latest")"
            if [ "$by_tag_latest_value" != "$trace_path" ]; then
                echo "FAIL: ${by_tag_latest} by-tag latest pointer must match Trace path (${trace_path})"
                ERRORS=$((ERRORS + 1))
            fi
        fi

        if [ -f "$TRACE_INDEX" ]; then
            if ! grep -F "\"tag\":\"${tag}\"" "$TRACE_INDEX" | \
                grep -F "\"trace_id\":\"${trace_id}\"" | \
                grep -F "\"trace_path\":\"${trace_path}\"" | \
                grep -F "\"command\":\"post-release\"" | \
                grep -F "\"status\":\"success\"" | \
                grep -F "\"source_release_doc\":\"${source_release_doc#"$REPO_ROOT/"}\"" > /dev/null; then
                echo "FAIL: ${TRACE_INDEX} trace index record must exactly match release doc for ${tag}"
                ERRORS=$((ERRORS + 1))
            fi
        fi

        trace_metadata_path="$REPO_ROOT/${trace_path}/metadata.json"
        if [ ! -f "$trace_metadata_path" ]; then
            artifact_gate_fail_or_warn "missing trace metadata for ${tag}: ${trace_path}/metadata.json"
        else
            if ! grep -F "\"tag\":\"${tag}\"" "$trace_metadata_path" | \
                grep -F "\"trace_id\":\"${trace_id}\"" | \
                grep -F "\"trace_path\":\"${trace_path}\"" | \
                grep -F "\"command\":\"post-release\"" | \
                grep -F "\"status\":\"success\"" | \
                grep -F "\"source_release_doc\":\"${source_release_doc#"$REPO_ROOT/"}\"" > /dev/null; then
                echo "FAIL: ${trace_metadata_path} trace metadata must exactly match release doc for ${tag}"
                ERRORS=$((ERRORS + 1))
            fi
        fi
    done < "$RELEASE_TRACE_EXPECTATIONS_FILE"

    while IFS= read -r tag; do
        [ -z "$tag" ] && continue
        if [ -f "$TRACE_INDEX" ]; then
            if ! grep -F "\"tag\":\"${tag}\"" "$TRACE_INDEX" | grep -F "\"command\":\"post-release\"" | grep -F "\"status\":\"success\"" > /dev/null; then
                artifact_gate_fail_or_warn "trace index missing successful post-release record for ${tag}"
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

if [ "$WARNINGS" -gt 0 ]; then
    echo ""
    echo "PASSED with warnings: $WARNINGS non-blocking release-artifact warning(s)"
fi

echo ""
echo "All doc governance checks passed."
