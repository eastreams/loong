#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: ./scripts/install.sh [--prefix <dir>] [--onboard] [--version <tag>] [--source] [--target-libc <gnu|musl>]

Options:
  --prefix <dir>   Install directory for loong (default: $HOME/.local/bin)
  --onboard        Run `loong onboard` after install
  --version <tag>  Release tag to install (default: latest)
  --source         Build from local source instead of downloading a release binary
  --target-libc    Override Linux libc target selection (`gnu` or `musl`)
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

  release_normalize_linux_arch() {
    local arch="${1:?arch is required}"
    local normalized_arch
    normalized_arch="$(printf '%s' "$arch" | tr '[:upper:]' '[:lower:]')"

    case "$normalized_arch" in
      x86_64|amd64) printf 'x86_64\n' ;;
      arm64|aarch64) printf 'aarch64\n' ;;
      *)
        echo "unsupported Linux architecture: ${arch}" >&2
        return 1
        ;;
    esac
  }

  release_supported_linux_libcs_for_arch() {
    local arch
    arch="$(release_normalize_linux_arch "${1:?arch is required}")" || return 1

    case "$arch" in
      x86_64) printf 'gnu\nmusl\n' ;;
      aarch64) printf 'gnu\n' ;;
      *)
        echo "unsupported Linux architecture: ${1}" >&2
        return 1
        ;;
    esac
  }

  release_linux_target_for_arch_and_libc() {
    local arch libc
    arch="$(release_normalize_linux_arch "${1:?arch is required}")" || return 1
    libc="$(printf '%s' "${2:?libc is required}" | tr '[:upper:]' '[:lower:]')"

    case "$arch:$libc" in
      x86_64:gnu) printf 'x86_64-unknown-linux-gnu\n' ;;
      x86_64:musl) printf 'x86_64-unknown-linux-musl\n' ;;
      aarch64:gnu) printf 'aarch64-unknown-linux-gnu\n' ;;
      *)
        echo "unsupported Linux architecture/libc combination: ${arch}/${libc}" >&2
        return 1
        ;;
    esac
  }

  release_gnu_glibc_floor_for_target() {
    local target="${1:?target is required}"

    case "$target" in
      x86_64-unknown-linux-gnu) printf '2.39\n' ;;
      aarch64-unknown-linux-gnu) printf '2.17\n' ;;
      *)
        echo "unsupported GNU Linux target for glibc floor lookup: ${target}" >&2
        return 1
        ;;
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
        release_linux_target_for_arch_and_libc "$normalized_arch" "gnu"
        ;;
      ANDROID)
        case "$normalized_arch" in
          arm64|aarch64) printf 'aarch64-linux-android\n' ;;
          *)
            echo "unsupported Android architecture: ${arch}" >&2
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

is_termux_environment() {
  local uname_operating_system

  uname_operating_system="$(uname -o 2>/dev/null || true)"
  if [[ -n "$uname_operating_system" ]]; then
    if [[ "$(printf '%s' "$uname_operating_system" | tr '[:lower:]' '[:upper:]')" == "ANDROID" ]]; then
      return 0
    fi
    return 1
  fi

  if [[ -n "${TERMUX_VERSION:-}" ]]; then
    return 0
  fi

  case "${PREFIX:-}" in
    */com.termux/files/usr) return 0 ;;
  esac

  return 1
}

detect_release_host_platform() {
  local host_platform
  host_platform="$(uname -s)"

  if [[ "$(printf '%s' "$host_platform" | tr '[:lower:]' '[:upper:]')" == "LINUX" ]] && is_termux_environment; then
    printf 'Android\n'
    return 0
  fi

  printf '%s\n' "$host_platform"
}

