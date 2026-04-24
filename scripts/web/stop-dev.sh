#!/usr/bin/env bash
set -euo pipefail

API_PORT="${API_PORT:-4317}"
DEV_PORT="${DEV_PORT:-4173}"
RUN_ROOT="${HOME}/.loong/run"
API_PID_FILE="${RUN_ROOT}/web-api.pid"
DEV_PID_FILE="${RUN_ROOT}/web-dev.pid"

kill_pid_file() {
  local pid_file="$1"
  if [[ -f "${pid_file}" ]]; then
    local pid
    pid="$(cat "${pid_file}" 2>/dev/null || true)"
    if [[ -n "${pid}" ]]; then
      kill -9 "${pid}" >/dev/null 2>&1 || true
    fi
    rm -f "${pid_file}"
  fi
}

kill_port() {
  local port="$1"
  local pids
  pids="$(lsof -ti "tcp:${port}" 2>/dev/null || true)"
  if [[ -n "${pids}" ]]; then
    echo "${pids}" | xargs kill -9 >/dev/null 2>&1 || true
  fi
}

kill_pid_file "${API_PID_FILE}"
kill_pid_file "${DEV_PID_FILE}"
kill_port "${API_PORT}"
kill_port "${DEV_PORT}"

echo "Stopped web dev processes on ports ${API_PORT} and ${DEV_PORT}."
