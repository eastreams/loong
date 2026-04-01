# Binding-First Approval Boundary Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Retire legacy optional-kernel compatibility from the conversation
app-dispatch approval boundary so approval routing is binding-first and any
remaining `Option<&KernelContext>` normalization stays limited to explicit
compatibility wrappers.

**Architecture:** Change the `AppToolDispatcher` approval contract to accept
`ConversationRuntimeBinding<'_>` directly, update concrete dispatcher
implementations to use that contract as the primary seam, and prove with focused
tests that mutating advisory-only turns still fail before approval routing while
governed approval persistence still works unchanged.

**Tech Stack:** Rust, `async_trait`, `loongclaw-app`, conversation runtime,
turn engine, turn coordinator, sqlite-backed session repository, security docs

---

## Execution Tasks

Verification note: use a unique test prefix such as
`binding_first_approval_boundary_` for any new coverage added in this slice.

### Task 1: Add failing tests for the binding-first dispatcher seam

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/conversation/turn_engine.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- a custom `AppToolDispatcher` can implement only
  `maybe_require_approval_with_binding(...)` and still participate in turn
  execution correctly
- mutating app intents running under `ConversationRuntimeBinding::direct()`
  still fail closed before approval routing
- governed app-tool approval persistence still succeeds when the dispatcher is
  called through the binding-first seam

Suggested test names:

```rust
async fn binding_first_approval_boundary_custom_dispatcher_uses_binding_only() { /* ... */ }
async fn binding_first_approval_boundary_advisory_mutation_denies_before_approval() { /* ... */ }
async fn binding_first_approval_boundary_persists_governed_request() { /* ... */ }
```

**Step 2: Run the tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-app binding_first_approval_boundary_ -- --test-threads=1
```

Expected: FAIL because the trait and concrete implementations still route
approval through `maybe_require_approval(...)` with `Option<&KernelContext>`.

**Step 3: Write the minimal implementation**

Do not change approval policy semantics yet. Only express the same behavior
through the binding-first seam.

**Step 4: Run the tests to verify they pass**

Run the same command and expect PASS.

**Step 5: Commit**

```bash
git add crates/app/src/conversation/tests.rs crates/app/src/conversation/turn_engine.rs
git commit -m "test: lock approval boundary to binding-first seam"
```

### Task 2: Retire the optional-kernel trait contract in `turn_engine`

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`

**Step 1: Replace the trait surface**

Change `AppToolDispatcher` so approval routing is defined only by
`maybe_require_approval_with_binding(...)`.

Use a trait default like:

```rust
async fn maybe_require_approval_with_binding(
    &self,
    _session_context: &SessionContext,
    _intent: &ToolIntent,
    _descriptor: &crate::tools::ToolDescriptor,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<Option<ApprovalRequirement>, String> {
    Ok(None)
}
```

Remove the optional-kernel approval method from the trait.

**Step 2: Update `DefaultAppToolDispatcher`**

Move the current approval logic under the binding-first method directly:

```rust
async fn maybe_require_approval_with_binding(
    &self,
    session_context: &SessionContext,
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<Option<ApprovalRequirement>, String> {
    let _ = binding;
    // existing governed approval persistence logic
}
```

Keep the existing repository writes, request ids, and denial strings unchanged.

**Step 3: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app binding_first_approval_boundary_ governed_tool_approval_request_ -- --test-threads=1
```

Expected: PASS

**Step 4: Commit**

```bash
git add crates/app/src/conversation/turn_engine.rs
git commit -m "refactor: make approval dispatch binding-first"
```

### Task 3: Remove the coordinator compatibility loopback

**Files:**
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Simplify `CoordinatorAppToolDispatcher`**

Delete the optional-kernel approval override and keep only the binding-first
implementation that delegates to `self.fallback`.

The coordinator wrapper should no longer do:

```rust
let binding = ConversationRuntimeBinding::from_optional_kernel_context(kernel_ctx);
```

at the approval boundary.

**Step 2: Add or tighten regression coverage**

If needed, extend an existing coordinator-level approval test so it proves the
turn still reaches `NeedsApproval(...)` or the persisted grant path through the
binding-first dispatcher contract.

Suggested test target:

```rust
async fn binding_first_approval_boundary_coordinator_routes_needs_approval() { /* ... */ }
```

**Step 3: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app binding_first_approval_boundary_ handle_turn_with_runtime_requires_approval_before_delegate_execution -- --test-threads=1
```

Expected: PASS

**Step 4: Commit**

```bash
git add crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs
git commit -m "refactor: remove approval boundary loopback in coordinator"
```

### Task 4: Tighten security docs and run verification

**Files:**
- Modify: `docs/SECURITY.md`

**Step 1: Update the security wording**

State explicitly that:

- conversation app-dispatch approval routing is now binding-first
- remaining optional-kernel entrypoints are explicit compatibility wrappers
- the public discovery-first entrypoint remains a compatibility wrapper, not a
  first-class dispatcher contract

**Step 2: Run verification**

Run:

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: PASS

**Step 3: Inspect staged scope before commit**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Expected: only Task 4 scoped files are staged

**Step 4: Commit**

```bash
git add docs/SECURITY.md
git commit -m "docs: clarify binding-first approval boundary"
```
