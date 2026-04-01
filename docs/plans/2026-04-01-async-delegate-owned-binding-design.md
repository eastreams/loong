# Async Delegate Owned Binding Design

Date: 2026-04-01
Branch: `feat/async-delegate-owned-binding-20260401`
Scope: tighten detached async delegate authority propagation around an owned runtime binding seam

## Problem

The binding-first approval boundary slice made `ConversationRuntimeBinding` the
main borrowed execution contract for conversation turns, but detached async
delegate spawn still carries inherited authority as `Option<KernelContext>`.

That shape is weaker than the actual runtime contract:

1. it treats "owned governed binding" as an implementation detail instead of a
   first-class concept
2. every async delegate spawner reconstructs borrowed binding ad hoc through
   `ConversationRuntimeBinding::from_optional_kernel_context(...)`
3. tests validate kernel inheritance indirectly through kernel-context presence
   rather than an explicit owned binding contract

The result is a seam that still speaks in raw kernel-state storage terms even
though the surrounding runtime has already moved to binding-first semantics.

## Goals

1. Make detached async delegate spawn requests carry explicit owned runtime
   binding semantics.
2. Preserve parent governed/advisory mode through detached child execution
   without borrowing parent stack state.
3. Keep the patch reviewable and local to conversation runtime and delegate test
   seams.

## Non-goals

1. Do not sweep the repository for every `Option<&KernelContext>` or
   `from_optional_kernel_context(...)` use.
2. Do not redesign synchronous delegate execution.
3. Do not bundle `session_history` cleanup or unrelated compatibility wrapper
   changes into this slice.
4. Do not force `run_started_delegate_child_turn_with_runtime(...)` to become
   fully owned end-to-end if a borrowed execution seam remains sufficient.

## Alternatives Considered

### A. Keep `Option<KernelContext>` in async delegate requests

Rejected. That preserves the current ambiguity and keeps binding semantics
secondary to raw kernel-context transport.

### B. Convert the entire conversation runtime to owned bindings immediately

Rejected for this slice. It would mix unrelated lifetimes and API cleanup into a
larger refactor than needed for the current authority seam.

### C. Introduce a narrow owned binding type for detached async delegate flow

Recommended. It makes the detached seam explicit while keeping the rest of the
runtime on the existing borrowed `ConversationRuntimeBinding<'_>` API.

## Decision

Introduce `OwnedConversationRuntimeBinding` alongside the existing borrowed
binding enum, and thread it through detached async delegate requests and
spawners.

The detach point in `turn_coordinator.rs` becomes the ownership boundary:

1. parent execution starts with borrowed `ConversationRuntimeBinding<'_>`
2. detached request stores `OwnedConversationRuntimeBinding`
3. async spawner converts the owned binding back to borrowed only when entering
   cleanup helpers and child-turn execution

This slice does not widen delegate eligibility. `delegate_async` already
requires a mutating runtime binding, so advisory/direct parents are denied
earlier in turn preparation and never reach the detached spawner path.

## Proposed Design

### 1. Add `OwnedConversationRuntimeBinding`

`crates/app/src/conversation/runtime_binding.rs` will define:

1. `OwnedConversationRuntimeBinding::Kernel(KernelContext)`
2. `OwnedConversationRuntimeBinding::AdvisoryOnly`

The owned type mirrors the semantics of the borrowed binding and provides:

1. `from_borrowed(ConversationRuntimeBinding<'_>) -> Self`
2. `kernel(KernelContext) -> Self`
3. `advisory_only()` / `direct()`
4. `as_borrowed(&self) -> ConversationRuntimeBinding<'_>`
5. helpers such as `kernel_context()`, `is_kernel_bound()`, and `session_mode()`

This keeps governed versus advisory semantics explicit even after detaching work
into another task.

### 2. Change async delegate spawn requests to store owned binding

`AsyncDelegateSpawnRequest` in `runtime.rs` will replace:

1. `kernel_context: Option<KernelContext>`

with:

1. `binding: OwnedConversationRuntimeBinding`

The request now states the intended execution contract directly rather than
reconstructing it from optional raw kernel state.

### 3. Convert at the child execution seam only

The default async delegate spawner and local test spawners will stop calling
`ConversationRuntimeBinding::from_optional_kernel_context(...)` against request
storage. Instead they will borrow from the owned binding just-in-time:

1. `request.binding.as_borrowed()` for
   `with_prepared_subagent_spawn_cleanup_if_kernel_bound(...)`
2. `request.binding.as_borrowed()` for
   `run_started_delegate_child_turn_with_runtime(...)`

This preserves the current execution API where borrowed binding is still the
right shape for immediate turn handling.

### 4. Tighten tests around the explicit contract

Conversation tests should stop asserting raw `kernel_context` presence on async
delegate requests. They should assert the owned binding directly:

1. governed parent turns queue `OwnedConversationRuntimeBinding::Kernel(...)`
2. advisory binding still round-trips correctly in owned-binding unit tests
3. local async delegate child execution still behaves as before once the owned
   binding is borrowed at the execution seam

## Expected Behavioral Outcome

1. Detached async delegate requests preserve parent runtime authority as an
   explicit owned binding contract.
2. Governed parents still spawn governed child turns.
3. Advisory/direct parents remain denied before async spawn, matching the
   existing mutating-binding policy contract.
4. The patch reduces ad hoc binding reconstruction without changing unrelated
   runtime behavior.

## Test Strategy

Add focused regression coverage for:

1. owned binding round-trips between borrowed and borrowed-again views
2. async delegate queueing preserves a governed owned binding
3. local async child execution continues to use the borrowed binding derived
   from the owned request binding

## Why This Slice Matters

This is the highest-value remaining binding-first seam in detached delegate
execution. It does not invent new capability rules; it makes the existing rules
explicit in the API that crosses the async ownership boundary.
