# Background Tasks

## User Story

As a Loong operator, I want a task-shaped background work surface so that I
can launch, inspect, wait on, and control delegated async work without having
to reason directly in raw session-runtime terms.

## Acceptance Criteria

- [x] Loong exposes a task-shaped operator surface for background delegated
      work rather than requiring the operator to compose raw `delegate_async`
      and `session_*` calls manually.
- [x] The first slice supports:
      create, list, inspect status, wait or follow, cancel, and recover for
      visible background tasks.
- [x] Task output surfaces approval-pending, blocked, failed, and recovered
      states explicitly.
- [x] Task output surfaces any session-scoped tool narrowing that materially
      affects what the delegated child may do.
- [x] `tasks create`, `tasks list`, `tasks status`, and `tasks wait` expose a
      derived `task_status.status`, `task_status.needs_attention`, and
      `task_status.next_action` summary instead of leaving operators to infer
      meaning only from raw session state and delegate phase.
- [x] The task surface remains truthful to the current runtime:
      background tasks are implemented as child sessions rather than a parallel
      scheduler-specific state model.
- [ ] Product docs clearly distinguish this first-slice background task surface
      from future cron, heartbeat, or always-on daemon scheduling work.

## Current Contract

The current runtime ships a task-shaped operator contract on top of the existing
async delegate substrate:

- `tasks create`
- `tasks list`
- `tasks status`
- `tasks events`
- `tasks wait`
- `tasks cancel`
- `tasks recover`

The contract is task-first:

- `task_id` is the canonical task identity
- `task_session_id` is the explicit runtime carrier for the current task lane
- `owner_session_id` remains visible as runtime metadata where ownership and
  lineage matter
- `task_status` is the primary operator summary, not raw `session.state`

The runtime substrate remains:

- `delegate_async`
- `session_status`
- `session_wait`
- `session_events`
- `session_cancel`
- `session_recover`
- approval request tooling
- session-scoped tool policy controls

This means Loong keeps one truthful model:

- background tasks are still child sessions at runtime
- operators should primarily reason in task terms
- session-oriented surfaces remain diagnostics and repair tools, not the primary
  day-to-day contract for delegated background work

## Remaining Product Work

- keep tightening task identity so no public task-facing surface relies on
  `session_id` as the primary visible identifier
- keep aligning control-plane and other local product surfaces to the same
  task-first wording and field contract
- keep `tasks` as the primary operator path and `sessions` as the runtime
  inspection path

## Out of Scope

- cron
- heartbeat jobs
- daemon ownership and service installation
- distributed scheduling
- Web UI task dashboards
