# Conversation Binding Normalization Design

Date: 2026-03-15
Scope: follow-up kernel-first refactor after issues #45, #154

## Problem

Issue #154 introduced `ConversationRuntimeBinding` at the main conversation
runtime seams, but the conversation module still exposes mixed contracts.

The remaining inconsistencies are concentrated in:

1. public conversation entrypoints such as `ConversationTurnLoop` and
   `ConversationTurnCoordinator`
2. conversation diagnostics and history helpers in `session_history.rs`
3. helper paths that still accept `Option<&KernelContext>` only to normalize it
   immediately into `ConversationRuntimeBinding`
4. shared followup helpers that still accept optional kernel context only to
   forward the explicit binding into runtime completion

That means the architecture is better than before, but not yet internally
consistent. A caller still has to remember which conversation-layer APIs are
binding-first and which still overload `None` to mean one of several things.

## Goals

1. Complete conversation-layer normalization so conversation-facing APIs expose
   explicit direct-versus-kernel-bound execution mode.
2. Preserve behavior:
   - direct provider fallback still works
   - kernel-backed memory/history windows still run when bound
   - direct history fallback still works when unbound
   - core-tool execution still fails closed without kernel authority
3. Keep the slice reviewable and isolated from provider/ACP leaf work.

## Non-goals

1. Do not sweep the provider module in this slice.
2. Do not redesign failover telemetry or provider auth/profile selection.
3. Do not force ACP internals onto a new binding type here.

## Alternatives Considered

### A. Stop after #154

Rejected. The conversation module would continue exposing mixed contracts, which
keeps the remaining ambiguity exactly where orchestration code is most likely to
drift.

### B. Normalize provider and conversation leftovers together

Rejected. That would turn one clean follow-up into a cross-module refactor while
the stacked base PRs are still open.

### C. Finish conversation-layer normalization first

Recommended. It produces a clean, explicit conversation contract and leaves the
provider/ACP follow-up with a smaller, better-defined boundary.

## Proposed Design

Use `ConversationRuntimeBinding` as the canonical execution-mode type across the
rest of the conversation module.

In practice this means:

1. conversation entrypoints accept or derive a binding instead of propagating
   raw `Option<&KernelContext>`
2. conversation diagnostics/history helpers accept binding directly
3. any leaf helper that still truly needs `Option<&KernelContext>` gets it only
   from `binding.kernel_context()`

## Scope of Replacement

This slice should cover the remaining conversation-layer seams:

1. `conversation/turn_loop.rs`
   - public runtime entrypoints
   - terminal action helpers
   - round evaluation helpers
2. `conversation/turn_coordinator.rs`
   - public orchestration entrypoints
   - checkpoint/diagnostic helpers
   - reply resolution and event persistence helpers that currently normalize
     optional kernel context internally
3. `conversation/session_history.rs`
   - turn checkpoint summary loading
   - safe-lane event summary loading
   - memory-window backed history fallback helpers
4. `conversation/turn_shared.rs`
   - shared completion fallback helpers used by the loop/coordinator paths

## Binding Rules

### 1. Conversation layer

The conversation layer should speak in terms of:

1. `ConversationRuntimeBinding::Kernel`
2. `ConversationRuntimeBinding::Direct`

This is where architectural intent matters most.

### 2. Leaf conversions

Leaf helpers may still use `Option<&KernelContext>` temporarily when:

1. they call older provider helpers not yet normalized
2. they branch between kernel-backed memory windows and direct fallback

The key rule is that the conversion happens at the leaf, not at the
conversation-facing seam.

## Expected Behavioral Outcome

Behavior after this refactor should stay the same:

1. `handle_turn*` and related conversation entrypoints still support direct
   fallback when intentionally unbound
2. history and diagnostics helpers still prefer kernel-backed memory windows
   when bound
3. history and diagnostics helpers still fall back to direct memory access when
   unbound
4. provider leaf helpers can remain unchanged for now

The change is architectural clarity: the conversation module will no longer mix
explicit binding seams with raw optional-kernel seams.

## Test Strategy

Add regression coverage that proves:

1. conversation entrypoints continue to work in both direct and kernel-bound
   mode after signature normalization
2. history summary helpers still prefer kernel memory when bound
3. history summary helpers still read sqlite/direct state when unbound
4. no behavior changes leak into turn execution and checkpoint diagnostics

## Why This Slice Matters

Kernel-first architecture is not complete when only the middle of the call graph
is explicit. The public conversation contract and its internal orchestration
helpers also need to declare whether they are running kernel-bound or direct.

Once that is true, the next provider/ACP cleanup can start from a stable,
uniform conversation boundary instead of inheriting a partially normalized one.
