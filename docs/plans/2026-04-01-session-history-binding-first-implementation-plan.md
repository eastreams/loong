# Session History Binding-First Discovery Summary Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the main public discovery-first session-history helper binding-first while preserving a clearly named optional-kernel compatibility shim.

**Architecture:** Reuse the existing binding-aware discovery-first implementation as the single worker path. Promote the canonical public helper name to `ConversationRuntimeBinding<'_>` and move `Option<&KernelContext>` normalization behind an explicitly named compatibility wrapper.

**Tech Stack:** Rust, Tokio tests, cargo test, cargo fmt, cargo clippy

---

### Task 1: Rewrite the public contract in tests first

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Change the discovery-first tests so they express the new contract:

1. `load_discovery_first_event_summary_accepts_explicit_runtime_binding` should
   call the public `load_discovery_first_event_summary(...)` name with
   `ConversationRuntimeBinding::direct()` and
   `ConversationRuntimeBinding::kernel(&kernel_ctx)`
2. the compatibility test should call a new explicit shim named
   `load_discovery_first_event_summary_with_kernel_context(...)`

Example assertion shape:

```rust
let direct_summary = load_discovery_first_event_summary(
    "session",
    16,
    ConversationRuntimeBinding::direct(),
    &mem_config,
)
.await?;
```

**Step 2: Run test to verify it fails**

Run:
- `cargo test -p loongclaw-app conversation::tests::load_discovery_first_event_summary_accepts_explicit_runtime_binding -- --exact --nocapture`
- `cargo test -p loongclaw-app conversation::tests::load_discovery_first_event_summary_preserves_public_kernel_context_signature -- --exact --nocapture`

Expected: FAIL because the public function still accepts `Option<&KernelContext>`
and the explicit compatibility shim does not exist yet.

**Step 3: Commit**

Do not commit yet.

### Task 2: Promote the public helper to binding-first

**Files:**
- Modify: `crates/app/src/conversation/session_history.rs`
- Modify: `crates/app/src/conversation/mod.rs`

**Step 1: Write minimal implementation**

1. Change the main public `load_discovery_first_event_summary(...)` signature to
   accept `ConversationRuntimeBinding<'_>`
2. Reuse the existing binding-aware implementation path rather than duplicating
   summary logic
3. Introduce `load_discovery_first_event_summary_with_kernel_context(...)` as
   the explicit compatibility shim
4. Export the compatibility shim from `conversation/mod.rs` only if the public
   surface should retain that migration helper

**Step 2: Run targeted tests to verify they pass**

Run:
- `cargo test -p loongclaw-app conversation::tests::load_discovery_first_event_summary_accepts_explicit_runtime_binding -- --exact --nocapture`
- `cargo test -p loongclaw-app conversation::tests::load_discovery_first_event_summary_preserves_public_kernel_context_signature -- --exact --nocapture`

Expected: PASS

**Step 3: Commit**

Do not commit yet.

### Task 3: Verify neighboring history helpers remain unchanged

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Re-run neighboring history summary tests**

Run:
- `cargo test -p loongclaw-app load_fast_lane_tool_batch_event_summary_accepts_explicit_runtime_binding -- --exact --nocapture`
- `cargo test -p loongclaw-app load_turn_checkpoint_event_summary_reads_recovery_state_from_sqlite_history -- --exact --nocapture`

Expected: PASS

**Step 2: Tighten tests only if behavior drift is found**

If any regression appears, add the smallest test needed to pin the intended
binding behavior without expanding the slice beyond discovery-first.

**Step 3: Commit**

Do not commit yet.

### Task 4: Full verification and commit

**Files:**
- Modify: `docs/plans/2026-04-01-session-history-binding-first-design.md`
- Modify: `docs/plans/2026-04-01-session-history-binding-first-implementation-plan.md`

**Step 1: Run formatting verification**

Run: `cargo fmt --all -- --check`
Expected: PASS

**Step 2: Run strict lint verification**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 3: Run workspace tests**

Run:
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Expected: PASS

**Step 4: Inspect final diff**

Run:
- `git status --short`
- `git diff --stat`

Expected: only the session-history binding-first slice and its docs are present.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/session_history.rs \
        crates/app/src/conversation/mod.rs \
        crates/app/src/conversation/tests.rs \
        docs/plans/2026-04-01-session-history-binding-first-design.md \
        docs/plans/2026-04-01-session-history-binding-first-implementation-plan.md
git commit -m "refactor: make discovery history binding-first"
```
