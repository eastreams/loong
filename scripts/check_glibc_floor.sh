#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/check_glibc_floor.sh <binary-path> <max-glibc-version>
USAGE
}

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 2
fi

bin_path="${1:?binary path is required}"
max_glibc="${2:?maximum glibc version is required}"

if [[ ! -f "$bin_path" ]]; then
  echo "error: binary not found: $bin_path" >&2
  exit 1
fi

if ! command -v readelf >/dev/null 2>&1; then
  echo "error: readelf is required to inspect glibc version references" >&2
  exit 1
fi

if ! readelf_output="$(readelf --version-info --wide "$bin_path" 2>&1)"; then
  printf 'error: failed to inspect %s with readelf\n%s\n' "$bin_path" "$readelf_output" >&2
  exit 1
fi

glibc_versions="$(
  printf '%s\n' "$readelf_output" |
    grep -oE 'GLIBC_[0-9]+(\.[0-9]+){1,2}' || true
)"

max_required="$(
  printf '%s\n' "$glibc_versions" |
    sed '/^$/d; s/^GLIBC_//' |
    sort -V |
    tail -n 1
)"

echo "Max required GLIBC: ${max_required:-none}"

if [[ -z "${max_required:-}" ]]; then
  echo "error: failed to detect required GLIBC version from $bin_path" >&2
  exit 1
fi

if [[ "$(printf '%s\n%s\n' "$max_glibc" "$max_required" | sort -V | tail -n 1)" != "$max_glibc" ]]; then
  echo "error: Binary requires GLIBC_${max_required}, expected <= GLIBC_${max_glibc}" >&2
  exit 1
fi

echo "GLIBC floor check passed for $bin_path"