prefix="${HOME}/.local/bin"
run_onboard=0
install_source=0
release_version="${LOONGCLAW_INSTALL_VERSION:-latest}"
release_repo="${LOONGCLAW_INSTALL_REPO:-loongclaw-ai/loongclaw}"
release_base_url="${LOONGCLAW_INSTALL_RELEASE_BASE_URL:-https://github.com/${release_repo}/releases}"
target_libc="${LOONGCLAW_INSTALL_TARGET_LIBC:-auto}"
package_name="loong"
bin_name="loong"
legacy_bin_name="loongclaw"

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
    --target-libc)
      if [[ $# -lt 2 ]]; then
        echo "error: --target-libc requires 'gnu' or 'musl'" >&2
        exit 2
      fi
      target_libc="$2"
      shift 2
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

print_missing_release_guidance() {
  cat >&2 <<EOF
error: no GitHub release is published for ${release_repo} yet.

Install from a local checkout instead:
  git clone https://github.com/${release_repo}.git
  cd $(basename "${release_repo}")
  bash scripts/install.sh --source --onboard
EOF
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
    print_missing_release_guidance
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
  local value="${1-}"
  printf '%s' "${value}" | tr '[:upper:]' '[:lower:]'
}

install_binary_pair() {
  local source_path="${1:?source_path is required}"
  local primary_output_name="${2:?primary_output_name is required}"
  local legacy_output_name="${3:?legacy_output_name is required}"

  mkdir -p "${prefix}"
  install -m 755 "${source_path}" "${prefix}/${primary_output_name}"
  install -m 755 "${source_path}" "${prefix}/${legacy_output_name}"
}

install_web_search_provider_display_name() {
  local provider="${1:-}"
  case "$(lowercase_value "${provider:-unknown}")" in
    ddg|duckduckgo) printf 'DuckDuckGo\n' ;;
    tavily) printf 'Tavily\n' ;;
    brave) printf 'Brave Search\n' ;;
    perplexity) printf 'Perplexity Search\n' ;;
    exa) printf 'Exa\n' ;;
    jina) printf 'Jina Search\n' ;;
    *) printf '%s\n' "${provider}" ;;
  esac
}

install_locale_looks_domestic_cn() {
  local value normalized

  for value in "${LC_ALL:-}" "${LC_MESSAGES:-}" "${LANG:-}"; do
    if [[ -n "${value}" ]]; then
      normalized="$(lowercase_value "${value}")"
      if [[ "${normalized}" == *"zh_cn"* || "${normalized}" == *"zh-hans"* || "${normalized}" == zh-cn* ]]; then
        return 0
      fi
    fi
  done

  normalized="$(lowercase_value "${TZ:-}")"
  case "${normalized}" in
    asia/shanghai|asia/chongqing|asia/harbin|asia/urumqi|asia/beijing) return 0 ;;
  esac

  return 1
}

probe_install_duckduckgo_route() {
  if ! command -v curl >/dev/null 2>&1; then
    return 1
  fi

  curl -fsSL \
    --retry 0 \
    --max-time 2 \
    -o /dev/null \
    "https://html.duckduckgo.com/html/?q=loongclaw" >/dev/null 2>&1
}

probe_install_tavily_route() {
  local http_code

  if ! command -v curl >/dev/null 2>&1; then
    return 1
  fi

  http_code="$(
    curl -sS \
      --retry 0 \
      --max-time 2 \
      -o /dev/null \
      -w '%{http_code}' \
      -H 'Content-Type: application/json' \
      -d '{"query":"loongclaw","max_results":1}' \
      "https://api.tavily.com/search" 2>/dev/null || true
  )"

  case "${http_code}" in
    2??|3??|4??) return 0 ;;
  esac

  return 1
}

install_env_has_non_empty_value() {
  local env_name="${1:?env_name is required}"
  local value="${!env_name:-}"

  [[ -n "${value}" && -n "${value//[[:space:]]/}" ]]
}

install_web_search_provider_has_ready_credential() {
  local provider="${1:?provider is required}"

  case "$provider" in
    brave)
      install_env_has_non_empty_value "BRAVE_API_KEY"
      ;;
    tavily)
      install_env_has_non_empty_value "TAVILY_API_KEY"
      ;;
    perplexity)
      install_env_has_non_empty_value "PERPLEXITY_API_KEY"
      ;;
    exa)
      install_env_has_non_empty_value "EXA_API_KEY"
      ;;
    jina)
      install_env_has_non_empty_value "JINA_API_KEY" \
        || install_env_has_non_empty_value "JINA_AUTH_TOKEN"
      ;;
    *)
      return 1
      ;;
  esac
}

recommend_onboard_web_search_provider_from_credentials() {
  local ready_provider=""
  local ready_count=0
  local provider=""

  for provider in brave tavily perplexity exa jina; do
    if ! install_web_search_provider_has_ready_credential "$provider"; then
      continue
    fi

    ready_provider="$provider"
    ready_count=$((ready_count + 1))
  done

  if [[ "$ready_count" -ne 1 ]]; then
    return 1
  fi

  printf '%s\n' "$ready_provider"
}

