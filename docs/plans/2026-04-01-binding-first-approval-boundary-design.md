# Binding-First Approval Boundary Design

Date: 2026-04-01
Branch: `feat/binding-first-approval-boundary-20260401`
Scope: tighten the app-dispatch approval boundary after `feat: require governed runtime binding`

## Problem

The conversation runtime now has an explicit `ConversationRuntimeBinding`, and
mutating app intents already fail closed before execution when the binding is
advisory-only. That removed the highest-risk behavioral drift, but the app-tool
approval boundary still exposes the old optional-kernel compatibility contract.

Two seams still reconstruct approval semantics from `Option<&KernelContext>`:

1. `AppToolDispatcher::maybe_require_approval(...)` in
   `crates/app/src/conversation/turn_engine.rs`
2. `CoordinatorAppToolDispatcher::maybe_require_approval(...)` in
   `crates/app/src/conversation/turn_coordinator.rs`

That API shape is weaker than the runtime contract around it:

1. `None` can still mean "advisory-only binding"
2. `None` can still mean "compat ingress wrapper"
3. `None` could still mean "caller forgot to preserve the binding semantics"

Those meanings are no longer equivalent. The binding-first runtime refactor only
becomes mechanically truthful when the approval boundary also requires the same
explicit binding contract.

## Goals

1. Make the approval-routing trait boundary binding-first.
2. Remove app-dispatch approval reconstruction from raw
   `Option<&KernelContext>` values.
3. Keep compatibility normalization only in explicit ingress or wrapper seams
   that intentionally translate older call sites into the new binding contract.
4. Preserve current behavior:
   - governed app tools still request approval when configured
   - advisory-only bindings still fail closed before approval routing for
     mutating app intents
   - non-mutating app tools still keep their existing behavior
5. Keep the slice narrow and reviewable.

## Non-goals

1. Do not refactor `AsyncDelegateSpawnRequest` owned binding carriage in this
   slice.
2. Do not remove every `Option<&KernelContext>` from the repository.
3. Do not expand provider, ACP, channel, or connector binding cleanup.
4. Do not redesign governed tool policy or approval persistence semantics.

## Current Evidence

### What is already correct

The current runtime already enforces the most important behavioral invariant:
mutating app tools require a mutating-capable binding before they can even reach
approval routing.

Evidence:

1. `crates/app/src/conversation/turn_engine.rs`
   - `requires_mutating_runtime_binding(...)`
   - `execute_turn_in_context(...)` rejects advisory-only bindings with
     `governed_runtime_binding_required`
2. `crates/app/src/conversation/tests.rs`
   - `governed_runtime_binding_rejects_mutating_app_intent_before_approval_on_advisory_binding`

### What is still drifting

The approval boundary itself still treats binding-first as an adapter layered on
top of the old optional contract:

1. the trait default implementation of
   `maybe_require_approval_with_binding(...)` converts the binding back into an
   optional kernel reference
2. the concrete `DefaultAppToolDispatcher` still implements the optional-kernel
   method as its primary seam
3. the coordinator wrapper still implements the optional-kernel method and then
   reconstructs the binding again

That means the approval contract is still semantically optional even though the
turn engine now reasons in terms of explicit binding.

## Alternatives Considered

### A. Retire optional-kernel compatibility from the approval boundary first

Recommended.

Make `maybe_require_approval_with_binding(...)` the primary trait contract,
remove the optional-kernel method from the approval seam, and keep any
normalization only in explicit compatibility wrappers outside the dispatcher
boundary.

Why this is the right next slice:

1. it closes a real remaining semantic drift seam
2. it stays local to the conversation app-dispatch boundary
3. it matches the already-landed runtime-binding direction
4. it avoids reopening broader history or async delegate work

### B. Start with async delegate owned-binding cleanup

Rejected for this slice.

That work is real, but it is a different seam with different test surface and
blast radius. Pulling it into Task 4 would mix approval-boundary cleanup with
spawn-lifecycle plumbing and make the patch less reviewable.

