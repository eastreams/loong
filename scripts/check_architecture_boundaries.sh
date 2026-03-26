#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
. "$REPO_ROOT/scripts/architecture_budget_lib.sh"

STRICT="${LOONGCLAW_ARCH_STRICT:-false}"
if [[ "$STRICT" != "true" && "$STRICT" != "false" ]]; then
  echo "invalid LOONGCLAW_ARCH_STRICT: $STRICT (expected true|false)" >&2
  exit 2
fi

violations=0
tight_hotspots=0
watch_hotspots=0
healthy_hotspots=0
hotspot_rows="$(architecture_hotspot_rows)" || exit 1

while IFS='|' read -r key file classes lines max_lines line_status functions max_functions fn_status peak_usage pressure; do
  if [[ "$line_status" == "over" ]]; then
    violations=$((violations + 1))
  fi
  if [[ "$fn_status" == "over" ]]; then
    violations=$((violations + 1))
  fi
  case "$pressure" in
    TIGHT)
      tight_hotspots=$((tight_hotspots + 1))
      ;;
    WATCH)
      watch_hotspots=$((watch_hotspots + 1))
      ;;
    HEALTHY)
      healthy_hotspots=$((healthy_hotspots + 1))
      ;;
  esac
  printf '[arch] %-16s class=%-36s pressure=%-7s peak=%6s lines=%4s/%-5s (%s) fns=%3s/%-3s (%s) file=%s\n' \
    "$key" "$classes" "$pressure" "$peak_usage" "$lines" "$max_lines" "$line_status" "$functions" "$max_functions" \
    "$fn_status" "$file"
done <<EOF_ROWS
${hotspot_rows}
EOF_ROWS

while IFS= read -r boundary_key; do
  [[ -z "$boundary_key" ]] && continue
  boundary_status="$(architecture_boundary_status "$boundary_key")"
  if [[ "$boundary_status" == "FAIL" ]]; then
    echo "[arch] over: $(architecture_boundary_fail_summary "$boundary_key")" >&2
    architecture_boundary_hits "$boundary_key" >&2
    violations=$((violations + 1))
  else
    echo "[arch] ok: $(architecture_boundary_pass_summary "$boundary_key")"
  fi
done <<EOF_BOUNDARIES
$(architecture_boundary_check_keys)
EOF_BOUNDARIES

echo "[arch] pressure summary: tight=${tight_hotspots} watch=${watch_hotspots} healthy=${healthy_hotspots}"

if (( violations > 0 )); then
  if [[ "$STRICT" == "true" ]]; then
    echo "[arch] failed: detected $violations architectural boundary violation(s)" >&2
    exit 1
  fi
  echo "[arch] warning: detected $violations architectural boundary violation(s) (strict mode disabled)" >&2
  exit 0
fi

echo "[arch] passed: architectural boundaries are within configured budgets"
