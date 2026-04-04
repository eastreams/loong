#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

PACKAGES=(
  loongclaw-contracts
  loongclaw-protocol
  loongclaw-kernel
  loongclaw-spec
  loongclaw-bench
  loongclaw-app
  loongclaw
)

mode="dry-run"
from_package=""

usage() {
  cat <<'EOF'
Usage: scripts/publish_crates_io.sh [--publish] [--from <package>] [--help]

Default behavior is a dry run:
  scripts/publish_crates_io.sh

Real publish mode:
  scripts/publish_crates_io.sh --publish

Resume from a package in the publish chain:
  scripts/publish_crates_io.sh --from loongclaw-spec
EOF
}

package_exists() {
  local candidate="$1"
  local pkg
  for pkg in "${PACKAGES[@]}"; do
    if [[ "$pkg" == "$candidate" ]]; then
      return 0
    fi
  done
  return 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --publish)
      mode="publish"
      ;;
    --from)
      shift
      if [[ "$#" -eq 0 ]]; then
        echo "error: --from requires a package name" >&2
        usage >&2
        exit 2
      fi
      from_package="$1"
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [[ -n "$from_package" ]] && ! package_exists "$from_package"; then
  echo "error: unknown package in publish order: $from_package" >&2
  exit 2
fi

echo "mode: $mode"
if [[ -n "$from_package" ]]; then
  echo "starting from: $from_package"
else
  echo "starting from: ${PACKAGES[0]}"
fi

start_emitting="false"
if [[ -z "$from_package" ]]; then
  start_emitting="true"
fi

for pkg in "${PACKAGES[@]}"; do
  if [[ "$pkg" == "$from_package" ]]; then
    start_emitting="true"
  fi
  if [[ "$start_emitting" != "true" ]]; then
    continue
  fi

  cmd=(cargo publish -p "$pkg" --locked)
  if [[ "$mode" == "dry-run" ]]; then
    cmd=(cargo publish --dry-run -p "$pkg" --locked)
  fi

  printf '==> %s\n' "${cmd[*]}"
  "${cmd[@]}"
done
