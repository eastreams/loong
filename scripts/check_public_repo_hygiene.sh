#!/usr/bin/env bash

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

found_issue=0

check_forbidden_path() {
    local path_prefix="$1"
    local tracked_files

    tracked_files=$(git ls-files -- "$path_prefix")

    if [[ -z "$tracked_files" ]]; then
        return
    fi

    echo "[public-hygiene] forbidden public path detected under \`$path_prefix\`" >&2
    echo "$tracked_files" >&2
    found_issue=1
}

check_secret_pattern() {
    local pattern="$1"
    local description="$2"
    local matches

    matches=$(git grep -nI -E "$pattern" -- . || true)

    if [[ -z "$matches" ]]; then
        return
    fi

    echo "[public-hygiene] $description" >&2
    echo "$matches" >&2
    found_issue=1
}

check_forbidden_path "harbor/jobs"

check_secret_pattern '"OPENAI_API_KEY"[[:space:]]*:[[:space:]]*"sk-[A-Za-z0-9_-]{20,}' \
    "inline OpenAI key material detected in tracked JSON-like content"
check_secret_pattern 'OPENAI_API_KEY=sk-[A-Za-z0-9_-]{20,}' \
    "inline OpenAI key assignment detected in tracked content"

if [[ "$found_issue" -ne 0 ]]; then
    exit 1
fi

echo "[public-hygiene] ok"
