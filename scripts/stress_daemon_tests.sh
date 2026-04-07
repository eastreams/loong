#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ROUNDS="${1:-10}"
THREAD_MODES="${2:-default,2,1}"
LOG_DIR="${3:-target/test-stress/daemon}"
TRAP_MODES="${4:-${LOONGCLAW_STRESS_WASM_TRAPS_MODES:-auto}}"
LOCKED="${LOONGCLAW_STRESS_LOCKED:-true}"
CONTINUE_ON_FAILURE="${LOONGCLAW_STRESS_CONTINUE_ON_FAILURE:-false}"

if ! [[ "$ROUNDS" =~ ^[0-9]+$ ]] || [[ "$ROUNDS" -le 0 ]]; then
  echo "invalid ROUNDS: $ROUNDS (expected positive integer)" >&2
  exit 2
fi

if [[ "$LOCKED" != "true" && "$LOCKED" != "false" ]]; then
  echo "invalid LOONGCLAW_STRESS_LOCKED: $LOCKED (expected true|false)" >&2
  exit 2
fi

if [[ "$CONTINUE_ON_FAILURE" != "true" && "$CONTINUE_ON_FAILURE" != "false" ]]; then
  echo "invalid LOONGCLAW_STRESS_CONTINUE_ON_FAILURE: $CONTINUE_ON_FAILURE (expected true|false)" >&2
  exit 2
fi

mkdir -p "$LOG_DIR"

start_ts="$(date +%s)"
SUMMARY_FILE="$LOG_DIR/summary.txt"
total_modes=0
failed_modes=0
passed_modes=0
had_failure=0

: >"$SUMMARY_FILE"
echo "[stress] config rounds=${ROUNDS} modes=${THREAD_MODES} traps=${TRAP_MODES} locked=${LOCKED} continue_on_failure=${CONTINUE_ON_FAILURE}" | tee -a "$SUMMARY_FILE"

run_mode() {
  local trap_mode="$1"
  local mode="$2"
  local passed_runs=0
  local run_index
  for run_index in $(seq 1 "$ROUNDS"); do
    local log_file="$LOG_DIR/traps-${trap_mode}-mode-${mode}-run-${run_index}.log"
    local cmd=(
      cargo test
      -p loong
      --bin loong
      --all-features
    )
    if [[ "$LOCKED" == "true" ]]; then
      cmd+=(--locked)
    fi
    if [[ "$mode" != "default" ]]; then
      cmd+=(-- "--test-threads=${mode}")
    fi

    echo "[stress] traps=${trap_mode} mode=${mode} run=${run_index}/${ROUNDS}"
    local status="PASS"
    if [[ "$trap_mode" == "auto" ]]; then
      if ! env -u LOONGCLAW_WASM_SIGNALS_BASED_TRAPS "${cmd[@]}" >"$log_file" 2>&1; then
        status="FAIL"
      fi
    elif ! LOONGCLAW_WASM_SIGNALS_BASED_TRAPS="$trap_mode" "${cmd[@]}" >"$log_file" 2>&1; then
      status="FAIL"
    fi

    echo "[stress] run traps=${trap_mode} mode=${mode} run=${run_index}/${ROUNDS} status=${status} log=${log_file}" >>"$SUMMARY_FILE"
    if [[ "$status" == "PASS" ]]; then
      passed_runs=$((passed_runs + 1))
      continue
    fi

    echo "[stress] failed: traps=${trap_mode} mode=${mode} run=${run_index}" >&2
    echo "[stress] log: $log_file" >&2
    tail -n 80 "$log_file" >&2 || true
    echo "[stress] mode-result traps=${trap_mode} mode=${mode} status=FAIL passed_runs=${passed_runs} failed_run=${run_index} log=${log_file}" | tee -a "$SUMMARY_FILE"
    had_failure=1
    failed_modes=$((failed_modes + 1))
    if [[ "$CONTINUE_ON_FAILURE" != "true" ]]; then
      return 1
    fi
    return 0
  done

  passed_modes=$((passed_modes + 1))
  echo "[stress] mode-result traps=${trap_mode} mode=${mode} status=PASS passed_runs=${passed_runs}" | tee -a "$SUMMARY_FILE"
  return 0
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
    total_modes=$((total_modes + 1))
    if ! run_mode "$trap_mode" "$mode"; then
      if [[ "$CONTINUE_ON_FAILURE" != "true" ]]; then
        break 2
      fi
    fi
  done
done

end_ts="$(date +%s)"
if [[ "$had_failure" -eq 1 ]]; then
  overall_status="FAIL"
else
  overall_status="PASS"
fi
echo "[stress] overall status=${overall_status} total_modes=${total_modes} passed_modes=${passed_modes} failed_modes=${failed_modes} duration_s=$((end_ts - start_ts))" | tee -a "$SUMMARY_FILE"
echo "[stress] completed in $((end_ts - start_ts))s"
echo "[stress] modes=${THREAD_MODES} traps=${TRAP_MODES} rounds=${ROUNDS} (logs: ${LOG_DIR})"
echo "[stress] summary: ${SUMMARY_FILE}"
if [[ "$had_failure" -eq 1 ]]; then
  exit 1
fi
