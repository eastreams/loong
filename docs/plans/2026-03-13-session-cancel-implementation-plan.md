# Session Cancel Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a first cancellation control surface for async delegate child sessions, with immediate queued cancellation and cooperative running cancellation at safe turn-loop checkpoints.

**Architecture:** Reuse the sqlite session/event model instead of introducing worker registry or hard process control. Add a root-visible `session_cancel` tool, represent cancellation through `delegate_cancel_requested` and `delegate_cancelled` events, and teach delegated child execution to stop at round boundaries when a cancel request is present.

**Tech Stack:** Rust, Tokio, rusqlite, serde_json, existing LoongClaw session repository / conversation turn-loop / app-tool runtime

---

### Task 1: Register the new tool surface

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/provider/mod.rs`
- Test: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/provider/mod.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add tests that:

- root runtime tool view includes `session_cancel`
- delegated child runtime tool view excludes `session_cancel`
- default dispatcher rejects hidden `session_cancel` from a child session

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app session_cancel -- --nocapture --test-threads=1`

Expected: FAIL because the tool is not registered or dispatched yet.

**Step 3: Write minimal implementation**

Add catalog/provider/dispatcher registration for `session_cancel` as a root-visible session tool.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app session_cancel -- --nocapture --test-threads=1`

Expected: PASS for tool registration / visibility cases.

### Task 2: Implement queued and running cancel requests

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/session/repository.rs`
- Test: `crates/app/src/tools/session.rs`
- Test: `crates/app/src/session/repository.rs`

**Step 1: Write the failing test**

Add tests that:

- `session_cancel` immediately terminalizes a queued async child as failed with `delegate_cancelled`
- `session_cancel` records a running-child `delegate_cancel_requested` event without terminalizing immediately
- unsupported targets are rejected cleanly

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app session_cancel -- --nocapture --test-threads=1`

Expected: FAIL because the tool behavior does not exist yet.

**Step 3: Write minimal implementation**

Implement:

- `execute_session_cancel(...)`
- lifecycle validation helper
- conditional queued finalize path
- running request path using an atomic state-preserving event write
- response payload with `cancel_action`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app session_cancel -- --nocapture --test-threads=1`

Expected: PASS

### Task 3: Teach delegated child execution to stop cooperatively

**Files:**
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/tools/session.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add tests that:

- a delegated child with a pending cancel request stops before the next provider round
- terminal state becomes failed with `delegate_cancelled`
- terminal outcome carries `delegate_cancelled: operator_requested`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app delegate_child_background_cancel -- --nocapture --test-threads=1`

Expected: FAIL because delegated child execution currently ignores cancel request events.

**Step 3: Write minimal implementation**

Implement a delegated-child cancellation checkpoint before each turn-loop round and finalize cancellation through the existing terminal persistence path.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app delegate_child_background_cancel -- --nocapture --test-threads=1`

Expected: PASS

### Task 4: Surface cancellation in session inspection

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Test: `crates/app/src/tools/session.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add tests that:

- `session_status` surfaces pending cancellation metadata for a running child
- `session_wait` returns that same metadata before terminal completion when cancellation is pending

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app session_cancel_requested -- --nocapture --test-threads=1`

Expected: FAIL because `delegate_lifecycle` does not currently encode cancellation state.

**Step 3: Write minimal implementation**

Extend delegate lifecycle inspection with optional cancellation metadata derived from the latest relevant cancel-request event.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app session_cancel_requested -- --nocapture --test-threads=1`

Expected: PASS

### Task 5: Update docs and run full verification

**Files:**
- Modify: `docs/product-specs/index.md`
- Modify: `docs/roadmap.md`

**Step 1: Update docs**

Document:

- `session_cancel`
- queued immediate cancellation
- running cooperative cancellation
- continued absence of hard process kill / retry / restart recovery

**Step 2: Run focused regression**

Run:

- `cargo test -p loongclaw-app session_cancel -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app delegate_child_background_cancel -- --nocapture --test-threads=1`
- `cargo test -p loongclaw-app session_ -- --nocapture --test-threads=1`

Expected: PASS

**Step 3: Run full verification**

Run:

- `cargo fmt --all`
- `cargo test --workspace --all-features -- --test-threads=1`
- `cargo fmt --all --check`
- `git diff --check`

Expected: PASS
