# Async Delegate Owned Binding Restoration Design

Date: 2026-04-01
Branch: `feat/async-delegate-owned-binding-restoration-20260401`
Scope: restore an explicit owned runtime-binding contract at the detached async delegate seam

## Problem

The current conversation runtime exposes explicit borrowed binding semantics
through `ConversationRuntimeBinding<'_>`, but detached async delegate transport
still falls back to raw optional kernel-state storage:

1. `AsyncDelegateSpawnRequest` stores `kernel_context: Option<KernelContext>`
2. async delegate spawners reconstruct borrowed binding through
   `ConversationRuntimeBinding::from_optional_kernel_context(...)`
3. the detach boundary therefore speaks in storage terms rather than runtime
   contract terms

This is exactly the kind of governed/direct drift tracked by roadmap item `D6`.
The detach seam crosses an ownership boundary, so it is the place where the
runtime should be most explicit about whether authority is governed or
advisory-only.

## Goals

1. Make detached async delegate requests carry explicit owned binding semantics.
2. Preserve the current governed-versus-advisory behavior without broadening
   delegate eligibility.
3. Keep the patch narrow and local to async delegate transport, spawners, and
   focused regression tests.

## Non-goals

1. Do not sweep the repository for every
   `ConversationRuntimeBinding::from_optional_kernel_context(...)` use.
2. Do not redesign synchronous `delegate` execution.
3. Do not bundle `session_history`, approval routing, or chat diagnostics into
   this slice.
4. Do not change the high-risk policy rule that advisory parents cannot reach
   `delegate_async`.

## Alternatives Considered

### A. Keep `Option<KernelContext>` in `AsyncDelegateSpawnRequest`

Rejected. That preserves the current ambiguity and keeps the ownership boundary
describing raw storage instead of the execution contract.

### B. Convert the whole conversation runtime to owned bindings

Rejected. That would widen the patch from one detached seam into a broad API
refactor with much higher merge risk.

### C. Introduce a narrow owned binding type for detached async delegate flow

Recommended. It keeps the immediate execution API borrowed, but makes the
detached transport contract explicit and auditable.

## Decision

Introduce `OwnedConversationRuntimeBinding` as the detached transport shape and
thread it through `AsyncDelegateSpawnRequest`.

The boundary will be:

1. parent turn execution uses borrowed `ConversationRuntimeBinding<'_>`
2. `turn_coordinator.rs` converts that borrowed binding into an owned binding
   when enqueuing async delegate work
3. async spawners borrow from the owned binding only when entering cleanup and
   child-turn execution helpers

This keeps the current behavior while removing raw optional-kernel authority
from the detached request contract.

## Proposed Design

### 1. Add `OwnedConversationRuntimeBinding`

`crates/app/src/conversation/runtime_binding.rs` should define an owned mirror
of the borrowed binding enum:

1. `OwnedConversationRuntimeBinding::Kernel(KernelContext)`
2. `OwnedConversationRuntimeBinding::Direct`

The owned type should provide:

1. `from_borrowed(...)`
2. `as_borrowed(&self)`
3. `kernel_context()`
4. `is_kernel_bound()`

The names and helpers should stay intentionally boring. This slice is about
contract truthfulness, not abstraction cleverness.

### 2. Change detached request storage

`AsyncDelegateSpawnRequest` in `crates/app/src/conversation/runtime.rs` should
replace:

1. `kernel_context: Option<KernelContext>`

with:

1. `binding: OwnedConversationRuntimeBinding`

That makes the detached request state the intended runtime authority directly.

### 3. Borrow only at execution seams

The default async delegate spawner and test spawners should stop reconstructing
binding from optional kernel context. They should instead call
`request.binding.as_borrowed()` only at the helper boundary:

1. `with_prepared_subagent_spawn_cleanup_if_kernel_bound(...)`
2. `run_started_delegate_child_turn_with_runtime(...)`

This keeps the borrowed API where it still fits, without leaking raw optional
authority into detached transport.

### 4. Tighten regression coverage

Tests should assert the explicit contract:

1. queued governed async delegate requests carry
   `OwnedConversationRuntimeBinding::Kernel(...)`
2. direct/advisory round-tripping still behaves correctly at the binding-type
   level
3. child execution still receives the correct borrowed view derived from the
   owned request binding

## Expected Behavioral Outcome

1. Detached async delegate requests preserve runtime authority as an explicit
   owned binding contract.
2. Governed parent turns still spawn governed child turns.
3. Advisory/direct parents remain denied before async spawn, matching the
   existing policy contract.
4. No unrelated runtime behavior changes.

## Test Strategy

Add focused regression coverage for:

1. owned binding round-trips between borrowed and owned views
2. async delegate queueing preserves governed binding explicitly
3. local child execution continues to work when the request stores owned
   binding instead of raw `KernelContext`
4. existing deny paths still fail closed for advisory parents

## Rollout Notes

This should land as a follow-up slice after `#768`, or as an explicit stacked
branch if review timing requires that. It should not be folded back into the
already-ready delivery PR.

## Why This Slice Matters

This is a small but important contract repair. The async delegate detach point
is one of the few places where authority crosses an ownership boundary. If that
seam still speaks in raw optional-kernel terms, the rest of the binding-first
story remains less truthful than the surrounding runtime already is.
