#!/usr/bin/env bash
set -euo pipefail

API_BIND="${API_BIND:-127.0.0.1:4317}"
DEV_HOST="${DEV_HOST:-127.0.0.1}"
DEV_PORT="${DEV_PORT:-4173}"
BUILD_DAEMON="${BUILD_DAEMON:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
WEB_ROOT="${REPO_ROOT}/web"
RUNTIME_ROOT="${HOME}/.loong"
LOG_ROOT="${RUNTIME_ROOT}/logs"
RUN_ROOT="${RUNTIME_ROOT}/run"

mkdir -p "${LOG_ROOT}" "${RUN_ROOT}"

API_LOG="${LOG_ROOT}/web-api.log"
API_ERR="${LOG_ROOT}/web-api.err.log"
DEV_LOG="${LOG_ROOT}/web-dev.log"
DEV_ERR="${LOG_ROOT}/web-dev.err.log"
API_PID_FILE="${RUN_ROOT}/web-api.pid"
DEV_PID_FILE="${RUN_ROOT}/web-dev.pid"

stop_pid_file_process() {
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

stop_port_processes() {
  local port="$1"
  local pids
  pids="$(lsof -ti "tcp:${port}" 2>/dev/null || true)"
  if [[ -n "${pids}" ]]; then
    echo "${pids}" | xargs kill -9 >/dev/null 2>&1 || true
    sleep 0.5
  fi
}

wait_for_http() {
  local url="$1"
  local max_attempts="$2"
  local ready=1

  for ((i = 0; i < max_attempts; i++)); do
    sleep 0.5
    if curl --silent --show-error --fail --max-time 3 "${url}" >/dev/null 2>&1; then
      ready=0
      break
    fi
  done

  return "${ready}"
}

resolve_daemon_exe() {
  local daemon_exe="${REPO_ROOT}/target/debug/loong"
  if [[ ! -f "${daemon_exe}" && -f "${REPO_ROOT}/target/debug/loongclaw" ]]; then
    daemon_exe="${REPO_ROOT}/target/debug/loongclaw"
  fi

  if [[ "${BUILD_DAEMON}" == "1" || ! -f "${daemon_exe}" ]]; then
    (
      cd "${REPO_ROOT}"
      cargo build --bin loong
    )
    daemon_exe="${REPO_ROOT}/target/debug/loong"
  fi

  if [[ ! -f "${daemon_exe}" ]]; then
    echo "Missing daemon binary: ${daemon_exe}" >&2
    echo "Run with BUILD_DAEMON=1 or build loong manually." >&2
    return 1
  fi

  echo "${daemon_exe}"
}

API_PORT="${API_BIND##*:}"
stop_pid_file_process "${API_PID_FILE}"
stop_pid_file_process "${DEV_PID_FILE}"
stop_port_processes "${API_PORT}"
stop_port_processes "${DEV_PORT}"

if [[ "${BUILD_DAEMON}" == "1" ]]; then
  DAEMON_BUILD_MODE="forced"
elif [[ -f "${REPO_ROOT}/target/debug/loong" || -f "${REPO_ROOT}/target/debug/loongclaw" ]]; then
  DAEMON_BUILD_MODE="reused existing binary"
else
  DAEMON_BUILD_MODE="built missing binary"
fi

DAEMON_EXE="$(resolve_daemon_exe)"

VITE_CMD="${WEB_ROOT}/node_modules/.bin/vite"
if [[ ! -f "${VITE_CMD}" ]]; then
  echo "Missing Vite binary: ${VITE_CMD}" >&2
  echo "Run: (cd web && npm install)" >&2
  exit 1
fi

(
  cd "${REPO_ROOT}"
  nohup "${DAEMON_EXE}" web serve --bind "${API_BIND}" >"${API_LOG}" 2>"${API_ERR}" &
  echo $! >"${API_PID_FILE}"
)

(
  cd "${WEB_ROOT}"
  nohup "${VITE_CMD}" --host "${DEV_HOST}" --port "${DEV_PORT}" >"${DEV_LOG}" 2>"${DEV_ERR}" &
  echo $! >"${DEV_PID_FILE}"
)

if ! wait_for_http "http://${API_BIND}/healthz" 20; then
  echo "Web API did not become ready. Check ${API_ERR}" >&2
  exit 1
fi

if ! wait_for_http "http://${DEV_HOST}:${DEV_PORT}/" 20; then
  echo "Web dev server did not become ready. Check ${DEV_ERR}" >&2
  exit 1
fi

echo "Web API: http://${API_BIND}"
echo "Web Dev: http://${DEV_HOST}:${DEV_PORT}"
echo "Logs: ${LOG_ROOT}"
echo "API PID: $(cat "${API_PID_FILE}")"
echo "Dev PID: $(cat "${DEV_PID_FILE}")"
if [[ "${BUILD_DAEMON}" == "1" ]]; then
  :
fi
echo "Daemon build: ${DAEMON_BUILD_MODE}"
