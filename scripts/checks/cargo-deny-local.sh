#!/usr/bin/env bash
set -euo pipefail

# Use a repo-local writable CARGO_HOME by default so cargo-deny can manage its
# advisory database lock and fetch state without depending on the caller's home
# directory permissions.

if [[ -z "${CARGO_HOME:-}" ]]; then
  export CARGO_HOME="$(pwd)/target/cargo-deny-home"
fi

mkdir -p "$CARGO_HOME"

exec cargo deny "$@"
