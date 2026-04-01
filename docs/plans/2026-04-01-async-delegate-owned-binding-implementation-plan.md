# Async Delegate Owned Binding Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace detached async delegate `Option<KernelContext>` transport with an explicit owned runtime binding contract.

**Architecture:** Keep borrowed `ConversationRuntimeBinding<'_>` as the immediate execution API, but introduce an owned mirror for detached work. The detach point in `turn_coordinator.rs` owns the binding, and spawners borrow it back only when entering child-turn helpers.

**Tech Stack:** Rust, Tokio, async-trait, cargo test, cargo fmt, cargo clippy

---

### Task 1: Add the owned binding type

**Files:**
- Modify: `crates/app/src/conversation/runtime_binding.rs`
- Modify: `crates/app/src/conversation/mod.rs`

**Step 1: Write the failing test**

Add a focused unit test in `crates/app/src/conversation/runtime_binding.rs` that:

```rust
let owned = OwnedConversationRuntimeBinding::from_borrowed(
    ConversationRuntimeBinding::kernel(&kernel_ctx),
);
assert!(owned.is_kernel_bound());
assert_eq!(
    owned.as_borrowed().session_mode(),
    ConversationRuntimeBinding::kernel(&kernel_ctx).session_mode()
);
```

**Step 2: Run test to verify it fails**

Run: `cargo test --workspace --locked owned_conversation_runtime_binding`
Expected: FAIL because `OwnedConversationRuntimeBinding` does not exist yet.

**Step 3: Write minimal implementation**

Implement an owned enum mirroring the borrowed binding:

```rust
pub enum OwnedConversationRuntimeBinding {
    Kernel(KernelContext),
    AdvisoryOnly,
}
```

Add helpers for `from_borrowed(...)`, `as_borrowed(...)`, and mode inspection.

**Step 4: Run test to verify it passes**

Run: `cargo test --workspace --locked owned_conversation_runtime_binding`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/conversation/runtime_binding.rs crates/app/src/conversation/mod.rs
git commit -m "refactor: add owned conversation runtime binding"
```

### Task 2: Move async delegate requests to owned binding

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Update the async delegate queue tests in `crates/app/src/conversation/tests.rs`
to assert:

```rust
assert!(matches!(
    spawn_request.binding,
    crate::conversation::OwnedConversationRuntimeBinding::Kernel(_)
));
```

**Step 2: Run test to verify it fails**

Run:
- `cargo test --workspace --locked handle_turn_with_runtime_delegate_async_preserves_kernel_binding_in_spawn_request`
- `cargo test --workspace --locked handle_turn_with_runtime_executes_delegate_async_via_coordinator_without_waiting`

Expected: FAIL because `AsyncDelegateSpawnRequest` still exposes `kernel_context`.

**Step 3: Write minimal implementation**

Change the detached request shape:

```rust
pub struct AsyncDelegateSpawnRequest {
    pub binding: OwnedConversationRuntimeBinding,
}
```

At the detach seam, convert the parent borrowed binding:

```rust
binding: OwnedConversationRuntimeBinding::from_borrowed(binding),
```

In spawners, borrow it back only at execution time:

```rust
let binding = request.binding.as_borrowed();
```

**Step 4: Run test to verify it passes**

Run the same targeted tests.
Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/conversation/runtime.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "refactor: make async delegate binding owned"
```

### Task 3: Keep local child execution and helper spawners aligned

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/conversation/runtime_binding.rs`

**Step 1: Write the failing test**

Add or tighten a local child-runtime async delegate test so the spawned child
still executes successfully when the request carries owned governed binding, and
keep the owned-binding unit tests covering advisory-mode round-tripping because
advisory/direct parents never reach async delegate spawn.

**Step 2: Run test to verify it fails**

Run: `cargo test --workspace --locked handle_turn_with_runtime_delegate_child_cannot_reenter_delegate_async_by_default`
Expected: FAIL if helper spawners still depend on `request.kernel_context`.

**Step 3: Write minimal implementation**

Update test-only spawners to use:

```rust
let binding = request.binding.as_borrowed();
```

and pass that borrowed view into cleanup and child-turn execution helpers.

**Step 4: Run test to verify it passes**

Run: `cargo test --workspace --locked handle_turn_with_runtime_delegate_child_cannot_reenter_delegate_async_by_default`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/conversation/tests.rs
git commit -m "test: align async delegate helpers with owned binding"
```

### Task 4: Full verification

**Files:**
- Modify: `docs/plans/2026-04-01-async-delegate-owned-binding-design.md`
- Modify: `docs/plans/2026-04-01-async-delegate-owned-binding-implementation-plan.md`

**Step 1: Run formatting verification**

Run: `cargo fmt --all -- --check`
Expected: PASS

**Step 2: Run strict lint verification**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 3: Run workspace tests**

Run: `cargo test --workspace --locked`
Expected: PASS

**Step 4: Inspect final diff**

Run:
- `git status --short`
- `git diff --stat`

Expected: only the owned-binding slice and its docs are present.

**Step 5: Commit**

```bash
git add docs/plans/2026-04-01-async-delegate-owned-binding-design.md docs/plans/2026-04-01-async-delegate-owned-binding-implementation-plan.md crates/app/src/conversation/runtime_binding.rs crates/app/src/conversation/mod.rs crates/app/src/conversation/runtime.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "refactor: preserve async delegate binding as owned authority"
```
