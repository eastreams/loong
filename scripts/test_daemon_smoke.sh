#!/usr/bin/env bash
set -euo pipefail

runner_args=()
if [[ "${LOONG_DAEMON_TEST_ALL_FEATURES:-0}" == "1" ]]; then
  runner_args+=(--all-features)
fi
runner_args+=(daemon_smoke)

exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/run_selected_daemon_tests.sh" "${runner_args[@]}"
