# Session Recover Running Delegate Design

## Context

LoongClaw already gives operators two narrow controls over async delegate children:

- `session_cancel` for queued cancellation and cooperative running cancellation
- `session_recover` for overdue queued async children still stuck in `ready`

That still leaves one operational hole: a visible async delegate child can remain in `running`
past its timeout and still be unrecoverable if the worker is wedged, lost, or never comes back
to observe a cooperative cancel request.

## Problem

We need one more thick control slice that helps operators resolve stale `running` async delegate
children without pretending LoongClaw already has durable worker identity, safe process kill, or a
restart-aware worker registry.

The key design question is what `session_recover` should do when an async delegate child is:

- visible
- still `running`
- clearly overdue from its lifecycle metadata
- not terminal yet

## Goals

- Extend `session_recover` to handle overdue `running` async delegate children.
- Keep recovery auditable and race-safe through existing sqlite session state.
- Preserve child authority boundaries.
- Distinguish queued-overdue and running-overdue recovery in machine-readable metadata.
- Preserve fallback recovery inference when structured recovery event persistence fails.

## Non-Goals

- No hard process kill or PID-based worker targeting.
- No automatic restart recovery daemon or lease manager.
- No new session state enum for "cancelled" or "recovered".
- No broad retry queue, requeue, or worker resurrection semantics.
- No change to child-visible tool authority.

## Options Considered

### Option 1: Keep `session_recover` queued-only

Pros:

- zero new semantics
- smallest implementation delta

Cons:

- operators cannot clear a visibly stale `running` child when cooperative cancel cannot progress
- keeps the session control surface incomplete in real failure cases

Rejected.

### Option 2: Extend `session_recover` to finalize overdue running async children as failed

Use the same race-safe conditional terminal finalize pattern already used for queued recovery, but
allow `expected_state = running` when lifecycle metadata says the async child is overdue in its
running phase.

Pros:

- reuses the current sqlite session/event model
- fits the existing operator-driven remediation tool surface
- avoids unsafe process-level control
- gives a deterministic remediation path for wedged or orphaned workers

Cons:

- if the worker is still alive, the operator can mark the session failed before the worker notices
- recovery remains an operator decision, not an automatic policy

Chosen.

### Option 3: Add process-aware hard recovery

Pros:

- more direct operator semantics

Cons:

- requires stronger worker identity and ownership semantics than the current subprocess model has
- expands this slice into platform/process management rather than session correctness

Rejected for now.

## Chosen Approach

Extend `session_recover` so it accepts two recoverable delegate lifecycle shapes:

### Queued async overdue child

Existing behavior remains unchanged:

- `state = ready`
- lifecycle `mode = async`
- lifecycle `phase = queued`
- lifecycle `staleness.state = overdue`

Recovery finalizes `ready -> failed` with recovery kind
`queued_async_overdue_marked_failed`.

### Running async overdue child

New behavior:

- `state = running`
- lifecycle `mode = async`
- lifecycle `phase = running`
- lifecycle `staleness.state = overdue`

Recovery finalizes `running -> failed` with recovery kind
`running_async_overdue_marked_failed`.

The recovery event stays `delegate_recovery_applied`, but the structured payload and `last_error`
prefix distinguish queued vs running remediation.

## Data Model

No schema change is required.

We continue using:

- `sessions.state`
- `sessions.last_error`
- `session_events`
- `session_terminal_outcomes`

Add one new recovery classification:

- `running_async_overdue_marked_failed`

Add one new `last_error` prefix for fallback inference:

- `delegate_async_running_overdue_marked_failed:`

## Payload Shape

`session_recover` continues returning a fresh inspection payload plus `recovery_action`.

For running overdue recovery:

- `recovery_action.kind = "running_async_overdue_marked_failed"`
- `recovery_action.previous_state = "running"`
- `recovery_action.next_state = "failed"`
- `recovery_action.reference = "started"` when a `delegate_started` timestamp exists
- the timeout / elapsed / deadline fields mirror lifecycle staleness

Queued recovery keeps its existing payload shape.

## Race Semantics

- If the target session becomes terminal before recovery finalizes it, terminal state wins and
  `session_recover` returns a state-changed error.
- If a `running` child transitions away from `running` before the operator recovery writes, the
  conditional finalize is a no-op and the caller gets a state-changed error.
- If a stale worker later tries to finalize after operator recovery already marked the session
  failed, repository guards must continue preventing double terminalization.

## Inspection Semantics

`session_status` and `session_wait` already prefer structured recovery events and fall back to
`last_error` prefixes when the recovery event is missing. This slice extends that fallback parser so
running-overdue recovery remains attributable even if event persistence fails.

## Testing

Add focused tests for:

- `session_recover` marking an overdue running async child failed
- `session_recover` still rejecting fresh running children
- recovery kind fallback inference from the new running-overdue `last_error` prefix
- optional session inspection coverage to ensure recovered-running sessions surface the new recovery
  kind through existing inspection payloads
