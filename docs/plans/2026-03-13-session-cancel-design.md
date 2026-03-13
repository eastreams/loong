# Session Cancel Design

## Context

LoongClaw now exposes a usable session control surface for async delegation:

- `delegate_async`
- `session_status`
- `session_events`
- `session_wait`
- `session_recover` for overdue queued async children

That gives operators observability plus one narrow recovery action, but it still leaves a control gap: once an async delegate child is visible and running, there is no first-class way to ask that child to stop.

## Problem

We want the next thick control slice without pretending LoongClaw already has a durable worker pool, cross-host task registry, or safe OS-level process identity model.

The key design question is what “cancel” should mean for a subprocess-backed async delegate child:

- do we kill the worker process immediately
- do we record intent and let the worker stop itself at a safe checkpoint
- or do we keep cancellation out of scope until a stronger worker runtime exists

## Goals

- Add a root-visible `session_cancel` tool for visible async delegate child sessions.
- Support immediate cancellation of queued async delegate children.
- Support safe best-effort cancellation of running async delegate children.
- Preserve child-session authority boundaries.
- Keep terminal state transitions race-safe and auditable.

## Non-Goals

- No durable queue or leased worker subsystem.
- No cross-host worker discovery or cancellation.
- No OS-level hard kill as the primary cancellation path in this phase.
- No new `cancelled` session state enum in this slice.
- No retries, restarts, or post-reboot worker recovery.

## Options Considered

### Option 1: Hard kill running subprocesses by PID

Pros:

- most immediate operator effect
- matches a stronger “stop now” intuition

Cons:

- current async spawn path does not persist a process handle or PID
- safe later kill requires more identity data than a bare PID to avoid reuse hazards
- pushes this phase into platform-specific process-control work before the repository has a durable worker model

Rejected for now.

### Option 2: Cooperative cancellation via session events

Record a structured cancel request in sqlite, let queued children fail immediately, and let running children observe cancellation at turn-loop checkpoints and then finalize themselves into a durable cancelled outcome.

Pros:

- reuses the existing sqlite-backed session/event model
- preserves race safety because terminal finalize still happens through the worker
- keeps cancellation explicit and auditable
- avoids unsafe or weakly identified process-kill semantics

Cons:

- cancellation is best-effort, not immediate preemption
- a child blocked inside a provider call or long tool step will only stop at the next checkpoint

Chosen.

### Option 3: No cancel until a durable worker runtime exists

Pros:

- smallest implementation risk
- avoids operator expectations that cannot be met yet

Cons:

- leaves the current session control surface materially incomplete
- forces operators to wait for timeout/recovery even when they know a child should stop

Rejected.

## Chosen Approach

Add a new root-visible session tool:

- `session_cancel`

Use two distinct execution paths:

### Queued async child

If the visible target session is:

- `kind = delegate_child`
- `state = ready`
- async
- queued
- not terminal

then `session_cancel` immediately finalizes it to terminal `failed` with:

- terminal event `delegate_cancelled`
- terminal outcome status `error`
- structured terminal payload carrying `cancel_reason = operator_requested`
- `cancel_action.kind = "queued_async_cancelled"`

This path is analogous to `session_recover`, except it is operator-driven rather than timeout-driven.

### Running async child

If the visible target session is:

- `kind = delegate_child`
- `state = running`
- async
- running
- not terminal

then `session_cancel` appends a structured `delegate_cancel_requested` event while keeping the session in `running`.

The delegated worker checks for that request at safe turn-loop checkpoints. Once observed, it exits its turn loop with a typed cancel error and finalizes the child to terminal `failed` with:

- terminal event `delegate_cancelled`
- terminal outcome status `error`
- `last_error = "delegate_cancelled: operator_requested"`

This is a cooperative, best-effort stop, not a hard process preemption.

## Tool Surface

### Root sessions

When enabled, root sessions will expose:

- `sessions_list`
- `sessions_history`
- `session_status`
- `session_events`
- `session_recover`
- `session_cancel`
- `session_wait`
- `delegate`
- `delegate_async`

### Delegated child sessions

Delegated child sessions will continue to hide:

- `session_cancel`
- `session_recover`
- `sessions_list`
- `session_events`
- `session_wait`

They keep only the current self-inspection surface.

## Data Model

No new sqlite table is required in this phase.

Cancellation state is represented through existing session/event storage:

- `delegate_cancel_requested`
- `delegate_cancelled`

Structured event payloads carry:

- `kind`
- `reference`
- `cancel_reason`
- optional operator session id in `actor_session_id`

## Execution Model

### `session_cancel`

`session_cancel` should:

1. validate target visibility
2. inspect delegate lifecycle
3. reject non-async or non-delegate-child targets
4. for queued child: conditionally finalize `ready -> failed`
5. for running child: atomically append `delegate_cancel_requested` while staying `running`
6. return a fresh session inspection payload plus a machine-readable `cancel_action`

### Delegate child worker checkpoint

At the start of each turn-loop round for a delegated child session:

1. inspect recent child-session events
2. detect whether a newer `delegate_cancel_requested` exists for the active running lifecycle
3. if cancellation is requested, stop before the next provider turn

This makes cancellation effective between provider/tool rounds while avoiding partially executed app-tool actions inside the same checkpoint boundary.

### Session inspection payload

Extend `delegate_lifecycle` for running children with optional cancellation metadata:

- `cancellation.state = "requested"`
- `cancellation.reference = "running"`
- `cancellation.requested_at`
- `cancellation.reason = "operator_requested"`

Queued cancelled children become terminal and therefore surface through existing terminal outcome plus recent events.

## Race Semantics

- If a queued child starts running before `session_cancel` finalizes it, the queued cancellation path must fail with a state-changed error.
- If a running child reaches terminal completion before it sees the cancel request, terminal completion wins.
- If a running child sees the cancel request before the next provider round, cancellation wins and finalizes the child as failed/cancelled.

This preserves “first durable terminal write wins” semantics without introducing destructive overrides.

## Testing Strategy

Add TDD coverage for:

1. root tool view/provider schema includes `session_cancel`
2. delegated child view still excludes `session_cancel`
3. `session_cancel` immediately finalizes a queued async child to failed with `delegate_cancelled`
4. `session_cancel` rejects unsupported targets such as synchronous/running non-async or non-delegate-child sessions
5. `session_cancel` records `delegate_cancel_requested` for a running async child
6. delegated child execution stops on the next checkpoint after a cancel request
7. `session_status` / `session_wait` surface cancellation metadata for running children with a pending cancel request

## Documentation Impact

Update product spec and roadmap to reflect:

- explicit `session_cancel`
- queued immediate cancel
- running cooperative cancel
- continued absence of hard kill, retry, and restart recovery semantics
