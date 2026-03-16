# Conversation Binding Normalization Implementation Plan

**Goal:** Complete conversation-layer runtime binding normalization so the remaining conversation entrypoints and history helpers use `ConversationRuntimeBinding` instead of raw optional kernel context.

**Architecture:** Keep the scope inside the conversation module. Move the remaining `turn_loop`, `turn_coordinator`, `session_history`, and shared followup-helper seams onto `ConversationRuntimeBinding`, and only convert back to `Option<&KernelContext>` at lower-level leaf helpers that have not been normalized yet.

**Tech Stack:** Rust, async-trait, Tokio tests, GitHub issue-first workflow

---

### Task 1: Normalize session history helpers onto `ConversationRuntimeBinding`

**Files:**
- Modify: `crates/app/src/conversation/session_history.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add or adapt tests so history-summary helpers are called with
`ConversationRuntimeBinding::kernel(...)` and `ConversationRuntimeBinding::direct()`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app load_turn_checkpoint_event_summary -- --test-threads=1`

Expected: FAIL because the helper signatures still accept `Option<&KernelContext>`.

**Step 3: Write minimal implementation**

Update the session-history APIs and internal helpers to accept
`ConversationRuntimeBinding` and convert to optional kernel context only at the
memory-window leaf.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app load_turn_checkpoint_event_summary -- --test-threads=1`

Expected: PASS

### Task 2: Normalize remaining `ConversationTurnLoop` seams

**Files:**
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/turn_shared.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Adapt turn-loop entrypoint tests to call the explicit binding-based APIs or to
exercise the normalized internal helpers through binding-based paths.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime -- --test-threads=1`

Expected: FAIL due to remaining optional-kernel conversation seams.

**Step 3: Write minimal implementation**

Move the remaining turn-loop seams to `ConversationRuntimeBinding` while
preserving the public direct fallback behavior.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime -- --test-threads=1`

Expected: PASS

### Task 3: Normalize remaining `ConversationTurnCoordinator` seams

**Files:**
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Adapt coordinator and checkpoint-diagnostics tests to use binding-based entry
and helper calls where those seams are still raw optional kernel context.

**Step 2: Run test to verify it fails**

Run:
- `cargo test -p loongclaw-app repair_turn_checkpoint -- --test-threads=1`
- `cargo test -p loongclaw-app load_turn_checkpoint_diagnostics -- --test-threads=1`
- `cargo test -p loongclaw-app probe_turn_checkpoint_tail_runtime_gate -- --test-threads=1`

Expected: FAIL because coordinator helper signatures still expose optional
kernel context.

**Step 3: Write minimal implementation**

Normalize the remaining coordinator entrypoints and helper paths to
`ConversationRuntimeBinding`, converting back to optional kernel context only at
provider/history leaves that have not been normalized yet.

**Step 4: Run test to verify it passes**

Run:
- `cargo test -p loongclaw-app repair_turn_checkpoint -- --test-threads=1`
- `cargo test -p loongclaw-app load_turn_checkpoint_diagnostics -- --test-threads=1`
- `cargo test -p loongclaw-app probe_turn_checkpoint_tail_runtime_gate -- --test-threads=1`

Expected: PASS

### Task 4: Update docs and finish verification

**Files:**
- Modify: `docs/SECURITY.md`
- Modify: `docs/plans/2026-03-15-conversation-binding-normalization-design.md`
- Modify: `docs/plans/2026-03-15-conversation-binding-normalization-implementation-plan.md`

**Step 1: Update docs**

Clarify that the conversation module is now fully normalized around the explicit
runtime binding, while provider/ACP leaf helpers remain future follow-up work.

**Step 2: Run targeted verification**

Run:
- `cargo test -p loongclaw-app load_turn_checkpoint_event_summary -- --test-threads=1`
- `cargo test -p loongclaw-app handle_turn_with_runtime -- --test-threads=1`
- `cargo test -p loongclaw-app repair_turn_checkpoint -- --test-threads=1`
- `cargo test -p loongclaw-app load_turn_checkpoint_diagnostics -- --test-threads=1`
- `cargo test -p loongclaw-app probe_turn_checkpoint_tail_runtime_gate -- --test-threads=1`

Expected: PASS

**Step 3: Run full verification**

Run:
- `cargo fmt --all`
- `cargo fmt --all -- --check`
- `cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings`
- `cargo test -p loongclaw-app -- --test-threads=1`

Expected: PASS

**Step 4: Commit**

```bash
git add docs/plans/2026-03-15-conversation-binding-normalization-design.md \
        docs/plans/2026-03-15-conversation-binding-normalization-implementation-plan.md \
        docs/SECURITY.md \
        crates/app/src/conversation/session_history.rs \
        crates/app/src/conversation/turn_loop.rs \
        crates/app/src/conversation/turn_coordinator.rs \
        crates/app/src/conversation/tests.rs
git commit -m "refactor: normalize remaining conversation runtime binding seams"
```
