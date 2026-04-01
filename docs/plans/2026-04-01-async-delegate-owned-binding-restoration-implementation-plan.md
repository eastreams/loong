# Async Delegate Owned Binding Restoration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restore an explicit owned runtime-binding contract for detached async delegate requests.

**Architecture:** Keep `ConversationRuntimeBinding<'_>` as the borrowed execution API for immediate runtime helpers, but add an owned mirror for detached transport. The detach point in `turn_coordinator.rs` should own the binding, and spawners should borrow it back only when they actually execute cleanup or child turns.

**Tech Stack:** Rust, Tokio, async-trait, cargo test, cargo fmt, cargo clippy

---

### Task 1: Add the owned binding type

**Files:**
- Modify: `crates/app/src/conversation/runtime_binding.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Test: `crates/app/src/conversation/runtime_binding.rs`

**Step 1: Write the failing test**

Add a focused unit test in `crates/app/src/conversation/runtime_binding.rs`
that:

```rust
let owned = OwnedConversationRuntimeBinding::from_borrowed(
    ConversationRuntimeBinding::kernel(&kernel_ctx),
);
assert!(owned.is_kernel_bound());
assert!(matches!(
    owned.as_borrowed(),
    ConversationRuntimeBinding::Kernel(_)
));
```

**Step 2: Run test to verify it fails**

Run: `cargo test --workspace --locked owned_conversation_runtime_binding`
Expected: FAIL because `OwnedConversationRuntimeBinding` does not exist yet.

**Step 3: Write minimal implementation**

Add:

```rust
pub enum OwnedConversationRuntimeBinding {
    Kernel(KernelContext),
    Direct,
}
```

Implement only the helpers needed by detached async delegate flow:
`from_borrowed(...)`, `as_borrowed(...)`, `kernel_context()`, and
`is_kernel_bound()`.

**Step 4: Run test to verify it passes**

Run: `cargo test --workspace --locked owned_conversation_runtime_binding`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/conversation/runtime_binding.rs crates/app/src/conversation/mod.rs
git commit -m "refactor: add owned conversation runtime binding"
```

### Task 2: Move async delegate requests onto the owned binding contract

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Update the async delegate queue tests in
`crates/app/src/conversation/tests.rs` to assert:

```rust
assert!(matches!(
    &spawn_request.binding,
    crate::conversation::OwnedConversationRuntimeBinding::Kernel(_)
));
```

**Step 2: Run test to verify it fails**

Run:
- `cargo test --workspace --locked handle_turn_with_runtime_delegate_async_preserves_kernel_binding_in_spawn_request`
- `cargo test --workspace --locked handle_turn_with_runtime_executes_delegate_async_via_coordinator_without_waiting`

Expected: FAIL because `AsyncDelegateSpawnRequest` still stores
`kernel_context`.

**Step 3: Write minimal implementation**

Replace:

```rust
pub kernel_context: Option<KernelContext>,
```

with:

```rust
pub binding: OwnedConversationRuntimeBinding,
```

At the detach seam in `turn_coordinator.rs`, convert the parent borrowed
binding into the owned form:

```rust
binding: OwnedConversationRuntimeBinding::from_borrowed(binding),
```

**Step 4: Run test to verify it passes**

Run the same targeted tests.
Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/conversation/runtime.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "refactor: store owned binding in async delegate requests"
```

### Task 3: Borrow the owned binding only at child execution seams

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing test**

Tighten local async delegate child execution coverage so the spawner path still
works when the request carries owned binding and no longer exposes
`request.kernel_context`.

**Step 2: Run test to verify it fails**

Run:
- `cargo test --workspace --locked handle_turn_with_runtime_delegate_child_cannot_reenter_delegate_async_by_default`
- `cargo test --workspace --locked handle_turn_with_runtime_delegate_async_preserves_kernel_context_in_spawn_request`

Expected: FAIL if the spawner path still reads `request.kernel_context`.

**Step 3: Write minimal implementation**

In runtime and test helper spawners, derive the borrowed binding only at the
helper call sites:

```rust
let binding = request.binding.as_borrowed();
```

Use that borrowed view for cleanup and child-turn execution helpers.

**Step 4: Run test to verify it passes**

Run the same targeted tests.
Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/conversation/runtime.rs crates/app/src/conversation/tests.rs
git commit -m "test: align async delegate spawners with owned binding"
```

### Task 4: Verify the bounded slice end-to-end

**Files:**
- Modify: `docs/plans/2026-04-01-async-delegate-owned-binding-restoration-design.md`
- Modify: `docs/plans/2026-04-01-async-delegate-owned-binding-restoration-implementation-plan.md`

**Step 1: Run formatting verification**

Run: `cargo fmt --all -- --check`
Expected: PASS

**Step 2: Run strict lint verification**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 3: Run targeted regression coverage**

Run:
- `cargo test -p loongclaw-app governed_runtime_binding --features memory-sqlite`
- `cargo test -p loongclaw-app approval_request_resolve --features memory-sqlite`
- `cargo test -p loongclaw-app handle_turn_with_runtime_delegate_async_preserves_kernel_binding_in_spawn_request --features memory-sqlite`
- `cargo test -p loongclaw-app handle_turn_with_runtime_delegate_async_preserves_kernel_context_in_spawn_request --features memory-sqlite`

Expected: PASS

**Step 4: Run workspace tests**

Run:
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Expected: PASS

**Step 5: Inspect final diff**

Run:
- `git status --short`
- `git diff --stat`

Expected: only the owned-binding restoration slice and its design docs are
present.

**Step 6: Commit**

```bash
git add docs/plans/2026-04-01-async-delegate-owned-binding-restoration-design.md docs/plans/2026-04-01-async-delegate-owned-binding-restoration-implementation-plan.md crates/app/src/conversation/runtime_binding.rs crates/app/src/conversation/mod.rs crates/app/src/conversation/runtime.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "refactor: restore owned binding for async delegate transport"
```
