# Session History Binding-First Discovery Summary Design

Date: 2026-04-01
Branch: `feat/session-history-binding-first-20260401`
Scope: tighten the remaining public discovery-summary compatibility seam in `session_history`

## Problem

`crates/app/src/conversation/session_history.rs` is already mostly
binding-first:

1. `load_safe_lane_event_summary(...)` takes `ConversationRuntimeBinding<'_>`
2. `load_fast_lane_tool_batch_event_summary(...)` takes
   `ConversationRuntimeBinding<'_>`
3. `load_turn_checkpoint_event_summary(...)` takes
   `ConversationRuntimeBinding<'_>`
4. the real discovery-first implementation already lives in
   `load_discovery_first_event_summary_with_binding(...)`

The remaining drift is the primary public discovery-first helper name:

1. `load_discovery_first_event_summary(...)` still accepts
   `Option<&KernelContext>`
2. it immediately normalizes that option back into
   `ConversationRuntimeBinding<'_>`
3. the public contract therefore still describes runtime authority in raw
   kernel-presence terms even though the module already reasons in explicit
   binding semantics

This is not a behavioral bug. It is a public API truthfulness problem.

## Goals

1. Make the primary public discovery-first session-history API binding-first.
2. Keep any remaining optional-kernel normalization in an explicitly named
   compatibility shim instead of the main public entrypoint.
3. Preserve behavior:
   - kernel-bound callers still use kernel memory-window reads
   - advisory/direct callers still use sqlite/direct reads
   - discovery-first summaries stay byte-for-byte equivalent for the same input
4. Keep the patch narrow and reviewable.

## Non-goals

1. Do not sweep unrelated `from_optional_kernel_context(...)` uses across the
   repository.
2. Do not refactor chat, provider, ACP, or turn-coordinator call paths in this
   slice.
3. Do not redesign history summary formats or memory-window semantics.
4. Do not remove explicit compatibility helpers everywhere; only tighten this
   one public seam.

## Alternatives Considered

### A. Leave the public `Option<&KernelContext>` helper as-is

Rejected. That keeps the most visible session-history entrypoint semantically
weaker than the module around it.

### B. Add a second public binding-first helper and leave the old one as the default

Rejected. That would make the public surface describe two competing contracts at
the same level and encourage drift to continue.

### C. Promote the main public helper name to binding-first and move the old shape behind an explicit compatibility shim

Recommended. It makes the module's primary API architecture-truthful while
still leaving a narrow migration path for any older callers that truly still
need optional-kernel normalization.

## Decision

Implement option C in one bounded slice:

1. `load_discovery_first_event_summary(...)` becomes binding-first
2. add a clearly named compatibility wrapper such as
   `load_discovery_first_event_summary_with_kernel_context(...)`
3. export both only if needed, but treat the binding-first function as the
   canonical public entrypoint

## Proposed Design

### 1. Public API shape

Change the main public helper in `session_history.rs` to:

```rust
pub async fn load_discovery_first_event_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<DiscoveryFirstEventSummary>
```

This brings discovery-first in line with the neighboring public summary
helpers.

### 2. Compatibility shim

Keep optional-kernel normalization available only in an explicitly named helper:

```rust
pub async fn load_discovery_first_event_summary_with_kernel_context(
    session_id: &str,
    limit: usize,
    kernel_ctx: Option<&KernelContext>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<DiscoveryFirstEventSummary>
```

That keeps the compatibility seam real but no longer lets it masquerade as the
main runtime contract.

### 3. Internal implementation

The existing binding-aware implementation should stay the single real worker.
This slice should minimize churn:

1. keep one binding-first implementation path
2. let the compatibility shim normalize to `ConversationRuntimeBinding` and call
   the canonical helper
3. avoid duplicating discovery-first summary logic

### 4. Tests

Tests should prove two things separately:

1. the canonical public API now accepts explicit runtime binding directly
2. the explicit compatibility shim still preserves the old `Option<&KernelContext>`
   behavior for migration callers

## Expected Behavioral Outcome

1. No discovery-first summary behavior changes.
2. The primary public session-history contract becomes binding-first.
3. Optional-kernel normalization remains available only at a deliberately named
   compatibility seam.

## Test Strategy

Add focused regression coverage for:

1. public discovery-first summary calls succeed for explicit direct binding
2. public discovery-first summary calls succeed for explicit kernel binding
3. the compatibility shim still accepts `None` and `Some(&ctx)`
4. kernel-backed discovery-first calls still route through the memory-window
   core operation with the same payload as before

## Why This Slice Matters

The repository has already done the harder work of moving history internals to
binding-aware execution. This slice finishes the public contract so the API
stops telling a weaker story than the implementation underneath it.
