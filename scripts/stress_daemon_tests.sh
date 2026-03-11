#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ROUNDS="${1:-10}"
THREAD_MODES="${2:-default,2,1}"
LOG_DIR="${3:-target/test-stress/daemon}"
TRAP_MODES="${4:-${LOONGCLAW_STRESS_WASM_TRAPS_MODES:-auto}}"
LOCKED="${LOONGCLAW_STRESS_LOCKED:-true}"

if ! [[ "$ROUNDS" =~ ^[0-9]+$ ]] || [[ "$ROUNDS" -le 0 ]]; then
  echo "invalid ROUNDS: $ROUNDS (expected positive integer)" >&2
  exit 2
fi

if [[ "$LOCKED" != "true" && "$LOCKED" != "false" ]]; then
  echo "invalid LOONGCLAW_STRESS_LOCKED: $LOCKED (expected true|false)" >&2
  exit 2
fi

mkdir -p "$LOG_DIR"

start_ts="$(date +%s)"

run_mode() {
  local trap_mode="$1"
  local mode="$2"
  local run_index
  for run_index in $(seq 1 "$ROUNDS"); do
    local log_file="$LOG_DIR/traps-${trap_mode}-mode-${mode}-run-${run_index}.log"
    local cmd=(
      cargo test
      -p loongclaw-daemon
      --bin loongclaw
      --all-features
    )
    if [[ "$LOCKED" == "true" ]]; then
      cmd+=(--locked)
    fi
    if [[ "$mode" != "default" ]]; then
      cmd+=(-- "--test-threads=${mode}")
    fi

    echo "[stress] traps=${trap_mode} mode=${mode} run=${run_index}/${ROUNDS}"
    if [[ "$trap_mode" == "auto" ]]; then
      if ! env -u LOONGCLAW_WASM_SIGNALS_BASED_TRAPS "${cmd[@]}" >"$log_file" 2>&1; then
        echo "[stress] failed: traps=${trap_mode} mode=${mode} run=${run_index}" >&2
        echo "[stress] log: $log_file" >&2
        tail -n 80 "$log_file" >&2 || true
        exit 1
      fi
      continue
    fi

    if ! LOONGCLAW_WASM_SIGNALS_BASED_TRAPS="$trap_mode" "${cmd[@]}" >"$log_file" 2>&1; then
      echo "[stress] failed: traps=${trap_mode} mode=${mode} run=${run_index}" >&2
      echo "[stress] log: $log_file" >&2
      tail -n 80 "$log_file" >&2 || true
      exit 1
    fi
  done
}

IFS=',' read -r -a modes <<<"$THREAD_MODES"
IFS=',' read -r -a trap_modes <<<"$TRAP_MODES"
for trap_mode_raw in "${trap_modes[@]}"; do
  trap_mode="${trap_mode_raw//[[:space:]]/}"
  if [[ -z "$trap_mode" ]]; then
    continue
  fi
  if [[ "$trap_mode" != "auto" && "$trap_mode" != "true" && "$trap_mode" != "false" ]]; then
    echo "invalid trap mode: $trap_mode (expected auto|true|false)" >&2
    exit 2
  fi

  for mode_raw in "${modes[@]}"; do
    mode="${mode_raw//[[:space:]]/}"
    if [[ -z "$mode" ]]; then
      continue
    fi
    if [[ "$mode" != "default" && ! "$mode" =~ ^[0-9]+$ ]]; then
      echo "invalid thread mode: $mode (expected default or positive integer)" >&2
      exit 2
    fi
    run_mode "$trap_mode" "$mode"
  done
done

end_ts="$(date +%s)"
echo "[stress] completed in $((end_ts - start_ts))s"
echo "[stress] modes=${THREAD_MODES} traps=${TRAP_MODES} rounds=${ROUNDS} (logs: ${LOG_DIR})"
