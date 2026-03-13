#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/bootstrap_release_local_artifacts.sh"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "expected to find '$needle' in $file" >&2
    cat "$file" >&2
    exit 1
  fi
}

assert_file_equals() {
  local file="$1"
  local expected="$2"
  local actual
  actual="$(cat "$file")"
  if [[ "$actual" != "$expected" ]]; then
    echo "expected $file to equal '$expected' but got '$actual'" >&2
    exit 1
  fi
}

make_fixture_repo() {
  local fixture
  fixture="$(mktemp -d)"
  mkdir -p \
    "$fixture/scripts" \
    "$fixture/docs/releases" \
    "$fixture/.github/ISSUE_TEMPLATE" \
    "$fixture/.github/workflows"

  cp "$REPO_ROOT/scripts/check-docs.sh" "$fixture/scripts/check-docs.sh"
  cp "$REPO_ROOT/scripts/release_artifact_lib.sh" "$fixture/scripts/release_artifact_lib.sh"
  chmod +x "$fixture/scripts/check-docs.sh"
  cp "$REPO_ROOT/docs/releases/README.md" "$fixture/docs/releases/README.md"
  cp "$REPO_ROOT/docs/releases/TEMPLATE.md" "$fixture/docs/releases/TEMPLATE.md"
  cp "$REPO_ROOT/docs/releases/v0.1.0.md" "$fixture/docs/releases/v0.1.0.md"
  cp "$REPO_ROOT/docs/releases/v0.1.1.md" "$fixture/docs/releases/v0.1.1.md"
  cp "$REPO_ROOT/docs/releases/v0.1.2.md" "$fixture/docs/releases/v0.1.2.md"
  cp "$REPO_ROOT/.github/ISSUE_TEMPLATE/config.yml" "$fixture/.github/ISSUE_TEMPLATE/config.yml"
  cp "$REPO_ROOT/.github/workflows/release.yml" "$fixture/.github/workflows/release.yml"

  cat >"$fixture/CHANGELOG.md" <<'EOF'
# Changelog

## [0.1.2] - 2026-03-09

## [0.1.1] - 2026-03-09

## [0.1.0] - 2026-03-09
EOF

  cat >"$fixture/AGENTS.md" <<'EOF'
# Mirror
EOF
  cp "$fixture/AGENTS.md" "$fixture/CLAUDE.md"

  printf '%s\n' "$fixture"
}

run_bootstrap_roundtrip_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_before="$fixture/strict-before.txt"
  local strict_after="$fixture/strict-after.txt"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_before" 2>&1
  ); then
    echo "expected strict doc check to fail before bootstrapping local release artifacts" >&2
    cat "$strict_before" >&2
    exit 1
  fi

  assert_contains "$strict_before" "missing local debug doc for v0.1.2"
  assert_contains "$strict_before" "missing trace index for released versions"
  assert_contains "$strict_before" "missing latest trace pointer"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  [[ -f "$fixture/.docs/releases/v0.1.0-debug.md" ]]
  [[ -f "$fixture/.docs/releases/v0.1.1-debug.md" ]]
  [[ -f "$fixture/.docs/releases/v0.1.2-debug.md" ]]
  [[ -f "$fixture/.docs/traces/index.jsonl" ]]
  [[ -f "$fixture/.docs/traces/latest" ]]
  [[ -f "$fixture/.docs/traces/by-tag/v0.1.0/latest" ]]
  [[ -f "$fixture/.docs/traces/by-tag/v0.1.1/latest" ]]
  [[ -f "$fixture/.docs/traces/by-tag/v0.1.2/latest" ]]
  [[ -f "$fixture/.docs/traces/20260309T045436Z-post-release-v0.1.0-78eec94e/metadata.json" ]]
  [[ -f "$fixture/.docs/traces/20260309T045437Z-post-release-v0.1.1-6cacc588/metadata.json" ]]
  [[ -f "$fixture/.docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67/metadata.json" ]]

  assert_contains "$fixture/.docs/releases/v0.1.2-debug.md" "Trace path:"
  assert_contains "$fixture/.docs/traces/index.jsonl" "\"tag\":\"v0.1.0\""
  assert_contains "$fixture/.docs/traces/index.jsonl" "\"tag\":\"v0.1.1\""
  assert_contains "$fixture/.docs/traces/index.jsonl" "\"tag\":\"v0.1.2\""
  assert_contains "$fixture/.docs/traces/index.jsonl" "\"command\":\"post-release\""
  assert_contains "$fixture/.docs/traces/index.jsonl" "\"status\":\"success\""

  assert_file_equals \
    "$fixture/.docs/traces/latest" \
    ".docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67"
  assert_file_equals \
    "$fixture/.docs/traces/by-tag/v0.1.0/latest" \
    ".docs/traces/20260309T045436Z-post-release-v0.1.0-78eec94e"
  assert_file_equals \
    "$fixture/.docs/traces/by-tag/v0.1.1/latest" \
    ".docs/traces/20260309T045437Z-post-release-v0.1.1-6cacc588"
  assert_file_equals \
    "$fixture/.docs/traces/by-tag/v0.1.2/latest" \
    ".docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67"

  (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_after" 2>&1
  )

  assert_contains "$strict_after" "All doc governance checks passed."
}

