#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${LOONG_RELEASE_ARTIFACTS_REPO_ROOT:-}" ]]; then
  REPO_ROOT="$LOONG_RELEASE_ARTIFACTS_REPO_ROOT"
else
  REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
cd "$REPO_ROOT"
. "$REPO_ROOT/scripts/release_artifact_lib.sh"

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

mkdir -p .docs/releases .docs/traces .docs/traces/by-tag

tmp_index="$(mktemp)"
trap 'rm -f "$tmp_index"' EXIT
: >"$tmp_index"

latest_version=""
latest_trace_path=""
processed=0

while IFS= read -r version; do
  [[ -z "$version" ]] && continue
  tag="$(release_tag_from_version "$version")"
  doc_path="docs/releases/${tag}.md"
  if [[ ! -f "$doc_path" ]]; then
    echo "missing release doc for ${tag}: ${doc_path}" >&2
    exit 1
  fi

  trace_id="$(release_doc_backticked_field "$doc_path" "Trace ID")"
  trace_path="$(release_doc_backticked_field "$doc_path" "Trace path")"
  generated_at="$(release_doc_generated_at "$doc_path")"

  if [[ -z "$trace_id" ]]; then
    echo "missing Trace ID in ${doc_path}" >&2
    exit 1
  fi
  if [[ -z "$trace_path" ]]; then
    echo "missing Trace path in ${doc_path}" >&2
    exit 1
  fi
  if ! release_trace_path_matches_contract "$tag" "$trace_id" "$trace_path"; then
    trace_basename="$(basename "$trace_path")"
    expected_trace_suffix="$(release_trace_expected_suffix "$tag" "$trace_id")"
    if [[ "$trace_path" != .docs/traces/* ]]; then
      echo "invalid Trace path in ${doc_path}: ${trace_path}" >&2
    elif ! release_trace_path_segments_safe "$trace_path"; then
      echo "invalid Trace path in ${doc_path}: ${trace_path}" >&2
    elif ! release_trace_path_symlink_prefixes_safe "$trace_path"; then
      echo "invalid Trace path in ${doc_path}: ${trace_path}" >&2
    elif [[ "$trace_basename" != *"-post-release-"* ]]; then
      echo "Trace path basename must include -post-release- in ${doc_path}: ${trace_basename}" >&2
    else
      echo "Trace path basename must end with ${expected_trace_suffix} in ${doc_path}: ${trace_basename}" >&2
    fi
    exit 1
  fi

  mkdir -p "$(dirname "$trace_path")" "$trace_path" ".docs/traces/by-tag/${tag}"

  debug_doc_relpath="$(release_debug_doc_relpath "$tag")"

  cat >"$debug_doc_relpath" <<EOF
# Local Release Debug ${tag}

- Source release doc: \`${doc_path}\`
- Trace ID: \`${trace_id}\`
- Trace path: \`${trace_path}\`
- Generated at: ${generated_at:-unknown}
- Bootstrap source: \`scripts/bootstrap_release_local_artifacts.sh\`

This local debug artifact was regenerated from the tracked release document.
EOF

  cat >"${trace_path}/metadata.json" <<EOF
{"tag":"$(json_escape "$tag")","trace_id":"$(json_escape "$trace_id")","trace_path":"$(json_escape "$trace_path")","command":"post-release","status":"success","source_release_doc":"$(json_escape "$doc_path")"}
EOF

  printf '%s\n' "$trace_path" >".docs/traces/by-tag/${tag}/latest"
  printf '{"tag":"%s","trace_id":"%s","trace_path":"%s","command":"post-release","status":"success","source_release_doc":"%s"}\n' \
    "$(json_escape "$tag")" \
    "$(json_escape "$trace_id")" \
    "$(json_escape "$trace_path")" \
    "$(json_escape "$doc_path")" >>"$tmp_index"

  if [[ -z "$latest_version" ]] || version_is_greater "$version" "$latest_version"; then
    latest_version="$version"
    latest_trace_path="$trace_path"
  fi

  processed=$((processed + 1))
done <<EOF
$(release_versions_from_changelog CHANGELOG.md)
EOF

if (( processed == 0 )); then
  echo "no released versions found in CHANGELOG.md" >&2
  exit 1
fi

cp "$tmp_index" .docs/traces/index.jsonl
printf '%s\n' "$latest_trace_path" >.docs/traces/latest

echo "[release-artifacts] bootstrapped ${processed} release debug doc(s)"
echo "[release-artifacts] index: .docs/traces/index.jsonl"
echo "[release-artifacts] latest: ${latest_trace_path}"
