#!/usr/bin/env bash
set -euo pipefail

BIND="${BIND:-127.0.0.1:4318}"
BUILD="${BUILD:-0}"
BUILD_DAEMON="${BUILD_DAEMON:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
WEB_ROOT="${REPO_ROOT}/web"
DIST_ROOT="${WEB_ROOT}/dist"
RUNTIME_ROOT="${HOME}/.loong"
LOG_ROOT="${RUNTIME_ROOT}/logs"
RUN_ROOT="${RUNTIME_ROOT}/run"

mkdir -p "${LOG_ROOT}" "${RUN_ROOT}"

UI_LOG="${LOG_ROOT}/web-same-origin.log"
UI_ERR="${LOG_ROOT}/web-same-origin.err.log"
UI_PID_FILE="${RUN_ROOT}/web-same-origin.pid"

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
  if [[ ! -f "${daemon_exe}" && -f "${REPO_ROOT}/target/debug/loong" ]]; then
    daemon_exe="${REPO_ROOT}/target/debug/loong"
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

PORT="${BIND##*:}"
stop_pid_file_process "${UI_PID_FILE}"
stop_port_processes "${PORT}"

DAEMON_EXE="$(resolve_daemon_exe)"

if [[ "${BUILD}" == "1" ]]; then
  (
    cd "${WEB_ROOT}"
    npm run build
  )
fi

DIST_INDEX="${DIST_ROOT}/index.html"
if [[ ! -f "${DIST_INDEX}" ]]; then
  echo "Missing built Web assets: ${DIST_INDEX}" >&2
  echo "Run: (cd web && npm run build)" >&2
  exit 1
fi

(
  cd "${REPO_ROOT}"
  nohup "${DAEMON_EXE}" web serve --bind "${BIND}" --static-root "${DIST_ROOT}" >"${UI_LOG}" 2>"${UI_ERR}" &
  echo $! >"${UI_PID_FILE}"
)

if ! wait_for_http "http://${BIND}/" 20; then
  echo "Same-origin Web server did not become ready. Check ${UI_ERR}" >&2
  exit 1
fi

echo "Web UI + API: http://${BIND}"
echo "Mode: same-origin-static"
echo "Logs: ${LOG_ROOT}"
echo "PID: $(cat "${UI_PID_FILE}")"