run_release_doc_linkage_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-linkage.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#- Trace directory: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace directory: `.docs/traces/WRONG-v0.1.2`#' "$release_doc"
  perl -0pi -e 's#- Local debug log: `\.docs/releases/v0\.1\.2-debug\.md`#- Local debug log: `.docs/releases/WRONG-v0.1.2-debug.md`#' "$release_doc"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent trace/detail linkage" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "trace detail link"
  assert_contains "$strict_output" "local debug log detail link"
}

run_trace_identity_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-id.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#- Trace ID: `020e2a67`#- Trace ID: `WRONG999`#' "$release_doc"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent trace identity" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "trace identity"
}

run_bootstrap_fails_on_inconsistent_trace_identity_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local output_file="$fixture/bootstrap-trace-id.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  perl -0pi -e 's#- Trace ID: `020e2a67`#- Trace ID: `WRONG999`#' "$release_doc"

  if LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1; then
    echo "expected bootstrap to fail on inconsistent trace identity" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "Trace path basename must end with"
}

run_bootstrap_fails_on_trace_path_traversal_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local output_file="$fixture/bootstrap-traversal.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  perl -0pi -e 's#- Trace path: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace path: `.docs/traces/../outside/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"

  if LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1; then
    echo "expected bootstrap to fail on trace path traversal" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "invalid Trace path"
}

run_strict_doc_check_fails_on_trace_path_traversal_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-traversal.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  perl -0pi -e 's#- Trace path: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace path: `.docs/traces/../outside/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"
  perl -0pi -e 's#- Trace directory: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace directory: `.docs/traces/../outside/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on trace path traversal" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "Trace path must stay under .docs/traces/"
}

run_bootstrap_fails_on_symlinked_trace_path_prefix_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local output_file="$fixture/bootstrap-symlink-prefix.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  mkdir -p "$fixture/.docs/traces" "$fixture/outside-target"
  ln -s "$fixture/outside-target" "$fixture/.docs/traces/out"

  perl -0pi -e 's#- Trace path: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace path: `.docs/traces/out/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"
  perl -0pi -e 's#- Trace directory: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace directory: `.docs/traces/out/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"

  if LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1; then
    echo "expected bootstrap to fail on symlink-backed trace path prefix" >&2
    cat "$output_file" >&2
    exit 1
  fi

  assert_contains "$output_file" "invalid Trace path"
}

run_strict_doc_check_fails_on_symlinked_trace_path_prefix_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-symlink-prefix.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  mkdir -p "$fixture/.docs/traces" "$fixture/outside-target"
  ln -s "$fixture/outside-target" "$fixture/.docs/traces/out"

  perl -0pi -e 's#- Trace path: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace path: `.docs/traces/out/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"
  perl -0pi -e 's#- Trace directory: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace directory: `.docs/traces/out/20260309T053941Z-post-release-v0.1.2-020e2a67`#' "$release_doc"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on symlink-backed trace path prefix" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "Trace path must stay under .docs/traces/"
}

run_trace_latest_pointer_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-latest.txt"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  printf '%s\n' ".docs/traces/WRONG-latest" >"$fixture/.docs/traces/latest"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent latest trace pointer" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "trace latest pointer"
}

run_trace_by_tag_pointer_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-by-tag.txt"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  printf '%s\n' ".docs/traces/WRONG-v0.1.2" >"$fixture/.docs/traces/by-tag/v0.1.2/latest"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent by-tag trace pointer" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "by-tag latest pointer"
}

run_trace_index_record_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-index.txt"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  cat >"$fixture/.docs/traces/index.jsonl" <<'EOF'
{"tag":"v0.1.2","trace_id":"020e2a67","trace_path":".docs/traces/WRONG-v0.1.2","command":"post-release","status":"success","source_release_doc":"docs/releases/v0.1.2.md"}
{"tag":"v0.1.1","trace_id":"6cacc588","trace_path":".docs/traces/20260309T045437Z-post-release-v0.1.1-6cacc588","command":"post-release","status":"success","source_release_doc":"docs/releases/v0.1.1.md"}
{"tag":"v0.1.0","trace_id":"78eec94e","trace_path":".docs/traces/20260309T045436Z-post-release-v0.1.0-78eec94e","command":"post-release","status":"success","source_release_doc":"docs/releases/v0.1.0.md"}
EOF

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent trace index record" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "trace index record"
}

run_trace_metadata_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-trace-metadata.txt"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#\"trace_id\":\"020e2a67\"#\"trace_id\":\"WRONG999\"#' "$fixture/.docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67/metadata.json"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent trace metadata" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "trace metadata"
}

run_debug_doc_trace_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-debug-doc.txt"

  LOONGCLAW_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#- Trace ID: `020e2a67`#- Trace ID: `WRONG999`#' "$fixture/.docs/releases/v0.1.2-debug.md"

  if (
    cd "$fixture" &&
      LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent debug doc trace linkage" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "debug doc trace"
}

run_bootstrap_roundtrip_test
run_release_doc_linkage_consistency_test
run_trace_identity_consistency_test
run_bootstrap_fails_on_inconsistent_trace_identity_test
run_bootstrap_fails_on_trace_path_traversal_test
run_strict_doc_check_fails_on_trace_path_traversal_test
run_bootstrap_fails_on_symlinked_trace_path_prefix_test
run_strict_doc_check_fails_on_symlinked_trace_path_prefix_test
run_trace_latest_pointer_consistency_test
run_trace_by_tag_pointer_consistency_test
run_trace_index_record_consistency_test
run_trace_metadata_consistency_test
run_debug_doc_trace_consistency_test

echo "bootstrap_release_local_artifacts.sh checks passed"
