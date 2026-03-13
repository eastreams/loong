# Sessions List Discovery Design

## Context

LoongClaw's session control surface is now materially stronger:

- `session_status` and `session_wait` expose normalized delegate lifecycle metadata
- `session_cancel` handles queued cancellation and cooperative running cancellation
- `session_recover` handles overdue queued and overdue running async delegate children

That still leaves a practical operator gap: the control tools are usable only after the caller
already knows which `session_id` needs attention.

At the same time, the current lifecycle implementation still leans on recent event windows. That is
acceptable for thin inspection, but it becomes a problem if we want discovery features to identify
stale delegate children reliably after a session has accumulated unrelated events.

## Problem

We need the next thick slice to improve operator discovery without adding new tool sprawl.

Two issues need to be solved together:

1. `sessions_list` is too coarse. It returns visible sessions, but offers no targeted filtering for
   root operators looking for stale delegate work.
2. `delegate_lifecycle` currently depends on a bounded recent-event window. A long-running child can
   push `delegate_queued` / `delegate_started` out of that window, which makes lifecycle-based
   discovery unreliable.

## Goals

- Extend `sessions_list` so root operators can find visible sessions by machine-readable filters.
- Allow `sessions_list` to identify overdue async delegate children directly.
- Make delegate lifecycle inspection use stable lifecycle anchor events rather than arbitrary recent
  event windows.
- Reuse the improved lifecycle path in `session_status` and `session_wait`.
- Preserve the existing root/child visibility boundaries and tool surface.

## Non-Goals

- No new discovery tool such as `session_search`.
- No batch mutation tool such as `session_recover_many` or `session_cancel_many`.
- No schema migration or new sqlite table.
- No broad session-tree browsing for delegated child sessions.
- No hard kill / worker registry / push subscriptions.

## Options Considered

### Option 1: Add app-layer filters to `sessions_list` and reuse recent events

Pros:

- smallest implementation diff
- no repository changes

Cons:

- overdue detection remains fragile once lifecycle anchor events are pushed out of the recent-event
  window
- discovery results can silently regress on noisier sessions

Rejected.

### Option 2: Strengthen lifecycle reads and extend `sessions_list`

Keep the existing `sessions_list` tool, but:

- add filter parameters for stateful discovery
- introduce a stable repository read for lifecycle anchor events
- reuse the stronger lifecycle path across `sessions_list`, `session_status`, and `session_wait`

Pros:

- fixes the causal weakness rather than just layering a filter on top
- keeps the tool surface compact
- improves both discovery and inspection correctness

Cons:

- larger code change than an app-only filter
- adds one more repository read path

Chosen.

### Option 3: Add a dedicated discovery tool

Examples:

- `session_search`
- `delegate_overdue_list`

Pros:

- highly explicit semantics
- leaves `sessions_list` unchanged

Cons:

- expands tool count for a problem the existing session list already owns
- splits discovery semantics across overlapping tools

Rejected.

## Chosen Approach

Enhance `sessions_list` instead of adding a new tool.

### Request filters

Add narrow, operator-meaningful filters:

- `limit`
- `state`
- `kind`
- `parent_session_id`
- `overdue_only`
- `include_delegate_lifecycle`

This keeps the tool simple while covering the real stale-delegate discovery flow.

### Stable lifecycle anchors

Introduce a repository helper that reads only lifecycle-relevant session events:

- `delegate_queued`
- `delegate_started`
- `delegate_cancel_requested`

This avoids relying on a fixed `recent_events` window when reconstructing delegate lifecycle.

### Shared lifecycle path

Use the lifecycle-anchor events for:

- `sessions_list` discovery and optional lifecycle payloads
- `session_status`
- `session_wait`
- internal recovery/cancel plan building

Recent event windows still matter for:

- user-facing `recent_events`
- recovery fallback inference when terminal outcome persistence is missing

## Tool Behavior

### `sessions_list`

Without filters, behavior stays compatible:

- returns visible sessions
- respects `tools.sessions.visibility`
- truncates to configured/default limit

With filters:

- `state` filters by session state
- `kind` filters by session kind
- `parent_session_id` filters by direct parent
- `overdue_only` keeps only sessions whose delegate lifecycle is async and overdue
- `include_delegate_lifecycle` adds normalized lifecycle payloads to returned sessions

When `overdue_only = true`, lifecycle payloads are included automatically because the caller
selected a lifecycle-derived filter and needs the evidence.

### Response shape

Add lightweight result metadata:

- `filters`
- `matched_count`
- `returned_count`

Each returned session keeps the existing summary fields. When lifecycle output is enabled, the item
also includes:

- `delegate_lifecycle`

## Data Model

No schema change is required.

We continue using:

- `sessions`
- `session_events`
- `session_terminal_outcomes`

The new repository helper is just a narrower read model over existing `session_events`.

## Race and Consistency Semantics

- `sessions_list` is still a point-in-time sqlite read; it is not a live stream.
- If a session transitions during list generation, callers may observe slightly stale data. That is
  acceptable because control tools (`session_cancel`, `session_recover`) already enforce
  conditional state transitions.
- Lifecycle reconstruction must not depend on whether unrelated progress or telemetry events were
  recently appended.

## Testing

Add focused tests for:

- `sessions_list` filtering by `state`, `kind`, and `parent_session_id`
- `sessions_list` returning overdue delegate children when `overdue_only = true`
- `sessions_list` surfacing `delegate_lifecycle` when requested
- lifecycle reconstruction still working when recent events are noisy and no longer contain
  `delegate_queued` / `delegate_started`
- `session_status` continuing to show lifecycle data with the same noisy-event condition
