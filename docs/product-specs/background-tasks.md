# Background Tasks

This document remains the product contract for the task-shaped async delegate
surface itself.

Scheduled and event-driven automation now live beside it, not inside it. See
[`../design-docs/automation-triggering.md`](../design-docs/automation-triggering.md)
for the newer trigger model that can queue these background tasks.

## User Story

As a Loong operator, I want a task-shaped background work surface so that I
can launch, inspect, wait on, and control delegated async work without having
to reason directly in raw session-runtime terms.

## Acceptance Criteria

- [ ] Loong exposes a task-shaped operator surface for background delegated
      work rather than requiring the operator to compose raw `delegate_async`
      and `session_*` calls manually.
- [ ] The first slice supports:
      create, list, inspect status, wait or follow, cancel, and recover for
      visible background tasks.
- [ ] Task output surfaces approval-pending, blocked, failed, and recovered
      states explicitly.
- [ ] Task output surfaces any session-scoped tool narrowing that materially
      affects what the delegated child may do.
- [ ] `tasks create`, `tasks list`, `tasks status`, and `tasks wait` expose a
      derived `task_status.status`, `task_status.needs_attention`, and
      `task_status.next_action` summary instead of leaving operators to infer
      meaning only from raw session state and delegate phase.
- [ ] The task surface remains truthful to the current runtime:
      background tasks are implemented as child sessions rather than a parallel
      scheduler-specific state model.
- [ ] Product docs clearly distinguish this first-slice background task surface
      from future cron, heartbeat, or always-on daemon scheduling work.

## Current Baseline

The current runtime already ships the substrate for this surface:

- `delegate_async`
- `session_status`
- `session_wait`
- `session_events`
- `session_cancel`
- `session_recover`
- approval request tooling
- session-scoped tool policy controls

The missing part is the operator-facing product contract that turns those
session primitives into a coherent task workflow.

## Out of Scope For This Task Surface

- cron
- heartbeat jobs
- daemon ownership and service installation
- distributed scheduling
- Web UI task dashboards
