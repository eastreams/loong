# Conversation Runtime Binding Implementation Plan

**Goal:** Replace raw optional kernel context propagation at conversation/runtime boundaries with an explicit conversation runtime binding type while preserving current behavior.

**Architecture:** Add a conversation-scoped binding enum that explicitly represents kernel-bound versus direct-fallback execution. Thread that type through the conversation runtime, context assembly, persistence helpers, turn loop, turn coordinator, and app-tool dispatcher boundaries, while keeping lower-level provider helpers behaviorally unchanged by converting the binding to an optional kernel reference only at the leaf.

**Tech Stack:** Rust, async-trait, Tokio test framework, GitHub issue-first workflow

---

### Task 1: Add the binding type and wire it into public conversation exports

**Files:**
- Create: `crates/app/src/conversation/runtime_binding.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add tests that construct the binding type and assert:
- `direct()` is not kernel-bound
- `kernel(&ctx)` is kernel-bound
- `kernel_context()` returns the expected optional reference

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app conversation_runtime_binding -- --test-threads=1`

Expected: FAIL because the binding type and tests do not exist yet.

**Step 3: Write minimal implementation**

Create the binding type, expose the helper methods, and re-export it from
`conversation/mod.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app conversation_runtime_binding -- --test-threads=1`

Expected: PASS

### Task 2: Move conversation runtime/context/persistence surfaces to the binding type

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/context_engine.rs`
- Modify: `crates/app/src/conversation/persistence.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add regression tests that use the explicit binding type for:
- direct-fallback context assembly
- kernel-bound persistence / memory-window behavior where already covered

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app 'default_runtime_|kernel_' -- --test-threads=1`

Expected: FAIL due to signature mismatches or missing binding-based call paths.

**Step 3: Write minimal implementation**

Update runtime/context/persistence APIs to accept the binding type and convert
to `Option<&KernelContext>` only at lower-level leaf helpers where needed.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app 'default_runtime_|kernel_' -- --test-threads=1`

Expected: PASS

### Task 3: Move turn-engine and orchestration boundaries to the binding type

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Add a turn-engine or dispatcher-focused regression test that proves:
- app tools receive explicit direct binding without kernel
- core tools still deny when the binding is direct

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app 'turn_engine_|handle_turn_with_runtime_' -- --test-threads=1`

Expected: FAIL because the new binding type is not threaded through execution yet.

**Step 3: Write minimal implementation**

Update `AppToolDispatcher`, `TurnEngine`, turn loop, and turn coordinator to use
the explicit binding type instead of raw optional kernel references.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app 'turn_engine_|handle_turn_with_runtime_' -- --test-threads=1`

Expected: PASS

### Task 4: Update docs and finish verification

**Files:**
- Modify: `docs/SECURITY.md`
- Modify: `docs/plans/2026-03-15-conversation-runtime-binding-design.md`
- Modify: `docs/plans/2026-03-15-conversation-runtime-binding-implementation-plan.md`

**Step 1: Update docs**

Reflect that the conversation layer now uses an explicit runtime binding rather
than bare optional kernel references for high-level execution routing.

**Step 2: Run targeted verification**

Run:
- `cargo test -p loongclaw-app conversation_runtime_binding -- --test-threads=1`
- `cargo test -p loongclaw-app turn_engine_ -- --test-threads=1`
- `cargo test -p loongclaw-app handle_turn_with_runtime -- --test-threads=1`

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
git add docs/plans/2026-03-15-conversation-runtime-binding-design.md \
        docs/plans/2026-03-15-conversation-runtime-binding-implementation-plan.md \
        docs/SECURITY.md \
        crates/app/src/conversation/mod.rs \
        crates/app/src/conversation/runtime_binding.rs \
        crates/app/src/conversation/runtime.rs \
        crates/app/src/conversation/context_engine.rs \
        crates/app/src/conversation/persistence.rs \
        crates/app/src/conversation/turn_engine.rs \
        crates/app/src/conversation/turn_loop.rs \
        crates/app/src/conversation/turn_coordinator.rs \
        crates/app/src/conversation/tests.rs
git commit -m "refactor: add explicit conversation runtime binding"
```
