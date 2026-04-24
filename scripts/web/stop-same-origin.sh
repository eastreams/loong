#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-4318}"
RUN_ROOT="${HOME}/.loong/run"
UI_PID_FILE="${RUN_ROOT}/web-same-origin.pid"

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

kill_pid_file "${UI_PID_FILE}"

pids="$(lsof -ti "tcp:${PORT}" 2>/dev/null || true)"
if [[ -n "${pids}" ]]; then
  echo "${pids}" | xargs kill -9 >/dev/null 2>&1 || true
fi

echo "Stopped same-origin Web process on port ${PORT}."
