# Background Tasks

## User Story

As a LoongClaw operator, I want a task-shaped background work surface so that I
can launch, inspect, wait on, and control delegated async work without having
to reason directly in raw session-runtime terms.

## Acceptance Criteria

- [x] LoongClaw exposes a task-shaped operator surface for background delegated
      work rather than requiring the operator to compose raw `delegate_async`
      and `session_*` calls manually.
- [x] The current shipped slice supports:
      create, list, inspect status, inspect events, wait, cancel, and recover
      for visible background tasks.
- [x] Task output surfaces lifecycle state, approval attention, and effective
      runtime narrowing for the delegated child.
- [x] The task surface remains truthful to the current runtime:
      background tasks are implemented as child sessions rather than a parallel
      scheduler-specific state model.
- [ ] When the broader workflow surface lands, background tasks surface parent
      workflow phase, lineage, and bound worktree or artifact references when
      that metadata exists.
- [ ] Product docs clearly distinguish this task surface from future cron,
      heartbeat, or always-on daemon scheduling work.

## Current Baseline

The shipped runtime already provides both substrate and a first operator-facing
translation layer:

- `delegate_async`
- `session_status`
- `session_wait`
- `session_events`
- `session_cancel`
- `session_recover`
- approval request tooling
- session-scoped tool policy controls
- `loong tasks create|list|status|events|wait|cancel|recover`

The remaining gap is broader workflow productization: phase-aware workflow
inspection, bound worktree or artifact identity, and control-plane parity.

## Out of Scope

- cron
- heartbeat jobs
- daemon ownership and service installation
- distributed scheduling
- Web UI task dashboards
