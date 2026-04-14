# Governed Workflows

## User Story

As a LoongClaw operator, I want long-running work to run through one truthful
workflow model so that planning, execution, verification, recovery, and future
browser or control-plane surfaces all describe the same runtime reality.

## Acceptance Criteria

- [ ] LoongClaw exposes one operator-facing workflow surface above the current
      session runtime instead of requiring prompt-only conventions or raw
      session juggling.
- [ ] The workflow surface uses an explicit phase model for:
      `plan`, `spec`, `execute`, `verify`, `fix`, plus terminal states such as
      `complete`, `failed`, and `cancelled`.
- [ ] Workflow state remains truthful to the current runtime:
      delegated child work still runs as child sessions, and workflow state does
      not introduce a second scheduler-specific truth model.
- [ ] Workflow inspection can surface task lineage, bound session identity,
      worktree or workspace binding, and durable artifact references without
      making tmux, HUD, or another adapter the semantic owner.
- [ ] Background task operations fit inside the workflow model rather than
      becoming a separate parallel orchestration system.
- [ ] Runtime-self continuity, session profile, and durable memory remain
      advisory continuity lanes rather than becoming workflow-owned identity
      authority.
- [ ] CLI, local control-plane, and future browser surfaces consume the same
      workflow model instead of inventing surface-local workflow ids or phase
      semantics.

## Current Baseline

The current runtime already ships important workflow substrate:

- typed governed workflow contracts such as
  `GovernedSessionBindingDescriptor`, `WorkflowOperationKind`,
  `WorkflowOperationScope`, and `WorktreeBindingDescriptor`
- session inspection that already surfaces workflow metadata for delegate child
  sessions, including task text, lineage root, lineage depth, and
  runtime-self continuity summary
- a task-shaped background-task CLI on top of the existing session runtime
- explicit runtime-self continuity boundaries and query-aware memory retrieval
- a localhost-only product control plane for session, approval, and turn
  observation

The missing part is the product layer that turns those substrate pieces into one
coherent workflow contract.

## Out of Scope

- introducing a second async scheduler beside child-session execution
- making tmux or any UI adapter the canonical owner of workflow truth
- mandatory multi-worker team orchestration in the first slice
- hosted or public workflow control planes by default
- letting workflow state override runtime-self identity or policy authority