format_install_web_search_provider_source() {
  local source="${1:?source is required}"

  case "$source" in
    preconfigured)
      printf 'preconfigured\n'
      ;;
    detected-credential)
      printf 'detected credential\n'
      ;;
    detected-signal)
      printf 'detected\n'
      ;;
    *)
      printf '%s\n' "$source"
      ;;
  esac
}

recommend_onboard_web_search_provider() {
  local domestic_locale_hint=0
  local duckduckgo_reachable=0
  local tavily_reachable=0
  local credential_provider=""

  if install_locale_looks_domestic_cn; then
    domestic_locale_hint=1
  fi
  credential_provider="$(recommend_onboard_web_search_provider_from_credentials || true)"
  if [[ -n "$credential_provider" ]]; then
    printf '%s|%s\n' "$credential_provider" "detected-credential"
    return 0
  fi
  if probe_install_duckduckgo_route; then
    duckduckgo_reachable=1
  fi
  if probe_install_tavily_route; then
    tavily_reachable=1
  fi

  if [[ "${domestic_locale_hint}" -eq 1 ]] && [[ "${tavily_reachable}" -eq 1 || "${duckduckgo_reachable}" -eq 0 ]]; then
    printf '%s|%s\n' "tavily" "detected-signal"
    return 0
  fi

  if [[ "${duckduckgo_reachable}" -eq 1 ]]; then
    printf '%s|%s\n' "duckduckgo" "detected-signal"
    return 0
  fi

  if [[ "${tavily_reachable}" -eq 1 ]]; then
    printf '%s|%s\n' "tavily" "detected-signal"
    return 0
  fi

  if [[ "${domestic_locale_hint}" -eq 1 ]]; then
    printf '%s|%s\n' "tavily" "detected-signal"
    return 0
  fi

  printf '%s|%s\n' "duckduckgo" "detected-signal"
}

run_guided_onboarding() {
  local selected_provider
  local provider_source
  local recommendation

  if [[ -n "${LOONGCLAW_WEB_SEARCH_PROVIDER:-}" ]]; then
    selected_provider="${LOONGCLAW_WEB_SEARCH_PROVIDER}"
    provider_source="preconfigured"
  else
    recommendation="$(recommend_onboard_web_search_provider)"
    selected_provider="${recommendation%%|*}"
    provider_source="${recommendation#*|}"
  fi

  if [[ -n "${selected_provider}" ]]; then
    printf '==> Onboarding web search default: %s (%s)\n' \
      "$(install_web_search_provider_display_name "${selected_provider}")" \
      "$(format_install_web_search_provider_source "${provider_source}")"
    "${prefix}/${bin_name}" onboard --web-search-provider "${selected_provider}"
    return 0
  fi

  "${prefix}/${bin_name}" onboard
}

normalize_target_libc() {
  local raw="${1:-auto}"
  local normalized
  normalized="$(lowercase_value "$raw")"

  case "$normalized" in
    auto|"") printf 'auto\n' ;;
    gnu|musl) printf '%s\n' "$normalized" ;;
    *)
      echo "error: unsupported --target-libc value: ${raw} (expected gnu or musl)" >&2
      exit 2
      ;;
  esac
}

parse_glibc_version() {
  local input="${1:-}"
  local parsed

  parsed="$(printf '%s\n' "$input" | grep -oE '[0-9]+(\.[0-9]+){1,2}' | head -n 1 || true)"
  if [[ -n "$parsed" ]]; then
    printf '%s\n' "$parsed"
    return 0
  fi
  return 1
}

detect_host_glibc_version() {
  local output normalized_output version

  if command -v getconf >/dev/null 2>&1; then
    if output="$(getconf GNU_LIBC_VERSION 2>/dev/null)"; then
      version="$(parse_glibc_version "$output" || true)"
      if [[ -n "$version" ]]; then
        printf '%s\n' "$version"
        return 0
      fi
    fi
  fi

  if command -v ldd >/dev/null 2>&1; then
    if output="$(ldd --version 2>&1 | head -n 1)"; then
      normalized_output="$(printf '%s' "$output" | tr '[:upper:]' '[:lower:]')"
      if [[ "$normalized_output" != *musl* ]] && \
        [[ "$normalized_output" == *glibc* || "$normalized_output" == *"gnu libc"* || "$normalized_output" == *"gnu c library"* ]]; then
        version="$(parse_glibc_version "$output" || true)"
        if [[ -n "$version" ]]; then
          printf '%s\n' "$version"
          return 0
        fi
      fi
    fi
  fi

  return 1
}

