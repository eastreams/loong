# Session Recover Running Delegate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend `session_recover` so operators can mark an overdue `running` async delegate child
as failed using the same audited sqlite session surface already used for queued recovery.

**Architecture:** Reuse `session_delegate_lifecycle_at(...)` and the repository's
`finalize_session_terminal_if_current(...)` guard. Generalize the recovery plan to describe the
expected source state and reference timestamp, add a distinct running-overdue recovery kind and
fallback error prefix, and keep the response payload machine-readable.

**Tech Stack:** Rust, rusqlite, serde_json, existing LoongClaw session repository and session tool
runtime

---

### Task 1: Pin the new recoverable shape with tests

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/session/mod.rs`

**Step 1: Write the failing test**

Add tests that:

- `session_recover` marks an overdue running async child failed
- `session_recover` rejects a fresh running child
- missing recovery events still infer `running_async_overdue_marked_failed` from `last_error`

**Step 2: Run test to verify it fails**

Run:

- `cargo test -p loongclaw-app session_recover_marks_overdue_running_async_child_failed -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app observe_missing_recovery_infers_running_async_overdue_from_last_error -- --nocapture --test-threads=1`

Expected: FAIL because running-overdue recovery is not implemented yet.

**Step 3: Write minimal implementation**

Implement only the code required to make those tests pass.

**Step 4: Run test to verify it passes**

Run the same focused commands and confirm both pass.

### Task 2: Generalize recovery planning and execution

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/session/recovery.rs`

**Step 1: Refactor the recovery plan**

Make `SessionRecoverPlan` encode:

- expected current state (`ready` or `running`)
- recovery kind
- reference (`queued` or `started`)
- reference timestamp and timeout metadata

**Step 2: Extend execution**

Update `execute_session_recover(...)` to:

- build the correct recovery error string per recovery kind
- conditionally finalize from the plan's expected state
- emit the matching structured recovery payload and `recovery_action`

**Step 3: Verify focused tests**

Run:

- `cargo test -p loongclaw-app session_recover_ -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app observe_missing_recovery_ -- --nocapture --test-threads=1`

Expected: PASS

### Task 3: Update product docs and run full verification

**Files:**
- Modify: `docs/product-specs/index.md`
- Modify: `docs/roadmap.md`

**Step 1: Update docs**

Document that:

- `session_recover` now covers overdue queued and overdue running async delegate children
- hard kill / retries / automatic post-restart recovery remain out of scope

**Step 2: Run focused regression**

Run:

- `cargo test -p loongclaw-app session_recover_ -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app session_wait_reports_missing_terminal_outcome_for_recovered_failed_session -- --nocapture --test-threads=1`

**Step 3: Run full verification**

Run:

- `cargo fmt --all`
- `cargo test --workspace --all-features -- --test-threads=1`
- `cargo fmt --all --check`
- `git diff --check`

Expected: PASS
