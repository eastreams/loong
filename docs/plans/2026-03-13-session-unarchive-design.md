# Session Unarchive Design

## Context

LoongClaw now has a truthful `session_archive` primitive for inventory cleanup of visible
terminal sessions. That solved the first half of the operator lifecycle: a finished session can be
retired from default `sessions_list` inventory without deleting history or pretending to close a
live route.

The next honest gap is reversibility. Once a session is archived, an operator can still inspect it
directly, but there is no durable tool to restore it back into default inventory. That creates an
asymmetry:

- `session_archive` is explicit and auditable
- rediscovery is possible through direct inspection or `include_archived=true`
- restoration back into the default list is not yet explicit or auditable

This is a better target than a fake `session_close`. External comparison showed that OpenClaw does
not provide a meaningful reopen path after close/delete, and LoongClaw still lacks truthful
route-unbinding or root-session successor semantics for fixed ids like `telegram:<chat_id>` and
`feishu:<chat_id>`.

## Problem

Operators can archive visible terminal sessions, but cannot later restore them into default
inventory without manual rediscovery workarounds. The current archive-state derivation also only
looks for `session_archived`, which is sufficient for archive-only behavior but becomes incorrect as
soon as a reversible lifecycle exists.

## Goals

- Add a truthful `session_unarchive` operator primitive for visible archived terminal sessions.
- Preserve the event-sourced audit trail instead of replacing archive state with mutable flags.
- Make archive state derivation correct under both archive and unarchive actions.
- Reuse the existing single-target and batch mutation patterns already used by
  `session_archive`, `session_cancel`, and `session_recover`.
- Keep archived sessions directly inspectable before and after restoration.

## Non-Goals

- No fake `session_close`, route shutdown, or inbound channel unbinding.
- No reopen / successor-session model for fixed channel-backed root ids.
- No transcript deletion, rewriting, or terminal outcome mutation.
- No mutation of non-terminal sessions.
- No hidden side effects beyond inventory visibility.

## Chosen Primitive

Add `session_unarchive`.

`session_unarchive` restores a visible archived terminal session back into the default
`sessions_list` inventory. It does not change execution state, transcript rows, or terminal
outcomes.

This keeps the tool truthful:

- `session_archive` hides a finished session from default inventory
- `session_unarchive` restores that finished session to default inventory
- neither tool claims to shut down runtime routing or reopen live execution

## Scope Rules

`session_unarchive` is allowed only when all of the following are true:

- target session is visible from the caller under existing visibility rules
- target session is currently archived
- target session is terminal: `completed`, `failed`, or `timed_out`

The terminal-state requirement keeps archive and unarchive as pure inventory hygiene over finished
work, rather than broad session-lifecycle control.

## State Model

Archive is still not a new execution state. Session execution state remains:

- `ready`
- `running`
- `completed`
- `failed`
- `timed_out`

Archive is an inventory overlay represented on summaries as:

- `archived: boolean`
- `archived_at: integer|null`

The key refinement is how that overlay is derived.

## Persistence Model

Archive lifecycle remains event-sourced:

- `session_archive` appends `session_archived`
- `session_unarchive` appends `session_unarchived`

The repository must no longer ask only "has this session ever been archived?" Instead it must look
at the most recent archive control event for the session and derive state from that event:

- latest control event = `session_archived` => session is archived, `archived_at = event.ts`
- latest control event = `session_unarchived` => session is not archived, `archived_at = null`
- no archive control event => session is not archived, `archived_at = null`

To avoid incorrect ties from second-granularity timestamps, "latest" should be resolved primarily by
session-event row `id`, not only by `ts`.

## Repository Semantics

Both visible-session listing and single-session summary loading must use the same archive-state
derivation so the tool layer does not drift from inventory behavior.

Recommended query shape:

- restrict to archive-control events: `session_archived`, `session_unarchived`
- pick the latest control event per session using descending event `id`
- project:
  - archived flag from latest event kind
  - `archived_at` only when latest event kind is `session_archived`

This avoids introducing a mutable `sessions.archived_at` column and preserves the full audit trail.

## Tool Semantics

### Single-target mode

Request:

```json
{
  "session_id": "delegate:child-123"
}
```

Response shape follows the existing single-target mutation pattern:

- returns the normal inspection payload
- adds `unarchive_action`

Representative action payload:

```json
{
  "kind": "session_unarchived",
  "previous_state": "completed",
  "next_state": "completed",
  "restores_to_sessions_list": true
}
```

### Batch mode

Request:

```json
{
  "session_ids": ["delegate:child-1", "delegate:child-2"],
  "dry_run": true
}
```

Batch result classifications:

- `would_apply`
- `applied`
- `skipped_not_visible`
- `skipped_not_archived`
- `skipped_not_archivable`
- `skipped_state_changed`

`skipped_not_archived` is separate from `skipped_not_archivable` so operators can distinguish "not
currently archived" from "wrong lifecycle state".

## Inspection And Listing Semantics

`session_status`, `session_events`, and `sessions_history` remain available for archived sessions and
for later-unarchived sessions. `session_events` naturally exposes both `session_archived` and
`session_unarchived`.

`sessions_list` behavior becomes:

- archived sessions are still excluded by default
- `include_archived=true` returns both archived and non-archived visible sessions
- after `session_unarchive`, the target reappears in default `sessions_list`

`session_wait` remains unchanged because archive lifecycle does not alter terminality.

## Error Model

Representative rejections:

- `session_unarchive_not_archivable: session \`...\` is not terminal`
- `session_unarchive_not_unarchivable: session \`...\` is not archived`
- `visibility_denied: ...`
- `session_unarchive_state_changed: session \`...\` is no longer unarchivable from state \`...\``

The `state_changed` branch covers races where another actor archives, unarchives, or otherwise
mutates the target after inspection.

## Alternatives Considered

### 1. Mutable `sessions.archived_at` field

Pros:

- simpler query path

Cons:

- weaker audit trail
- duplicates information already present in durable events
- makes archive lifecycle less transparent in `session_events`

Rejected because LoongClaw's session surface is explicitly trying to stay auditable and truthful.

### 2. List-layer filtering without durable unarchive state

Pros:

- smaller change set

Cons:

- inventory behavior can drift from inspection behavior
- no durable record of restoration
- weak operator evidence for who restored a session and when

Rejected because it is not robust enough for a control-plane primitive.

## Why This Design

This is the smallest reversible lifecycle that stays honest:

- it deepens the existing archive primitive instead of widening the surface with a fake close
- it keeps state event-sourced and auditable
- it fixes the underlying repository model rather than papering over it in the tool layer
- it gives operators a complete archive / restore inventory workflow without claiming runtime
  semantics LoongClaw does not yet have
