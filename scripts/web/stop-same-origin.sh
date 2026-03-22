#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-4318}"
pids="$(lsof -ti "tcp:${PORT}" 2>/dev/null || true)"
if [[ -n "${pids}" ]]; then
  echo "${pids}" | xargs kill -9 >/dev/null 2>&1 || true
fi

echo "Stopped same-origin Web process on port ${PORT}."
