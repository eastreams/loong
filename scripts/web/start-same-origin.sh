#!/usr/bin/env bash
set -euo pipefail

BIND="${BIND:-127.0.0.1:4318}"
BUILD="${BUILD:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
WEB_ROOT="${REPO_ROOT}/web"
DIST_ROOT="${WEB_ROOT}/dist"
RUNTIME_ROOT="${HOME}/.loong"
LOG_ROOT="${RUNTIME_ROOT}/logs"

mkdir -p "${LOG_ROOT}"

UI_LOG="${LOG_ROOT}/web-same-origin.log"
UI_ERR="${LOG_ROOT}/web-same-origin.err.log"

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

PORT="${BIND##*:}"
stop_port_processes "${PORT}"

resolve_daemon_exe() {
  (
    cd "${REPO_ROOT}"
    cargo build --bin loong
  )

  if [[ -f "${REPO_ROOT}/target/debug/loong" ]]; then
    echo "${REPO_ROOT}/target/debug/loong"
    return 0
  fi

  if [[ -f "${REPO_ROOT}/target/debug/loongclaw" ]]; then
    echo "${REPO_ROOT}/target/debug/loongclaw"
    return 0
  fi

  echo "Missing daemon binary after build: ${REPO_ROOT}/target/debug/loong" >&2
  return 1
}

DAEMON_EXE="$(resolve_daemon_exe)"

DIST_INDEX="${DIST_ROOT}/index.html"
if [[ "${BUILD}" == "1" ]]; then
  (
    cd "${WEB_ROOT}"
    npm run build >/dev/null
  )
fi

if [[ ! -f "${DIST_INDEX}" ]]; then
  echo "Missing built Web assets: ${DIST_INDEX}" >&2
  echo "Run: (cd web && npm run build)" >&2
  exit 1
fi

(
  cd "${REPO_ROOT}"
  nohup "${DAEMON_EXE}" web serve --bind "${BIND}" --static-root "${DIST_ROOT}" >"${UI_LOG}" 2>"${UI_ERR}" &
  UI_PID=$!
)

if ! wait_for_http "http://${BIND}/" 20; then
  echo "Same-origin Web server did not become ready. Check ${UI_ERR}" >&2
  exit 1
fi

echo "Web UI + API: http://${BIND}"
echo "Mode: same-origin-static"
echo "Logs: ${LOG_ROOT}"
