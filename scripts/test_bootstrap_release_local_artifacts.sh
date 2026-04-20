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

write_fixture_release_doc() {
  local path="$1"
  local tag="$2"
  local prerelease="$3"
  local trace_id="$4"
  local trace_path="$5"
  local workflow_run_id="$6"

  cat >"$path" <<EOF
# Release ${tag}

## Summary
- Generated at: 2026-03-09T05:39:43Z
- Release status: published (draft=false, prerelease=${prerelease})
- Target commitish: \`dev\`
- Artifact count: 1
- Trace ID: \`${trace_id}\`
- Trace path: \`${trace_path}\`

## Highlights
- Fixture release doc for ${tag}.

## Process
- Date: 2026-03-09T05:39:43Z
- Owner: fixture
- Scope summary: fixture release for ${tag}.
- Gates run: task verify
- Refactor budget item: fixture placeholder

## Artifacts
| Asset | Size (bytes) | SHA256 | Download |
|---|---:|---|---|
| \`loong-${tag}-x86_64-unknown-linux-gnu.tar.gz\` | 1 | \`deadbeef\` | [link](https://github.com/eastreams/loong/releases/download/${tag}/linux.tar.gz) |

## Verification
| Check | Result | Evidence |
|---|---|---|
| Release workflow completed successfully | PASS | [workflow run](https://github.com/eastreams/loong/actions/runs/${workflow_run_id}) |
| GitHub release is not draft | PASS | [release page](https://github.com/eastreams/loong/releases/tag/${tag}) |

## Refactor Budget
- Hotspot metric paid down: none
- Evidence: fixture
- If no paydown shipped, rationale: fixture

## Known Issues
- None observed during fixture verification.

## Rollback
- Re-run the release workflow and replace assets.

## Detail Links
- [Changelog entry](../../CHANGELOG.md)
- [Release workflow run](https://github.com/eastreams/loong/actions/runs/${workflow_run_id})
- [GitHub release page](https://github.com/eastreams/loong/releases/tag/${tag})
- [Release workflow definition](../../.github/workflows/release.yml)
- Trace directory: \`${trace_path}\`
- Local debug log: \`.docs/releases/${tag}-debug.md\`
EOF
}

make_fixture_repo() {
  local fixture
  fixture="$(mktemp -d)"
  mkdir -p \
    "$fixture/scripts" \
    "$fixture/docs/design-docs" \
    "$fixture/docs/releases" \
    "$fixture/docs/releases/support" \
    "$fixture/crates/contracts/src" \
    "$fixture/.github/ISSUE_TEMPLATE" \
    "$fixture/.github/workflows"

  cp "$REPO_ROOT/scripts/check-docs.sh" "$fixture/scripts/check-docs.sh"
  cp "$REPO_ROOT/scripts/check_governance_docs_consistency.sh" \
    "$fixture/scripts/check_governance_docs_consistency.sh"
  cp "$REPO_ROOT/scripts/release_artifact_lib.sh" "$fixture/scripts/release_artifact_lib.sh"
  chmod +x "$fixture/scripts/check-docs.sh"
  chmod +x "$fixture/scripts/check_governance_docs_consistency.sh"
  cp "$REPO_ROOT/docs/releases/README.md" "$fixture/docs/releases/README.md"
  cp "$REPO_ROOT/docs/releases/support/README.md" "$fixture/docs/releases/support/README.md"
  cp "$REPO_ROOT/docs/releases/support/TEMPLATE.md" "$fixture/docs/releases/support/TEMPLATE.md"
  cp "$REPO_ROOT/crates/contracts/src/audit_types.rs" \
    "$fixture/crates/contracts/src/audit_types.rs"
  cp "$REPO_ROOT/.github/ISSUE_TEMPLATE/config.yml" "$fixture/.github/ISSUE_TEMPLATE/config.yml"
  cp "$REPO_ROOT/.github/workflows/release.yml" "$fixture/.github/workflows/release.yml"

  write_fixture_release_doc \
    "$fixture/docs/releases/v0.1.0.md" \
    "v0.1.0" \
    "false" \
    "78eec94e" \
    ".docs/traces/20260309T045436Z-post-release-v0.1.0-78eec94e" \
    "10"
  write_fixture_release_doc \
    "$fixture/docs/releases/v0.1.1.md" \
    "v0.1.1" \
    "false" \
    "6cacc588" \
    ".docs/traces/20260309T045437Z-post-release-v0.1.1-6cacc588" \
    "11"
  write_fixture_release_doc \
    "$fixture/docs/releases/v0.1.2.md" \
    "v0.1.2" \
    "false" \
    "020e2a67" \
    ".docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67" \
    "12"

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

  cat >"$fixture/docs/ROADMAP.md" <<'EOF'
# Loong Roadmap

- kernel-level request-policy gate for tool calls through `PolicyEngine::authorize(...)`
  plus `PolicyExtensionChain`, with explicit deny/approval-required outcomes before
  tool dispatch (Rule of Two)
EOF

  cat >"$fixture/docs/SECURITY.md" <<'EOF'
# Security

CapabilityToken → PolicyEngine.authorize(...) → PolicyExtensionChain → Execution → Audit

- 10 event kinds with atomic sequencing
EOF

  cat >"$fixture/docs/QUALITY_SCORE.md" <<'EOF'
# Quality Score

| Domain | Grade | Last Reviewed | Gaps |
|--------|-------|---------------|------|
| Kernel Security (L1) | B+ | 2026-04-03 | Core tool paths now route through policy extensions; remaining gaps are non-uniform L1 coverage for connector/ACP/runtime-only analytics and explicit `Direct` compatibility lanes |
| Observability (L4) | C+ | 2026-04-03 | Durable JSONL/fanout audit exists; remaining gaps are tamper-evident verification, query baseline, and richer export lanes |
EOF

  cat >"$fixture/docs/design-docs/index.md" <<'EOF'
# Design Documents Index

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-003 | Append-only event log as single source of truth | Partial — audit events exist, JSONL/fanout retention exists, no tamper-evident verification or query baseline yet |
EOF

  cat >"$fixture/docs/design-docs/layered-kernel-design.md" <<'EOF'
# Layered Kernel Design

- Tool plane core/extension execution must route deterministic request-policy approval through the
  kernel authorization stack before dispatch: token/capability validation in `PolicyEngine::authorize`
  plus tool-specific tightening in `PolicyExtensionChain` (Rule of Two: model intent plus
  deterministic policy decision).
- The deprecated `PolicyEngine::check_tool_call` hook remains compatibility-only and must not be
  treated as the live request-policy seam.
EOF

  printf '%s\n' "$fixture"
}

make_prerelease_fixture_repo() {
  local fixture
  fixture="$(mktemp -d)"
  mkdir -p \
    "$fixture/scripts" \
    "$fixture/docs/design-docs" \
    "$fixture/docs/releases" \
    "$fixture/docs/releases/support" \
    "$fixture/crates/contracts/src" \
    "$fixture/.github/ISSUE_TEMPLATE" \
    "$fixture/.github/workflows"

  cp "$REPO_ROOT/scripts/check-docs.sh" "$fixture/scripts/check-docs.sh"
  cp "$REPO_ROOT/scripts/check_governance_docs_consistency.sh" \
    "$fixture/scripts/check_governance_docs_consistency.sh"
  cp "$REPO_ROOT/scripts/release_artifact_lib.sh" "$fixture/scripts/release_artifact_lib.sh"
  chmod +x "$fixture/scripts/check-docs.sh"
  chmod +x "$fixture/scripts/check_governance_docs_consistency.sh"
  cp "$REPO_ROOT/docs/releases/README.md" "$fixture/docs/releases/README.md"
  cp "$REPO_ROOT/docs/releases/support/README.md" "$fixture/docs/releases/support/README.md"
  cp "$REPO_ROOT/docs/releases/support/TEMPLATE.md" "$fixture/docs/releases/support/TEMPLATE.md"
  cp "$REPO_ROOT/crates/contracts/src/audit_types.rs" \
    "$fixture/crates/contracts/src/audit_types.rs"
  cp "$REPO_ROOT/.github/ISSUE_TEMPLATE/config.yml" "$fixture/.github/ISSUE_TEMPLATE/config.yml"
  cp "$REPO_ROOT/.github/workflows/release.yml" "$fixture/.github/workflows/release.yml"

  cat >"$fixture/CHANGELOG.md" <<'EOF'
# Changelog

## [0.1.0-alpha.1] - 2026-03-17
EOF

  cat >"$fixture/docs/releases/v0.1.0-alpha.1.md" <<'EOF'
# Release v0.1.0-alpha.1

## Summary
- Generated at: 2026-03-17T00:00:00Z
- Release status: published (draft=false, prerelease=true)
- Target commitish: `dev`
- Artifact count: 4
- Trace ID: `prealpha01`
- Trace path: `.docs/traces/20260317T000000Z-post-release-v0.1.0-alpha.1-prealpha01`

## Highlights
- README-based prerelease summary.

## Process
- Date: 2026-03-17
- Owner: fixture
- Scope summary: prerelease reset fixture
- Gates run: task verify
- Refactor budget item: fixture placeholder

## Artifacts
| Asset | Size (bytes) | SHA256 | Download |
|---|---:|---|---|
| `loong-v0.1.0-alpha.1-x86_64-unknown-linux-gnu.tar.gz` | 1 | `deadbeef` | [link](https://github.com/eastreams/loong/releases/download/v0.1.0-alpha.1/linux.tar.gz) |

## Verification
| Check | Result | Evidence |
|---|---|---|
| Release workflow completed successfully | PASS | [workflow run](https://github.com/eastreams/loong/actions/runs/1) |
| GitHub release is not draft | PASS | [release page](https://github.com/eastreams/loong/releases/tag/v0.1.0-alpha.1) |

## Refactor Budget
- Hotspot metric paid down: none
- Evidence: fixture
- If no paydown shipped, rationale: fixture

## Known Issues
- None observed during fixture verification.

## Rollback
- Delete the prerelease tag and regenerate.

## Detail Links
- [Changelog entry](../../CHANGELOG.md)
- [Release workflow run](https://github.com/eastreams/loong/actions/runs/1)
- [GitHub release page](https://github.com/eastreams/loong/releases/tag/v0.1.0-alpha.1)
- Trace directory: `.docs/traces/20260317T000000Z-post-release-v0.1.0-alpha.1-prealpha01`
- Local debug log: `.docs/releases/v0.1.0-alpha.1-debug.md`
EOF

  cat >"$fixture/AGENTS.md" <<'EOF'
# Mirror
EOF
  cp "$fixture/AGENTS.md" "$fixture/CLAUDE.md"

  cat >"$fixture/docs/ROADMAP.md" <<'EOF'
# Loong Roadmap

- kernel-level request-policy gate for tool calls through `PolicyEngine::authorize(...)`
  plus `PolicyExtensionChain`, with explicit deny/approval-required outcomes before
  tool dispatch (Rule of Two)
EOF

  cat >"$fixture/docs/SECURITY.md" <<'EOF'
# Security

CapabilityToken → PolicyEngine.authorize(...) → PolicyExtensionChain → Execution → Audit

- 10 event kinds with atomic sequencing
EOF

  cat >"$fixture/docs/QUALITY_SCORE.md" <<'EOF'
# Quality Score

| Domain | Grade | Last Reviewed | Gaps |
|--------|-------|---------------|------|
| Kernel Security (L1) | B+ | 2026-04-03 | Core tool paths now route through policy extensions; remaining gaps are non-uniform L1 coverage for connector/ACP/runtime-only analytics and explicit `Direct` compatibility lanes |
| Observability (L4) | C+ | 2026-04-03 | Durable JSONL/fanout audit exists; remaining gaps are tamper-evident verification, query baseline, and richer export lanes |
EOF

  cat >"$fixture/docs/design-docs/index.md" <<'EOF'
# Design Documents Index

| ID | Decision | Implementation Status |
|----|----------|---------------------|
| D-003 | Append-only event log as single source of truth | Partial — audit events exist, JSONL/fanout retention exists, no tamper-evident verification or query baseline yet |
EOF

  cat >"$fixture/docs/design-docs/layered-kernel-design.md" <<'EOF'
# Layered Kernel Design

- Tool plane core/extension execution must route deterministic request-policy approval through the
  kernel authorization stack before dispatch: token/capability validation in `PolicyEngine::authorize`
  plus tool-specific tightening in `PolicyExtensionChain` (Rule of Two: model intent plus
  deterministic policy decision).
- The deprecated `PolicyEngine::check_tool_call` hook remains compatibility-only and must not be
  treated as the live request-policy seam.
EOF

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
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_before" 2>&1
  ); then
    echo "expected strict doc check to fail before bootstrapping local release artifacts" >&2
    cat "$strict_before" >&2
    exit 1
  fi

  assert_contains "$strict_before" "missing local debug doc for v0.1.2"
  assert_contains "$strict_before" "missing trace index for released versions"
  assert_contains "$strict_before" "missing latest trace pointer"

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

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
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_after" 2>&1
  )

  assert_contains "$strict_after" "All doc governance checks passed."
}

run_prerelease_bootstrap_roundtrip_test() {
  local fixture
  fixture="$(make_prerelease_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  [[ -f "$fixture/.docs/releases/v0.1.0-alpha.1-debug.md" ]]
  [[ -f "$fixture/.docs/traces/latest" ]]
  [[ -f "$fixture/.docs/traces/by-tag/v0.1.0-alpha.1/latest" ]]
  [[ -f "$fixture/.docs/traces/20260317T000000Z-post-release-v0.1.0-alpha.1-prealpha01/metadata.json" ]]

  assert_contains "$fixture/.docs/releases/v0.1.0-alpha.1-debug.md" "Trace path:"
  assert_contains "$fixture/.docs/traces/index.jsonl" "\"tag\":\"v0.1.0-alpha.1\""
  assert_file_equals \
    "$fixture/.docs/traces/latest" \
    ".docs/traces/20260317T000000Z-post-release-v0.1.0-alpha.1-prealpha01"
  assert_file_equals \
    "$fixture/.docs/traces/by-tag/v0.1.0-alpha.1/latest" \
    ".docs/traces/20260317T000000Z-post-release-v0.1.0-alpha.1-prealpha01"

  (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >/dev/null
  )
}

run_release_doc_highlights_required_test() {
  local fixture
  fixture="$(make_prerelease_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-highlights.txt"
  local release_doc="$fixture/docs/releases/v0.1.0-alpha.1.md"

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's/^## Highlights\n.*?\n## Process\n/## Process\n/ms' "$release_doc"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail when highlights are missing" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "missing section '## Highlights'"
}

run_release_doc_linkage_consistency_test() {
  local fixture
  fixture="$(make_fixture_repo)"
  trap 'rm -rf "$fixture"' RETURN

  local strict_output="$fixture/strict-linkage.txt"
  local release_doc="$fixture/docs/releases/v0.1.2.md"

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#- Trace directory: `\.docs/traces/20260309T053941Z-post-release-v0\.1\.2-020e2a67`#- Trace directory: `.docs/traces/WRONG-v0.1.2`#' "$release_doc"
  perl -0pi -e 's#- Local debug log: `\.docs/releases/v0\.1\.2-debug\.md`#- Local debug log: `.docs/releases/WRONG-v0.1.2-debug.md`#' "$release_doc"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#- Trace ID: `020e2a67`#- Trace ID: `WRONG999`#' "$release_doc"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  if LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1; then
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

  if LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1; then
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
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  if LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST" >"$output_file" 2>&1; then
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
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  printf '%s\n' ".docs/traces/WRONG-latest" >"$fixture/.docs/traces/latest"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  printf '%s\n' ".docs/traces/WRONG-v0.1.2" >"$fixture/.docs/traces/by-tag/v0.1.2/latest"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  cat >"$fixture/.docs/traces/index.jsonl" <<'EOF'
{"tag":"v0.1.2","trace_id":"020e2a67","trace_path":".docs/traces/WRONG-v0.1.2","command":"post-release","status":"success","source_release_doc":"docs/releases/v0.1.2.md"}
{"tag":"v0.1.1","trace_id":"6cacc588","trace_path":".docs/traces/20260309T045437Z-post-release-v0.1.1-6cacc588","command":"post-release","status":"success","source_release_doc":"docs/releases/v0.1.1.md"}
{"tag":"v0.1.0","trace_id":"78eec94e","trace_path":".docs/traces/20260309T045436Z-post-release-v0.1.0-78eec94e","command":"post-release","status":"success","source_release_doc":"docs/releases/v0.1.0.md"}
EOF

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#\"trace_id\":\"020e2a67\"#\"trace_id\":\"WRONG999\"#' "$fixture/.docs/traces/20260309T053941Z-post-release-v0.1.2-020e2a67/metadata.json"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
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

  LOONG_RELEASE_ARTIFACTS_REPO_ROOT="$fixture" "$SCRIPT_UNDER_TEST"

  perl -0pi -e 's#- Trace ID: `020e2a67`#- Trace ID: `WRONG999`#' "$fixture/.docs/releases/v0.1.2-debug.md"

  if (
    cd "$fixture" &&
      LOONG_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh >"$strict_output" 2>&1
  ); then
    echo "expected strict doc check to fail on inconsistent debug doc trace linkage" >&2
    cat "$strict_output" >&2
    exit 1
  fi

  assert_contains "$strict_output" "debug doc trace"
}

run_bootstrap_roundtrip_test
run_prerelease_bootstrap_roundtrip_test
run_release_doc_highlights_required_test
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
