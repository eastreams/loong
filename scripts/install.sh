#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: ./scripts/install.sh [--prefix <dir>] [--setup]

Options:
  --prefix <dir>   Install directory for loongclaw (default: $HOME/.local/bin)
  --setup          Run `loongclaw setup --force` after install
  -h, --help       Show this help
USAGE
}

prefix="${HOME}/.local/bin"
run_setup=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      if [[ $# -lt 2 ]]; then
        echo "error: --prefix requires a directory" >&2
        exit 2
      fi
      prefix="$2"
      shift 2
      ;;
    --setup)
      run_setup=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found in PATH. Install Rust first: https://rustup.rs" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

printf '==> Building loongclaw (release)\n'
(
  cd "${repo_root}"
  cargo build -p loongclaw-daemon --bin loongclaw --release
)

mkdir -p "${prefix}"
install -m 755 "${repo_root}/target/release/loongclaw" "${prefix}/loongclaw"

printf '==> Installed loongclaw to %s\n' "${prefix}/loongclaw"

if [[ "${run_setup}" -eq 1 ]]; then
  printf '==> Running initial setup\n'
  "${prefix}/loongclaw" setup --force
fi

case ":${PATH}:" in
  *":${prefix}:"*)
    ;;
  *)
    printf '\nAdd to PATH if needed:\n  export PATH="%s:$PATH"\n' "${prefix}"
    ;;
esac

printf '\nDone. Try:\n  loongclaw --help\n'
