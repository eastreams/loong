#!/usr/bin/env bash

release_versions_from_changelog() {
  local changelog_path="${1:?changelog_path is required}"
  grep -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?\]' "$changelog_path" | \
    sed -E 's/^## \[([0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?)\]$/\1/'
}

release_tag_from_version() {
  local version="${1:?version is required}"
  printf 'v%s\n' "$version"
}

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
        arm64|aarch64) printf 'aarch64-unknown-linux-gnu\n' ;;
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

release_doc_backticked_field() {
  local doc_path="${1:?doc_path is required}"
  local field_label="${2:?field_label is required}"
  sed -n -E "s/^- ${field_label}: \`([^\\\`]+)\`\$/\\1/p" "$doc_path" | head -n 1
}

release_doc_generated_at() {
  local doc_path="${1:?doc_path is required}"
  sed -n -E 's/^- Generated at: (.+)$/\1/p' "$doc_path" | head -n 1
}

release_debug_doc_relpath() {
  local tag="${1:?tag is required}"
  printf '.docs/releases/%s-debug.md\n' "$tag"
}

parse_release_version() {
  local version="${1:?version is required}"
  local matches
  if [[ "$version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)(-([0-9A-Za-z]+(\.[0-9A-Za-z]+)*))?$ ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}" "${BASH_REMATCH[5]}"
    return 0
  fi

  echo "invalid release version: ${version}" >&2
  return 1
}

is_numeric_identifier() {
  local value="${1:?value is required}"
  [[ "$value" =~ ^[0-9]+$ ]]
}

compare_prerelease_identifiers() {
  local left="${1-}"
  local right="${2-}"
  local left_parts right_parts
  local i left_part right_part

  IFS='.' read -r -a left_parts <<< "$left"
  IFS='.' read -r -a right_parts <<< "$right"

  for (( i = 0; i < ${#left_parts[@]} || i < ${#right_parts[@]}; i++ )); do
    left_part="${left_parts[i]-}"
    right_part="${right_parts[i]-}"

    if [[ -z "$left_part" && -n "$right_part" ]]; then
      printf -- '-1\n'
      return 0
    fi
    if [[ -n "$left_part" && -z "$right_part" ]]; then
      printf '1\n'
      return 0
    fi

    if is_numeric_identifier "$left_part" && is_numeric_identifier "$right_part"; then
      if (( 10#$left_part > 10#$right_part )); then
        printf '1\n'
        return 0
      fi
      if (( 10#$left_part < 10#$right_part )); then
        printf -- '-1\n'
        return 0
      fi
      continue
    fi

    if is_numeric_identifier "$left_part" && ! is_numeric_identifier "$right_part"; then
      printf -- '-1\n'
      return 0
    fi
    if ! is_numeric_identifier "$left_part" && is_numeric_identifier "$right_part"; then
      printf '1\n'
      return 0
    fi

    if [[ "$left_part" > "$right_part" ]]; then
      printf '1\n'
      return 0
    fi
    if [[ "$left_part" < "$right_part" ]]; then
      printf -- '-1\n'
      return 0
    fi
  done

  printf '0\n'
}

version_is_greater() {
  local left="${1:?left version is required}"
  local right="${2:?right version is required}"
  local left_major left_minor left_patch left_prerelease
  local right_major right_minor right_patch right_prerelease
  local prerelease_cmp

  mapfile -t left_parts < <(parse_release_version "$left")
  mapfile -t right_parts < <(parse_release_version "$right")
  left_major="${left_parts[0]}"
  left_minor="${left_parts[1]}"
  left_patch="${left_parts[2]}"
  left_prerelease="${left_parts[3]}"
  right_major="${right_parts[0]}"
  right_minor="${right_parts[1]}"
  right_patch="${right_parts[2]}"
  right_prerelease="${right_parts[3]}"

  if (( left_major > right_major )); then
    return 0
  fi
  if (( left_major < right_major )); then
    return 1
  fi
  if (( left_minor > right_minor )); then
    return 0
  fi
  if (( left_minor < right_minor )); then
    return 1
  fi
  if (( left_patch > right_patch )); then
    return 0
  fi
  if (( left_patch < right_patch )); then
    return 1
  fi

  if [[ -z "$left_prerelease" && -z "$right_prerelease" ]]; then
    return 1
  fi
  if [[ -z "$left_prerelease" && -n "$right_prerelease" ]]; then
    return 0
  fi
  if [[ -n "$left_prerelease" && -z "$right_prerelease" ]]; then
    return 1
  fi

  prerelease_cmp="$(compare_prerelease_identifiers "$left_prerelease" "$right_prerelease")"
  if (( prerelease_cmp > 0 )); then
    return 0
  fi
  return 1
}

release_trace_expected_suffix() {
  local tag="${1:?tag is required}"
  local trace_id="${2:?trace_id is required}"
  printf -- '-%s-%s\n' "$tag" "$trace_id"
}

release_trace_path_segments_safe() {
  local trace_path="${1:?trace_path is required}"
  local segment
  local IFS='/'
  read -r -a segments <<< "$trace_path"
  for segment in "${segments[@]}"; do
    [[ -n "$segment" ]] || return 1
    [[ "$segment" != "." && "$segment" != ".." ]] || return 1
  done
}

release_trace_path_symlink_prefixes_safe() {
  local trace_path="${1:?trace_path is required}"
  local segment
  local prefix=""
  local IFS='/'
  read -r -a segments <<< "$trace_path"
  for segment in "${segments[@]}"; do
    prefix="${prefix:+${prefix}/}${segment}"
    if [[ -L "$prefix" ]]; then
      return 1
    fi
  done
}

release_trace_path_matches_contract() {
  local tag="${1:?tag is required}"
  local trace_id="${2:?trace_id is required}"
  local trace_path="${3:?trace_path is required}"

  [[ "$trace_path" == .docs/traces/* ]] || return 1
  release_trace_path_segments_safe "$trace_path" || return 1
  release_trace_path_symlink_prefixes_safe "$trace_path" || return 1
  local trace_basename
  trace_basename="$(basename "$trace_path")"
  [[ "$trace_basename" == *"-post-release-"* ]] || return 1
  local expected_suffix
  expected_suffix="$(release_trace_expected_suffix "$tag" "$trace_id")"
  [[ "$trace_basename" == *"$expected_suffix" ]] || return 1
}