compare_versions() {
  local actual="${1:?actual version is required}"
  local minimum="${2:?minimum version is required}"
  local IFS=.
  local -a actual_parts=() minimum_parts=()
  local len i a m

  read -r -a actual_parts <<< "$actual"
  read -r -a minimum_parts <<< "$minimum"

  len="${#actual_parts[@]}"
  if (( ${#minimum_parts[@]} > len )); then
    len="${#minimum_parts[@]}"
  fi

  for (( i = 0; i < len; i++ )); do
    a="${actual_parts[i]:-0}"
    m="${minimum_parts[i]:-0}"
    [[ "$a" =~ ^[0-9]+$ ]] || a=0
    [[ "$m" =~ ^[0-9]+$ ]] || m=0

    if (( 10#$a > 10#$m )); then
      return 0
    fi
    if (( 10#$a < 10#$m )); then
      return 1
    fi
  done

  return 0
}

supports_sort_version() {
  local sorted
  if ! sorted="$(printf '2.9\n2.10\n' | sort -V 2>/dev/null)"; then
    return 1
  fi

  [[ "$sorted" == $'2.9\n2.10' ]]
}

version_at_least() {
  local actual="${1:?actual version is required}"
  local minimum="${2:?minimum version is required}"

  if supports_sort_version; then
    [[ "$(printf '%s\n%s\n' "$minimum" "$actual" | sort -V | head -n 1)" == "$minimum" ]]
    return $?
  fi

  compare_versions "$actual" "$minimum"
}

release_target_for_install() {
  local platform="${1:?platform is required}"
  local arch="${2:?arch is required}"
  local requested_libc="${3:?requested_libc is required}"
  local normalized_platform normalized_libc normalized_arch gnu_target musl_target required_glibc detected_glibc

  normalized_platform="$(printf '%s' "$platform" | tr '[:lower:]' '[:upper:]')"
  normalized_libc="$(normalize_target_libc "$requested_libc")"

  if [[ "$normalized_platform" != "LINUX" ]]; then
    if [[ "$normalized_libc" != "auto" ]]; then
      echo "error: --target-libc is only supported for Linux installs" >&2
      exit 2
    fi
    release_target_for_platform "$platform" "$arch"
    return 0
  fi

  normalized_arch="$(release_normalize_linux_arch "$arch")"
  gnu_target="$(release_linux_target_for_arch_and_libc "$normalized_arch" "gnu")"

  if [[ "$normalized_libc" == "gnu" ]]; then
    if ! detected_glibc="$(detect_host_glibc_version)"; then
      echo "error: explicit GNU install requires detectable glibc on the host; use --target-libc musl instead" >&2
      exit 1
    fi
    required_glibc="$(release_gnu_glibc_floor_for_target "$gnu_target")"
    if ! version_at_least "$detected_glibc" "$required_glibc"; then
      printf 'error: %s requires glibc >= %s but the host reports %s; use --target-libc musl instead\n' \
        "$gnu_target" \
        "$required_glibc" \
        "$detected_glibc" >&2
      exit 1
    fi
    printf '%s\n' "$gnu_target"
    return 0
  fi

  if [[ "$normalized_libc" == "musl" ]]; then
    release_linux_target_for_arch_and_libc "$normalized_arch" "musl"
    return 0
  fi

  if detected_glibc="$(detect_host_glibc_version)"; then
    required_glibc="$(release_gnu_glibc_floor_for_target "$gnu_target")"
    if version_at_least "$detected_glibc" "$required_glibc"; then
      printf '%s\n' "$gnu_target"
      return 0
    fi
  fi

  musl_target="$(release_linux_target_for_arch_and_libc "$normalized_arch" "musl" || true)"
  if [[ -n "$musl_target" ]]; then
    printf '%s\n' "$musl_target"
    return 0
  fi

  if [[ -n "${detected_glibc:-}" ]]; then
    printf 'error: %s requires glibc >= %s but the host reports %s; no musl release artifact is published for %s; use --source instead\n' \
      "$gnu_target" \
      "$required_glibc" \
      "$detected_glibc" \
      "$normalized_arch" >&2
  else
    printf 'error: could not detect a compatible glibc on the host and no musl release artifact is published for %s; use --source instead\n' \
      "$normalized_arch" >&2
  fi
  exit 1
}

install_from_source() {
  local repo_root host_target source_binary primary_binary_name legacy_binary_name
  require_command "cargo" "Install Rust first: https://rustup.rs"
  require_command "install" "Install coreutils or use a different shell environment."

  repo_root=""
  if [[ -n "${script_dir}" && -f "${script_dir}/../Cargo.toml" ]]; then
    repo_root="$(cd "${script_dir}/.." && pwd)"
  fi
  if [[ -z "${repo_root}" ]]; then
    echo "error: --source requires running this installer from a loongclaw repository checkout" >&2
    exit 1
  fi

  host_target="$(release_target_for_platform "$(detect_release_host_platform)" "$(uname -m)")"
  primary_binary_name="$(release_binary_name_for_target "${bin_name}" "${host_target}")"
  legacy_binary_name="$(release_binary_name_for_target "${legacy_bin_name}" "${host_target}")"

  printf '==> Building loong from source (release)\n'
  (
    cd "${repo_root}"
    LOONGCLAW_RELEASE_BUILD=1 \
      cargo build -p loongclaw-daemon --bin "${bin_name}" --release --locked
  )

  source_binary="${repo_root}/target/release/${primary_binary_name}"
  if [[ ! -f "${source_binary}" ]]; then
    echo "error: built binary not found at ${source_binary}" >&2
    exit 1
  fi

  install_binary_pair "${source_binary}" "${primary_binary_name}" "${legacy_binary_name}"
}

install_from_release() {
  local host_platform host_arch target_tag target archive_name checksum_name
  local archive_url checksum_url binary_name legacy_binary_name tmp_dir archive_path checksum_path
  local extract_dir installed_binary expected_sha actual_sha

  require_command "curl" "Install curl first or use --source inside a repository checkout."
  require_command "install" "Install coreutils or use --source inside a repository checkout."

  host_platform="$(detect_release_host_platform)"
  host_arch="$(uname -m)"
  target="$(release_target_for_install "${host_platform}" "${host_arch}" "${target_libc}")"
  target_tag="$(normalize_release_tag "${release_version}")"
  if [[ "${target_tag}" == "latest" ]]; then
    target_tag="$(resolve_latest_release_tag)"
  fi

  archive_name="$(release_archive_name "${package_name}" "${target_tag}" "${target}")"
  checksum_name="$(release_archive_checksum_name "${package_name}" "${target_tag}" "${target}")"
  archive_url="${release_base_url}/download/${target_tag}/${archive_name}"
  checksum_url="${release_base_url}/download/${target_tag}/${checksum_name}"
  binary_name="$(release_binary_name_for_target "${bin_name}" "${target}")"
  legacy_binary_name="$(release_binary_name_for_target "${legacy_bin_name}" "${target}")"

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir}"' RETURN
  archive_path="${tmp_dir}/${archive_name}"
  checksum_path="${tmp_dir}/${checksum_name}"
  extract_dir="${tmp_dir}/extract"
  mkdir -p "${extract_dir}"

  printf '==> Downloading loong %s for %s\n' "${target_tag}" "${target}"
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

  install_binary_pair "${installed_binary}" "${binary_name}" "${legacy_binary_name}"
}

if [[ "${install_source}" -eq 1 ]]; then
  install_from_source
else
  install_from_release
fi

printf '==> Installed loong to %s\n' "${prefix}/${bin_name}"
printf '==> Installed compatible loongclaw command to %s\n' "${prefix}/${legacy_bin_name}"

if [[ "${run_onboard}" -eq 1 ]]; then
  printf '==> Running guided onboarding\n'
  run_guided_onboarding
fi

case ":${PATH}:" in
  *":${prefix}:"*)
    ;;
  *)
    printf '\nAdd to PATH if needed:\n  export PATH="%s:$PATH"\n' "${prefix}"
    ;;
esac

printf '\nDone. Try:\n  loong --help\n'
