# Session Archive Design

## Context

LoongClaw's current session surface is strong on truthful execution control for delegate children:

- inspect: `sessions_list`, `sessions_history`, `session_status`, `session_events`, `session_wait`
- mutate: `session_cancel`, `session_recover`
- outbound operator messaging: `sessions_send`

The remaining gap is not a fake `close` primitive. External comparison showed that OpenClaw's `close` is tied to real ACP runtime shutdown and route unbinding. LoongClaw does not yet have equivalent live runtime bindings, root-session reopening semantics, or a session epoch model for channel-backed roots. Adding a `session_close` name right now would either:

1. lie by only adding a label without changing routing, or
2. break inbound channel routing for fixed ids like `telegram:<chat_id>` / `feishu:<chat_id>`.

What LoongClaw can truthfully support today is archive semantics for already-terminal visible sessions.

## Problem

Root sessions can accumulate many completed, failed, or timed-out delegate children. `sessions_list` currently keeps returning those terminal children forever unless the caller manually filters by state every time. That creates inventory noise without providing an explicit operator action to retire finished work.

## Goals

- Add a truthful operator primitive to retire finished visible sessions from default inventory.
- Keep direct inspection possible after archival.
- Preserve transcript rows and terminal outcomes exactly as they are.
- Reuse the existing single-target and batch mutation patterns (`session_cancel` / `session_recover`).
- Avoid introducing fake runtime-close behavior.

## Non-Goals

- No root-session routing shutdown or channel unbinding.
- No hard session deletion.
- No transcript truncation or memory clearing.
- No hiding of running or ready sessions.
- No automatic reopen / successor-session model.
- No unarchive tool in this phase.

## Chosen Primitive

Add `session_archive`.

`session_archive` archives a visible terminal session so it no longer appears in `sessions_list` by default. The archival is durable and auditable, but does not remove any historical data.

This is intentionally narrower than `close`:

- `session_cancel` changes an in-flight delegate lifecycle.
- `session_recover` marks an overdue async delegate as failed.
- `session_archive` changes only session inventory visibility for already-terminal sessions.

## Scope Rules

`session_archive` is allowed only when all of the following are true:

- target session is visible from the caller under existing visibility rules
- target session is already terminal: `completed`, `failed`, or `timed_out`
- target session is not already archived

Practical effect in today's architecture:

- terminal delegate children are archivable
- non-terminal root sessions are rejected because they are not terminal

This keeps the primitive truthful and avoids pretending to close active channel-backed roots.

## State Model

Archive is not a new execution state. Session execution state remains:

- `ready`
- `running`
- `completed`
- `failed`
- `timed_out`

Archive is a separate inventory overlay represented as archive metadata on session summaries:

- `archived: boolean`
- `archived_at: integer|null`

This avoids conflating execution lifecycle with operator inventory hygiene.

## Persistence Model

Archive state is persisted via control-plane metadata rather than transcript mutation:

- append a durable `session_archived` session event
- derive `archived_at` from archive metadata when loading session summaries

Consequences:

- transcript rows stay unchanged
- terminal outcome rows stay unchanged
- `session_events` naturally exposes the archive action
- `session_status` can report that a session is archived

## Tool Semantics

### Single-target mode

Request:

```json
{
  "session_id": "delegate:child-123"
}
```

Response shape follows the existing single-target mutation pattern:

- returns normal inspection payload
- adds `archive_action`

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
- `skipped_not_archivable`
- `skipped_already_archived`
- `skipped_state_changed`

## Listing Semantics

`sessions_list` changes in one important way:

- default behavior excludes archived visible sessions
- `include_archived=true` returns both archived and non-archived visible sessions

Returned session summaries include:

- `archived`
- `archived_at`

This makes archive state explicit instead of silently hiding data.

## Inspection Semantics

`session_status` remains available for archived visible sessions and includes archive metadata.

`sessions_history` remains available for archived visible sessions.

`session_events` remains available for archived visible sessions and includes the `session_archived` event.

`session_wait` is unchanged. Waiting on a terminal archived session still returns completed state immediately because archive does not alter terminality.

## Error Model

Representative rejections:

- `session_archive_not_archivable: session \`...\` is not terminal`
- `session_archive_not_archivable: session \`...\` is already archived`
- `visibility_denied: ...`
- `session_archive_state_changed: session \`...\` is no longer archivable`

The `state_changed` branch covers races where another actor archives or mutates the target after inspection.

## Why This Design

This is the smallest truthful primitive that closes the current operator gap:

- it solves real inventory clutter for child-session orchestration
- it preserves LoongClaw's auditable session model
- it does not claim capabilities the runtime does not yet possess

If LoongClaw later grows real route-bound root sessions with explicit reopen / successor semantics, a future `session_close` can be added honestly on top of that stronger substrate.
