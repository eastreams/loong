# Conversation Runtime Binding Design

Date: 2026-03-15
Scope: follow-up kernel-first refactor after issue #45 / PR #151

## Problem

The conversation stack still encodes a real architectural distinction as raw
`Option<&KernelContext>` values. That shape is too weak for the current runtime:

1. `None` can mean "intentional direct fallback path"
2. `None` can mean "no kernel authority, so core execution must be denied"
3. `None` can also mean "this helper does not care"

Those meanings are not interchangeable, but the type surface makes them look the
same. The previous kernel-policy unification slice fixed the highest-risk
behavioral bug by restoring no-kernel app/session tool execution while keeping
core-tool execution kernel-bound. Even after that fix, the high-level
conversation/runtime APIs still ask every caller to reconstruct the same meaning
from a bare optional reference.

That leaves the next refactor hazard in place: future callers can accidentally
route a direct-fallback path into a kernel-required seam, or vice versa, without
the API pushing back clearly enough.

## Goals

1. Replace raw optional kernel references with an explicit conversation-scoped
   binding type at the high-level runtime seams.
2. Preserve current behavior:
   - core tools stay kernel-bound
   - lifecycle hooks stay kernel-bound
   - app/session tools still execute through dispatcher-backed no-kernel paths
   - provider request and persistence fallback paths remain available when the
     conversation runtime intentionally runs without a kernel
3. Keep this slice reviewable and local to the conversation module instead of
   sweeping the full repository.

## Non-goals

1. Do not force all provider, connector, ACP, or analytics paths through the
   kernel in this slice.
2. Do not remove every `Option<&KernelContext>` in the repository.
3. Do not redesign the kernel policy chain or app-tool dispatcher behavior.

## Alternatives Considered

### A. Keep `Option<&KernelContext>` and rely on comments/tests

Rejected. The problem is not missing commentary. The problem is that the API
surface does not encode the architectural intent strongly enough.

### B. Require `&KernelContext` everywhere immediately

Rejected. That would either break deliberate direct-fallback paths or expand
this follow-up into a much larger, mixed-purpose architecture change.

### C. Introduce a conversation-scoped binding type

Recommended. This keeps the current behavior but makes caller intent explicit at
the conversation/runtime layer. It also creates a stable seam for later work
that may further tighten policy routing without breaking behavior now.

## Proposed Design

Introduce a new conversation-scoped binding type:

- `ConversationRuntimeBinding::Kernel(&KernelContext)`
- `ConversationRuntimeBinding::Direct`

The type should live in the conversation module and expose small helper methods
such as:

- `kernel(...)`
- `direct()`
- `kernel_context() -> Option<&KernelContext>`
- `is_kernel_bound()`

The important property is semantic, not mechanical: callers stop passing a raw
optional reference and instead declare which conversation-runtime mode they are
using.

## Binding Rules

### 1. Kernel-bound paths

These paths already require kernel authority and should stay that way:

1. core tool execution
2. lifecycle hooks (`bootstrap`, `ingest`, `after_turn`, `compact_context`,
   subagent lifecycle hooks)
3. memory-window reads when the default context engine uses kernel-backed memory

For these actions, the binding type is a transport type, not a relaxation. Code
that truly requires kernel authority should still demand a concrete
`&KernelContext` at the point of execution.

### 2. Direct-fallback paths

These paths intentionally remain available without a kernel:

1. provider request / completion dispatch
2. direct persistence fallback
3. app/session tool dispatch through `AppToolDispatcher`
4. legacy context assembly paths

The binding type should make those paths explicit rather than implicit.

## Scope of Replacement

This slice should replace raw `Option<&KernelContext>` with the binding type in
conversation-layer APIs where the distinction is architectural:

1. `ConversationRuntime`
2. `ConversationContextEngine` context assembly methods
3. persistence helpers in `conversation/persistence.rs`
4. turn loop / turn coordinator orchestration helpers
5. `TurnEngine` app-tool dispatcher boundary and execution helpers

It is acceptable for lower-level provider helpers to continue accepting
`Option<&KernelContext>` internally for now. The conversation layer can convert
the explicit binding back into an optional reference at those lower seams until
those modules get their own dedicated cleanup.

## Expected Behavioral Outcome

Behavior should remain the same after this refactor:

1. no-kernel app/session tools still work
2. no-kernel provider/persistence fallback still works
3. core tools still deny with `no_kernel_context`
4. lifecycle hooks still run only when kernel-bound

The change is that these outcomes will now be routed through APIs that say
directly whether a call is using kernel authority or direct fallback.

## Test Strategy

Add regression coverage that specifically proves:

1. the new binding type preserves existing no-kernel app/session tool execution
2. the binding type preserves direct-fallback context assembly and persistence
3. kernel-bound paths still use the kernel when bound
4. core tools still deny without kernel authority

This should include at least one turn-engine level test and one runtime/context
or persistence level test that exercise the new explicit binding shape.

## Why This Slice Matters

Kernel-first architecture does not only mean "more places use the kernel." It
also means the runtime contract clearly distinguishes:

1. actions that are kernel-authorized
2. actions that are intentionally direct fallback

Without that distinction encoded in the type surface, the architecture remains
easy to misuse even if current behavior happens to pass tests.
