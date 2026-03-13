# Session Wait Batch Design

## Context

LoongClaw's session tool surface already supports:

- filtered `sessions_list` discovery
- batch `session_status`
- batch `session_cancel` / `session_recover`
- single-target `session_wait` with bounded polling and optional `after_id`

That leaves one operator gap: after discovering or remediating several candidate sessions, the
caller still has to wait on each child one by one. That makes the thick session surface uneven and
pushes orchestration loops back into the provider.

External repo review points in the same direction:

- OpenClaw emphasizes a compact operator control surface with primitives such as list, status, log,
  send, steer, cancel, and spawn rather than proliferating top-level tools.
- NanoBot similarly concentrates lifecycle actions into thicker tools and command families instead
  of many narrow single-purpose commands.

## Problem

We need to deepen `session_wait` without adding a new top-level tool and without widening
visibility or child authority.

The design question is how to let operators wait on multiple visible sessions while preserving:

- backward compatibility for existing single-session callers
- shared bounded wait behavior
- per-target visibility boundaries
- per-target terminal or timeout classification
- optional incremental event-tail continuation via the existing `after_id` contract

## Goals

- Extend `session_wait` to accept either `session_id` or `session_ids`.
- Preserve the legacy single-target response shape and status codes.
- Support shared `timeout_ms` and shared `after_id` semantics in batch mode.
- Return per-target results in request order with machine-readable outcome classification.
- Reuse the existing inspection and wait payload shapes rather than inventing a second wait schema.

## Non-Goals

- No new top-level tool such as `session_wait_many`.
- No push streaming or subscription transport.
- No hard-kill or preemptive cancellation semantics.
- No widening of delegated child visibility.
- No full async dispatcher signature refactor in this slice.

## Options Considered

### Option 1: Keep `session_wait` single-target and rely on caller loops

Pros:

- zero request-schema change
- no implementation risk

Cons:

- leaves the session surface inconsistent after batch status and remediation
- forces providers to reimplement bounded wait fan-out
- makes multi-session orchestration less auditable

Rejected.

### Option 2: Extend `session_wait` with `session_ids`

Accept either:

- `session_id`
- `session_ids`

Keep the old single-target shape for `session_id`. In batch mode, use a shared timeout window and
return structured per-target results.

Pros:

- keeps the tool surface compact
- composes directly with `sessions_list`, `session_status`, `session_cancel`, and `session_recover`
- preserves the strong existing single-target wait payload
- keeps visibility enforcement per target

Cons:

- introduces a second response shape for batch calls
- requires explicit classification for mixed completed, timeout, and hidden targets

Chosen.

### Option 3: Add a new batch wait tool

Examples:

- `session_wait_many`
- `sessions_wait`

Pros:

- explicit name
- no dual response shape on `session_wait`

Cons:

- expands the top-level operator surface
- overlaps almost entirely with an existing wait primitive

Rejected.

## Chosen Approach

Extend `session_wait` instead of adding a new tool.

### Request model

`session_wait` accepts exactly one of:

- `session_id: string`
- `session_ids: string[]`

It continues to accept:

- `timeout_ms?: integer`
- `after_id?: integer`

For batch mode, `timeout_ms` and `after_id` apply to the whole request.

### Backward compatibility

When the request uses `session_id`, behavior remains unchanged:

- top-level status stays `ok` or `timeout`
- payload stays the existing wait payload shape

### Batch response shape

Batch mode returns:

- `tool`
- `current_session_id`
- `requested_count`
- `timeout_ms`
- `after_id`
- `result_counts`
- `results[]`

Each result entry returns:

- `session_id`
- `result`
- `message`
- `inspection`

Result values:

- `ok`
- `timeout`
- `skipped_not_visible`

For `ok` and `timeout`, `inspection` reuses the existing single-target wait payload including:

- `wait_status`
- `timeout_ms`
- `after_id`
- `next_after_id`
- `events`
- the normal inspection fields

For hidden or not-visible targets:

- `inspection = null`
- `message` contains the visibility failure reason

### Execution semantics

- Visibility is checked per target.
- Hidden targets are classified immediately as `skipped_not_visible`.
- Visible targets share one bounded timeout window.
- The wait loop polls only the remaining non-terminal visible targets.
- Terminal targets finalize early with `result = ok`.
- Targets still non-terminal at deadline finalize with `result = timeout`.
- Request order is preserved in the final `results[]`.

### Routing choice

`session_wait` still needs async polling, so this slice does not change the dispatcher signature.
Instead, it moves the core wait logic into `tools/session.rs` so the routing split becomes thinner:

- `tools/mod.rs` remains the async app-tool entrypoint
- `tools/session.rs` owns the actual wait semantics and shared payload construction

That keeps this slice focused on user-facing depth without turning it into a cross-cutting async
refactor.

## Testing

Required coverage:

1. provider schema exposes `session_ids` for `session_wait`
2. single-target `session_wait` remains behaviorally unchanged
3. batch wait returns mixed `ok`, `timeout`, and `skipped_not_visible` results in request order
4. existing `after_id` event-tail tests remain green for single-target calls

