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

check_staged_diff_pattern() {
    local pattern="$1"
    local description="$2"
    local matches

    matches=$(git diff --cached --no-ext-diff --unified=0 -- . | grep -nE "$pattern" || true)

    if [[ -z "$matches" ]]; then
        return
    fi

    echo "[public-hygiene] $description" >&2
    echo "$matches" >&2
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
check_staged_diff_pattern '^(\+).*/Users/[^[:space:]"'"'"'<>]+' \
    "local absolute macOS path detected in staged diff"
check_staged_diff_pattern '^(\+).*/home/[^[:space:]"'"'"'<>]+' \
    "local absolute Linux path detected in staged diff"
check_staged_diff_pattern '^(\+).*([A-Za-z]:\\\\Users\\\\[^[:space:]"'"'"'<>]+)' \
    "local absolute Windows path detected in staged diff"
check_staged_diff_pattern '^(\+).*-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----' \
    "private key material detected in staged diff"
check_staged_diff_pattern '^(\+).*(ghp_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9_-]{20,})' \
    "high-risk credential literal detected in staged diff"

if [[ "$found_issue" -ne 0 ]]; then
    exit 1
fi

echo "[public-hygiene] ok"
