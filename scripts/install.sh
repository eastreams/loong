#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: ./scripts/install.sh [--prefix <dir>] [--onboard] [--version <tag>] [--source]

Options:
  --prefix <dir>   Install directory for loongclaw (default: $HOME/.local/bin)
  --onboard        Run `loongclaw onboard` after install
  --version <tag>  Release tag to install (default: latest)
  --source         Build from local source instead of downloading a release binary
  -h, --help       Show this help
USAGE
}

if [[ -n "${BASH_SOURCE[0]:-}" ]]; then
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
else
  script_dir=""
fi

if [[ -n "${script_dir}" && -f "${script_dir}/release_artifact_lib.sh" ]]; then
  # Prefer the shared helper when the installer runs from a repository checkout.
  . "${script_dir}/release_artifact_lib.sh"
else
  release_archive_extension_for_target() {
    local target="${1:?target is required}"
    case "$target" in
      *-pc-windows-*) printf 'zip\n' ;;
      *) printf 'tar.gz\n' ;;
    esac
  }

  release_archive_name() {
    local package_name="${1:?package_name is required}"
    local tag="${2:?tag is required}"
    local target="${3:?target is required}"
    local archive_ext
    archive_ext="$(release_archive_extension_for_target "$target")"
    printf '%s-%s-%s.%s\n' "$package_name" "$tag" "$target" "$archive_ext"
  }

  release_archive_checksum_name() {
    local package_name="${1:?package_name is required}"
    local tag="${2:?tag is required}"
    local target="${3:?target is required}"
    printf '%s.sha256\n' "$(release_archive_name "$package_name" "$tag" "$target")"
  }

  release_binary_name_for_target() {
    local bin_name="${1:?bin_name is required}"
    local target="${2:?target is required}"
    case "$target" in
      *-pc-windows-*) printf '%s.exe\n' "$bin_name" ;;
      *) printf '%s\n' "$bin_name" ;;
    esac
  }

  release_target_for_platform() {
    local platform="${1:?platform is required}"
    local arch="${2:?arch is required}"
    local normalized_platform normalized_arch

    normalized_platform="$(printf '%s' "$platform" | tr '[:lower:]' '[:upper:]')"
    normalized_arch="$(printf '%s' "$arch" | tr '[:upper:]' '[:lower:]')"

    case "$normalized_platform" in
      LINUX)
        case "$normalized_arch" in
          x86_64|amd64) printf 'x86_64-unknown-linux-gnu\n' ;;
          *)
            echo "unsupported Linux architecture: ${arch}" >&2
            return 1
            ;;
        esac
        ;;
      DARWIN)
        case "$normalized_arch" in
          x86_64|amd64) printf 'x86_64-apple-darwin\n' ;;
          arm64|aarch64) printf 'aarch64-apple-darwin\n' ;;
          *)
            echo "unsupported macOS architecture: ${arch}" >&2
            return 1
            ;;
        esac
        ;;
      WINDOWS_NT|MINGW*|MSYS*|CYGWIN*)
        case "$normalized_arch" in
          x86_64|amd64) printf 'x86_64-pc-windows-msvc\n' ;;
          *)
            echo "unsupported Windows architecture: ${arch}" >&2
            return 1
            ;;
        esac
        ;;
      *)
        echo "unsupported platform: ${platform}" >&2
        return 1
        ;;
    esac
  }
fi

prefix="${HOME}/.local/bin"
run_onboard=0
install_source=0
release_version="${LOONGCLAW_INSTALL_VERSION:-latest}"
release_repo="${LOONGCLAW_INSTALL_REPO:-loongclaw-ai/loongclaw}"
release_base_url="https://github.com/${release_repo}/releases"
package_name="loongclaw"
bin_name="loongclaw"

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
    --onboard)
      run_onboard=1
      shift
      ;;
    --version)
      if [[ $# -lt 2 ]]; then
        echo "error: --version requires a release tag or 'latest'" >&2
        exit 2
      fi
      release_version="$2"
      shift 2
      ;;
    --source)
      install_source=1
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

require_command() {
  local command_name="${1:?command_name is required}"
  local install_hint="${2:?install_hint is required}"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: ${command_name} not found in PATH. ${install_hint}" >&2
    exit 1
  fi
}

normalize_release_tag() {
  local raw="${1:?raw version is required}"
  if [[ "$raw" == "latest" ]]; then
    printf 'latest\n'
    return 0
  fi
  if [[ "$raw" == v* ]]; then
    printf '%s\n' "$raw"
    return 0
  fi
  printf 'v%s\n' "$raw"
}

resolve_latest_release_tag() {
  local api_url response tag
  api_url="https://api.github.com/repos/${release_repo}/releases/latest"
  if ! response="$(
    curl -fsSL \
      -H 'Accept: application/vnd.github+json' \
      -H 'User-Agent: LoongClaw-Install' \
      "${api_url}"
  )"; then
    echo "error: no GitHub release is published for ${release_repo} yet. Run this installer from a repository checkout with --source, or install from source manually." >&2
    exit 1
  fi

  tag="$(
    printf '%s\n' "$response" |
      sed -n -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' |
      head -n 1
  )"
  if [[ -z "${tag}" ]]; then
    echo "error: failed to resolve latest GitHub release tag for ${release_repo}" >&2
    exit 1
  fi
  printf '%s\n' "${tag}"
}

