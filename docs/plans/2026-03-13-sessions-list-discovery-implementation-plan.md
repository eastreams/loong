# Sessions List Discovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `sessions_list` a real operator discovery surface by adding machine-readable filters
and reliable overdue delegate detection without adding new tools.

**Architecture:** Keep the existing `sessions_list` tool, but add request filters and response
metadata at the app layer. Strengthen delegate lifecycle reconstruction with a repository helper
that reads lifecycle anchor events directly, then reuse that path in `sessions_list`,
`session_status`, and internal cancel/recover planning so lifecycle-derived discovery remains
correct even when recent event windows are noisy.

**Tech Stack:** Rust, rusqlite, serde_json, existing LoongClaw app-layer session tools and sqlite
session repository

---

### Task 1: Write failing tests for discovery filters and stable lifecycle anchors

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing test**

Add tests that:

- `sessions_list` filters visible sessions by `state`, `kind`, and `parent_session_id`
- `sessions_list` returns overdue delegate children when `overdue_only = true`
- `session_status` still reconstructs delegate lifecycle when recent events are noisy
- provider schema for `sessions_list` advertises the new filter parameters

**Step 2: Run test to verify it fails**

Run:

- `cargo test -p loongclaw-app sessions_list_filters_visible_sessions_by_state_kind_and_parent -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app sessions_list_overdue_only_uses_lifecycle_anchor_events -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app session_status_uses_delegate_lifecycle_anchor_events_when_recent_window_is_noisy -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app provider_tool_definitions_are_stable_and_complete -- --nocapture --test-threads=1`

Expected: FAIL because `sessions_list` has no filters and lifecycle still depends on recent events.

**Step 3: Write minimal implementation**

Implement only the code needed to satisfy the tests.

**Step 4: Run test to verify it passes**

Run the same commands and confirm they pass.

### Task 2: Add repository lifecycle-anchor reads and wire shared lifecycle usage

**Files:**
- Modify: `crates/app/src/session/repository.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Add lifecycle-anchor repository helper**

Introduce a helper that returns lifecycle-relevant events in ascending order for one session:

- `delegate_queued`
- `delegate_started`
- `delegate_cancel_requested`

**Step 2: Reuse the helper in session inspection**

Update session inspection snapshots and lifecycle planning so:

- `session_status`
- `session_wait`
- `session_cancel`
- `session_recover`

all reconstruct delegate lifecycle from the stable lifecycle-anchor events instead of arbitrary
recent-event windows.

**Step 3: Extend `sessions_list`**

Add request parsing, filter application, response metadata, and optional `delegate_lifecycle`
payloads.

**Step 4: Verify focused tests**

Run:

- `cargo test -p loongclaw-app sessions_list_ -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app session_status_uses_delegate_lifecycle_anchor_events_when_recent_window_is_noisy -- --nocapture --test-threads=1`

Expected: PASS

### Task 3: Update docs and run full verification

**Files:**
- Modify: `docs/product-specs/index.md`
- Modify: `docs/roadmap.md`

**Step 1: Update docs**

Document that:

- `sessions_list` supports filtered discovery of visible sessions
- overdue delegate discovery is available through the existing tool surface
- lifecycle-derived discovery uses the stable lifecycle read path rather than recent-event windows

**Step 2: Run focused regression**

Run:

- `cargo test -p loongclaw-app sessions_list_ -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app session_status_ -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app session_recover_ -- --nocapture --test-threads=1`

**Step 3: Run full verification**

Run:

- `cargo fmt --all`
- `cargo test --workspace --all-features -- --test-threads=1`
- `cargo fmt --all --check`
- `git diff --check`

Expected: PASS
