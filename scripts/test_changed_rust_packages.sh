#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

all_features=0
explicit_paths=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all-features)
      all_features=1
      shift
      ;;
    --)
      shift
      while [[ $# -gt 0 ]]; do
        explicit_paths+=("$1")
        shift
      done
      ;;
    *)
      explicit_paths+=("$1")
      shift
      ;;
  esac
done

collect_changed_files() {
  if [[ "${#explicit_paths[@]}" -gt 0 ]]; then
    printf '%s\n' "${explicit_paths[@]}"
    return
  fi
  git diff --name-only --relative HEAD
  git diff --name-only --cached --relative
  git ls-files --others --exclude-standard
}

collect_daemon_test_targets() {
  python3 scripts/daemon_changed_test_targets.py --format names "$@"
}

changed_files=()
while IFS= read -r line; do
  changed_files+=("$line")
done < <(collect_changed_files | awk 'NF' | sort -u)
if [[ "${#changed_files[@]}" -eq 0 ]]; then
  echo "[test:changed] no local file changes detected"
  exit 0
fi

package_names=()
while IFS= read -r line; do
  package_names+=("$line")
done < <(python3 scripts/rust_changed_packages.py --format names "${changed_files[@]}")
if [[ "${#package_names[@]}" -eq 0 ]]; then
  echo "[test:changed] no Rust workspace packages matched the local changes"
  exit 0
fi

echo "[test:changed] files:"
printf '  %s\n' "${changed_files[@]}"
echo "[test:changed] packages: ${package_names[*]}"

other_packages=()
run_daemon=0
for package_name in "${package_names[@]}"; do
  if [[ "$package_name" == "loong" ]]; then
    run_daemon=1
    continue
  fi
  other_packages+=("-p" "$package_name")
done

if [[ "${#other_packages[@]}" -gt 0 ]]; then
  cargo_args=(test --locked)
  if [[ "$all_features" -eq 1 ]]; then
    cargo_args+=(--all-features)
  fi
  cargo_args+=("${other_packages[@]}" --lib --tests)
  ./scripts/cargo-local-toolchain.sh "${cargo_args[@]}"
fi

if [[ "$run_daemon" -eq 1 ]]; then
  daemon_test_targets=()
  while IFS= read -r line; do
    daemon_test_targets+=("$line")
  done < <(collect_daemon_test_targets "${changed_files[@]}")

  if [[ "${#daemon_test_targets[@]}" -eq 0 ]]; then
    daemon_test_targets=("daemon_smoke")
  fi

  echo "[test:changed] daemon targets: ${daemon_test_targets[*]}"

  daemon_runner_args=()
  if [[ "$all_features" -eq 1 ]]; then
    daemon_runner_args+=(--all-features)
  fi
  daemon_runner_args+=(--include-lib-bins)
  daemon_runner_args+=("${daemon_test_targets[@]}")
  ./scripts/run_selected_daemon_tests.sh "${daemon_runner_args[@]}"
fi