extract_archive() {
  local archive_path="${1:?archive_path is required}"
  local destination_dir="${2:?destination_dir is required}"
  case "$archive_path" in
    *.tar.gz) tar -xzf "$archive_path" -C "$destination_dir" ;;
    *.zip)
      require_command "unzip" "Install unzip or use --source inside a repository checkout."
      unzip -q "$archive_path" -d "$destination_dir"
      ;;
    *)
      echo "error: unsupported archive format: ${archive_path}" >&2
      exit 1
      ;;
  esac
}

sha256_file() {
  local file_path="${1:?file_path is required}"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file_path" | awk '{print $1}'
    return 0
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file_path" | awk '{print $1}'
    return 0
  fi
  echo "error: neither shasum nor sha256sum is available for checksum verification" >&2
  exit 1
}

lowercase_value() {
  printf '%s' "${1:?value is required}" | tr '[:upper:]' '[:lower:]'
}

install_from_source() {
  local repo_root source_binary
  require_command "cargo" "Install Rust first: https://rustup.rs"

  repo_root=""
  if [[ -n "${script_dir}" && -f "${script_dir}/../Cargo.toml" ]]; then
    repo_root="$(cd "${script_dir}/.." && pwd)"
  fi
  if [[ -z "${repo_root}" ]]; then
    echo "error: --source requires running this installer from a loongclaw repository checkout" >&2
    exit 1
  fi

  printf '==> Building loongclaw from source (release)\n'
  (
    cd "${repo_root}"
    cargo build -p loongclaw-daemon --bin "${bin_name}" --release --locked
  )

  source_binary="${repo_root}/target/release/${bin_name}"
  if [[ ! -f "${source_binary}" ]]; then
    echo "error: built binary not found at ${source_binary}" >&2
    exit 1
  fi

  mkdir -p "${prefix}"
  install -m 755 "${source_binary}" "${prefix}/${bin_name}"
}

install_from_release() {
  local host_platform host_arch target_tag target archive_name checksum_name
  local archive_url checksum_url binary_name tmp_dir archive_path checksum_path
  local extract_dir installed_binary expected_sha actual_sha

  require_command "curl" "Install curl first or use --source inside a repository checkout."
  require_command "install" "Install coreutils or use --source inside a repository checkout."

  host_platform="$(uname -s)"
  host_arch="$(uname -m)"
  target="$(release_target_for_platform "${host_platform}" "${host_arch}")"
  target_tag="$(normalize_release_tag "${release_version}")"
  if [[ "${target_tag}" == "latest" ]]; then
    target_tag="$(resolve_latest_release_tag)"
  fi

  archive_name="$(release_archive_name "${package_name}" "${target_tag}" "${target}")"
  checksum_name="$(release_archive_checksum_name "${package_name}" "${target_tag}" "${target}")"
  archive_url="${release_base_url}/download/${target_tag}/${archive_name}"
  checksum_url="${release_base_url}/download/${target_tag}/${checksum_name}"
  binary_name="$(release_binary_name_for_target "${bin_name}" "${target}")"

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir}"' RETURN
  archive_path="${tmp_dir}/${archive_name}"
  checksum_path="${tmp_dir}/${checksum_name}"
  extract_dir="${tmp_dir}/extract"
  mkdir -p "${extract_dir}"

  printf '==> Downloading loongclaw %s for %s\n' "${target_tag}" "${target}"
  curl -fsSL --retry 3 --retry-delay 1 -o "${archive_path}" "${archive_url}"
  curl -fsSL --retry 3 --retry-delay 1 -o "${checksum_path}" "${checksum_url}"

  expected_sha="$(awk '{print $1}' "${checksum_path}" | head -n 1)"
  if [[ -z "${expected_sha}" ]]; then
    echo "error: checksum file ${checksum_name} did not contain a SHA256 value" >&2
    exit 1
  fi
  actual_sha="$(sha256_file "${archive_path}")"
  if [[ "$(lowercase_value "${expected_sha}")" != "$(lowercase_value "${actual_sha}")" ]]; then
    echo "error: checksum verification failed for ${archive_name}" >&2
    echo "expected: ${expected_sha}" >&2
    echo "actual:   ${actual_sha}" >&2
    exit 1
  fi

  extract_archive "${archive_path}" "${extract_dir}"
  installed_binary="${extract_dir}/${binary_name}"
  if [[ ! -f "${installed_binary}" ]]; then
    echo "error: extracted binary not found at ${installed_binary}" >&2
    exit 1
  fi

  mkdir -p "${prefix}"
  install -m 755 "${installed_binary}" "${prefix}/${bin_name}"
}

if [[ "${install_source}" -eq 1 ]]; then
  install_from_source
else
  install_from_release
fi

printf '==> Installed loongclaw to %s\n' "${prefix}/${bin_name}"

if [[ "${run_onboard}" -eq 1 ]]; then
  printf '==> Running guided onboarding\n'
  "${prefix}/${bin_name}" onboard
fi

case ":${PATH}:" in
  *":${prefix}:"*)
    ;;
  *)
    printf '\nAdd to PATH if needed:\n  export PATH="%s:$PATH"\n' "${prefix}"
    ;;
esac

printf '\nDone. Try:\n  loongclaw --help\n'
