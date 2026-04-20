#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

collect_changed_files() {
  git diff --name-only --relative HEAD
  git diff --name-only --cached --relative
  git ls-files --others --exclude-standard
}

collect_daemon_test_targets() {
  python3 scripts/daemon_changed_test_targets.py --format names "$@"
}

run_daemon_test_target() {
  local target_name="$1"

  echo "[test:changed] daemon target: ${target_name}"

  if [[ "$target_name" == "daemon_smoke" ]]; then
    ./scripts/test_daemon_smoke.sh
    return
  fi

  if [[ "$target_name" == "integration" ]]; then
    ./scripts/cargo-local-toolchain.sh test --locked -p loong --test integration
    return
  fi

  ./scripts/cargo-local-toolchain.sh test --locked -p loong --features test-support --test "$target_name"
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
  ./scripts/cargo-local-toolchain.sh test --locked "${other_packages[@]}" --lib --tests
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

  ./scripts/cargo-local-toolchain.sh test --locked -p loong --lib --bins

  for target_name in "${daemon_test_targets[@]}"; do
    run_daemon_test_target "$target_name"
  done
fi