### C. Do naming-only cleanup and leave both contracts in place

Rejected.

Renaming methods without removing the optional contract would preserve the same
architectural ambiguity while making the code look more settled than it really
is.

## Decision

Implement option A as a bounded D6 follow-up slice:

1. `AppToolDispatcher` becomes binding-first for approval routing
2. concrete dispatcher implementations operate on
   `ConversationRuntimeBinding<'_>` directly
3. explicit compatibility normalization remains allowed only at ingress-style
   seams that intentionally bridge older interfaces into the binding contract

## Proposed Design

### 1. Trait boundary

Change `AppToolDispatcher` so the approval entry contract is:

```rust
async fn maybe_require_approval_with_binding(
    &self,
    session_context: &SessionContext,
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<Option<ApprovalRequirement>, String>;
```

The trait should no longer define approval routing in terms of
`Option<&KernelContext>`.

This does not mean every caller must already be kernel-bound. It means every
caller must be explicit about whether the turn is running in `Kernel` or
advisory-only mode.

### 2. Concrete dispatcher implementation

`DefaultAppToolDispatcher` should implement approval logic directly on the
binding-first method. For this slice, the approval logic itself does not need to
change:

1. governed app tools still compute approval requirements from descriptor
   metadata and config
2. approval persistence still uses the same repository rows and deterministic
   request ids
3. the binding may remain unused inside the sqlite-backed approval persistence
   path for now, but the contract must stay explicit

Important design point:

Unused binding in a concrete implementation is acceptable. Hidden reconstruction
from `Option<&KernelContext>` is not.

### 3. Coordinator wrapper

`CoordinatorAppToolDispatcher` should stop reintroducing the optional-kernel
compat seam. It should implement only the binding-first approval method and
delegate directly to the fallback dispatcher.

### 4. Compatibility boundary rules

This slice intentionally allows lower-priority compatibility wrappers to remain
where they are already explicit and shallow, for example:

1. session-history public helper entrypoints that accept
   `Option<&KernelContext>` and immediately normalize to a binding
2. async delegate spawn request plumbing that still carries owned
   `Option<KernelContext>` until its dedicated cleanup slice

The rule is:

`optional kernel is acceptable only at explicit ingress/compat boundaries, not
at the approval-routing trait boundary`

## Expected Behavioral Outcome

No user-visible product behavior should change in this slice.

Expected outcomes:

1. strict approval mode still persists approval requests for governed app tools
2. advisory-only turns still deny mutating app intents before approval routing
3. the approval boundary becomes architecture-truthful because callers can no
   longer silently erase binding semantics at the dispatcher seam

## Test Strategy

Add focused regression coverage that proves:

1. custom app dispatchers can implement approval routing directly on the
   binding-first seam without an optional-kernel override
2. advisory-only mutating app turns still fail before approval routing
3. governed approval persistence still works through the binding-first contract
4. no approval-boundary call path in `turn_engine` or `turn_coordinator`
   reconstructs the binding from `Option<&KernelContext>`

The slice should prefer test additions to existing `turn_engine.rs` and
`conversation/tests.rs` coverage rather than introducing new modules.

## Documentation Impact

`docs/SECURITY.md` should be tightened so it no longer implies that the
discovery-first compatibility note is the only public optional-kernel seam that
matters. After this slice, the more accurate claim is:

1. approval routing in the conversation app-dispatch path is binding-first
2. remaining optional-kernel surfaces are explicit compatibility wrappers, not
   dispatcher-boundary contracts

## Why This Slice Matters

The repository has already done the harder behavioral work: governed binding is
real, and advisory-only mutation is denied early. Task 4 matters because it
removes one of the last places where the old optional-kernel story still leaks
through a first-class conversation/runtime seam.

That is the right kind of follow-up slice:

1. small enough to review
2. strong enough to improve architectural truthfulness
3. narrow enough to avoid pretending this is a full repository-wide cleanup
