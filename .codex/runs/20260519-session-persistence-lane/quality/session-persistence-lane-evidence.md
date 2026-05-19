## Session Persistence And Session Tool Surfaces Lane Evidence

### Scope

- Worktree: `/Users/chum/worktrees/loong/release-hardening-session-persistence-20260519`
- Primary targets:
  - `crates/app/src/session/repository.rs`
  - `crates/app/src/tools/session.rs`
- Lane-local child modules introduced:
  - `crates/app/src/session/repository/persistence.rs`
  - `crates/app/src/session/repository/projections.rs`
  - `crates/app/src/session/repository/records.rs`
  - `crates/app/src/session/repository/tree.rs`
  - `crates/app/src/session/repository/repository_tests.rs`
  - `crates/app/src/tools/session/mutations.rs`
  - `crates/app/src/tools/session/projections.rs`
  - `crates/app/src/tools/session/session_tool_tests.rs`

### Architectural Claim

- `session/repository` now separates:
  - persisted fact storage and write/update flows in `persistence.rs`
  - summary/search/observation/read-snapshot projections in `projections.rs`
  - session record/raw-row/codecs/shared normalization helpers in `records.rs`
  - session tree/head/artifact retention and branch-structure ownership in `tree.rs`
- `tools/session` now separates:
  - operator mutation flows in `mutations.rs`
  - session/task projection payloads, status rendering, batch/wait payloads, and workflow/session inspection shaping in `projections.rs`
  - lane-local tests in `session_tool_tests.rs`
- The fact-truth-first model and artifact retention behavior stay in repository-owned code paths; the tool layer remains an operator/projection surface over those persisted facts.

### Before And After Line Counts

- Before:
  - `crates/app/src/tools/session.rs`: `12487`
  - `crates/app/src/session/repository.rs`: `7698`
- After:
  - `crates/app/src/tools/session.rs`: `2607`
  - `crates/app/src/tools/session/mutations.rs`: `1561`
  - `crates/app/src/tools/session/projections.rs`: `2466`
  - `crates/app/src/session/repository.rs`: `53`
  - `crates/app/src/session/repository/persistence.rs`: `1549`
  - `crates/app/src/session/repository/projections.rs`: `1245`
  - `crates/app/src/session/repository/records.rs`: `1180`
  - `crates/app/src/session/repository/tree.rs`: `736`
- Line-cap result:
  - every non-test production Rust file in this lane is below `3000`
  - extracted test files use `*_tests.rs` names so they remain outside the release line-cap gate

### Focused Verification

- `./scripts/cargo-local-toolchain.sh check -p loong-app --all-features`
  - result: pass
- `./scripts/cargo-local-toolchain.sh test -p loong-app tools::session::session_tool_tests::session_create_checkpoint_creates_artifact_and_checkpoint_head -- --exact`
  - result: pass
- `./scripts/cargo-local-toolchain.sh test -p loong-app tools::session::session_tool_tests::session_create_branch_summary_captures_head_exclusive_range -- --exact`
  - result: pass
- `./scripts/cargo-local-toolchain.sh test -p loong-app tools::session::session_tool_tests::session_status_includes_workflow_metadata_for_delegate_child -- --exact`
  - result: pass
- `./scripts/cargo-local-toolchain.sh test -p loong-app session::repository::repository_tests::create_session_artifact_enforces_overlay_retention_to_latest_records -- --exact`
  - result: pass
- `./scripts/cargo-local-toolchain.sh test -p loong-app session::repository::repository_tests::replace_turns_preserves_named_heads_and_artifacts_across_rewrites -- --exact`
  - result: pass

### Residual Persistence Ownership Notes

- `records.rs` still carries a broad shared type/codec surface because the repository APIs and tests depend on common raw-row decoding and normalization helpers.
- `persistence.rs` still contains multiple fact-write surfaces beyond the artifact/session lane subset, including approval and control-plane persistence. That is acceptable for Pass 1 because the lane goal was to separate persistence from projections/operator surfaces, not to redesign adjacent product areas.
- `tools/session.rs` still keeps the main dispatcher, shared request structs, wait/runtime entrypoints, and parsing helpers. The deeper extraction removed the largest operator/projection masses without widening scope into chat, channels, or runtime host work.

### Constraint Check

- No work was done outside the session persistence/session tool lane.
- No broad chat shell, channel system, runtime host, or memory product redesign changes were introduced.
- Artifact retention behavior remains repository-owned and covered by focused tests.
- Session/task status and workflow projection shaping remain covered by focused tests.

### Commit Shape Summary

- Intended clean sequence:
  - repository lane split and shared-internal visibility adjustment
  - session tool split and shared-internal visibility adjustment
  - focused verification/evidence commit
- Current worktree state is uncommitted at evidence capture time.
