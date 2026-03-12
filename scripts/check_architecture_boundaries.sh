#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

STRICT="${LOONGCLAW_ARCH_STRICT:-false}"
if [[ "$STRICT" != "true" && "$STRICT" != "false" ]]; then
  echo "invalid LOONGCLAW_ARCH_STRICT: $STRICT (expected true|false)" >&2
  exit 2
fi

violations=0

have_rg() {
  command -v rg >/dev/null 2>&1
}

count_pattern_in_file() {
  local pattern="$1"
  local file="$2"
  if have_rg; then
    rg -n "$pattern" "$file" | wc -l | tr -d '[:space:]'
  else
    { grep -En "$pattern" "$file" || true; } | wc -l | tr -d '[:space:]'
  fi
}

find_memory_literal_hits() {
  local pattern="$1"
  if have_rg; then
    rg -n "$pattern" crates/app/src --glob '!crates/app/src/memory/**' || true
  else
    grep -REn "$pattern" crates/app/src --exclude-dir=memory || true
  fi
}

check_file_budget() {
  local key="$1"
  local file="$2"
  local max_lines="$3"
  local max_functions="$4"

  if [[ ! -f "$file" ]]; then
    echo "[arch] missing file: $file" >&2
    violations=$((violations + 1))
    return
  fi

  local lines
  lines="$(wc -l <"$file" | tr -d '[:space:]')"

  local functions
  functions="$(count_pattern_in_file '^(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+' "$file")"

  local line_status="ok"
  local fn_status="ok"

  if (( lines > max_lines )); then
    line_status="over"
    violations=$((violations + 1))
  fi
  if (( functions > max_functions )); then
    fn_status="over"
    violations=$((violations + 1))
  fi

  printf '[arch] %-16s lines=%4s/%-4s (%s) fns=%3s/%-3s (%s) file=%s\n' \
    "$key" "$lines" "$max_lines" "$line_status" "$functions" "$max_functions" "$fn_status" "$file"
}

check_file_budget "spec_runtime" "crates/spec/src/spec_runtime.rs" 3600 65
check_file_budget "spec_execution" "crates/spec/src/spec_execution.rs" 3700 80
check_file_budget "provider_mod" "crates/app/src/provider/mod.rs" 1000 20
check_file_budget "memory_mod" "crates/app/src/memory/mod.rs" 650 16

memory_literal_hits="$(find_memory_literal_hits '"append_turn"|"window"|"clear_session"')"
if [[ -n "$memory_literal_hits" ]]; then
  echo "[arch] over: memory operation literals found outside memory module boundary" >&2
  echo "$memory_literal_hits" >&2
  violations=$((violations + 1))
else
  echo "[arch] ok: memory operation literals are centralized in crates/app/src/memory/*"
fi

if (( violations > 0 )); then
  if [[ "$STRICT" == "true" ]]; then
    echo "[arch] failed: detected $violations architectural boundary violation(s)" >&2
    exit 1
  fi
  echo "[arch] warning: detected $violations architectural boundary violation(s) (strict mode disabled)" >&2
  exit 0
fi

echo "[arch] passed: architectural boundaries are within configured budgets"
