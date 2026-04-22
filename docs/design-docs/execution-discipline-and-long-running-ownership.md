# Execution Discipline And Long-Running Ownership

This document defines the next harness layer for Loong's proactive agent
behavior.

The goal is not only to stop bad behavior such as loops, hallucinated tool
results, or unsafe execution. The goal is also to make the runtime more likely
to:

- act when ambiguity is low
- retrieve missing facts before asking
- keep using tools when more evidence would materially improve correctness
- keep ownership of long-running work until a real stop condition is reached
- verify before claiming completion

## Read This Document When

- you are changing system-prompt or runtime contract assembly
- you are changing turn completion semantics for long-running work
- you are deciding where proactive behavior belongs: prompt text, runtime
  policy, or durable task state
- you are reviewing watchdog, background-task, serve-loop, or session-resume
  behavior

## Problem

Loong already has strong governance and anti-loop constraints:

- runtime-self and capability/tool guidance in the system prompt
- kernel-governed tool execution
- followup payload budgeting
- loop guards for repeated tool patterns
- safe-lane verification and session governor controls
- runtime-self continuity and checkpoint-style recovery surfaces

That is valuable, but it is not the same as an explicit proactive-completion
contract.

Today the runtime mostly answers these questions:

- how should tools be exposed and governed?
- when should repeated tool behavior be stopped?
- how should compaction and recall continuity behave?

It does not yet answer these questions strongly enough:

- when should the agent act instead of asking?
- when is missing context retrievable rather than clarification-worthy?
- when is the task still in progress even though one turn has ended?
- what durable state proves that a long-running task is active, waiting,
  verifying, blocked, or complete?

Loong therefore needs one design that keeps the following true at the same
time:

1. direct-tool and governed-tool behavior stay intact
2. prompt guidance teaches active execution discipline, not only tool shape
3. long-running ownership survives beyond one model turn
4. completion is tied to task state and verification, not merely reply emission

## Design Goals

1. Add one explicit execution-discipline contract to the runtime prompt.
2. Separate turn completion from task completion for long-running work.
3. Keep one canonical durable source of truth for task ownership and progress.
4. Reuse existing runtime seams such as `ConversationTurnCoordinator`,
   continuity, session events, and safe-lane verification.
5. Avoid introducing a parallel orchestration ecosystem outside Loong's
   existing runtime and session model.

## Core Decision 1: Add A Dedicated Execution-Discipline Prompt Fragment

Loong should add a built-in prompt fragment named `Execution Discipline`.

This fragment should be rendered by the product runtime, not loaded from
workspace `TOOLS.md` or other repo-local runtime-self files.

Reason:

- runtime-self files are useful for workspace-specific instructions
- the behavior here is product policy, not project-local advice
- this contract should stay active even when a workspace has no `TOOLS.md`

### Fragment responsibilities

The fragment should define six sections.

#### 1. `tool_persistence`

Rules:

- use tools whenever they materially improve correctness, completeness, or
  grounding
- do not stop early when another tool call would likely close an evidence gap
- if one tool returns partial or empty results, retry with a different bounded
  strategy before asking the user

#### 2. `mandatory_tool_use`

Rules:

- do not answer live system questions from memory
- do not answer file-content, git-state, or environment-state questions from
  memory
- do not answer current-fact questions from memory when configured retrieval is
  available

This is not a tool catalog. It is a policy that says when memory is
insufficient by design.

#### 3. `act_dont_ask`

Rules:

- when ambiguity does not change the next tool or runtime action, act on the
  obvious local interpretation
- ask only when ambiguity changes the required tool, target, or side effect
- small missing detail is not a reason to stop if the runtime can retrieve it

#### 4. `prerequisite_checks`

Rules:

- before a mutating or high-confidence claim, check whether discovery,
  inspection, or preflight lookup is still needed
- prerequisite steps are part of the task, not optional ceremony

#### 5. `verification`

Rules:

- before finalizing, check correctness, grounding, output shape, and stop
  conditions
- if the task was long-running, verify task state as well as content correctness
- “reply generated” is not enough evidence of completion

#### 6. `missing_context`

Rules:

- if required information is retrievable, retrieve it instead of asking
- ask only when the missing information is not locally or remotely retrievable
- if proceeding under uncertainty is unavoidable, label assumptions explicitly

### Placement

The new fragment should render after runtime-self standing instructions and
before tool-surface-specific examples.

That order keeps the prompt stack coherent:

1. workspace/runtime identity
2. execution discipline
3. tool access contract
4. capability snapshot / discovery deltas

### Integration point

The first implementation target should be
`crates/app/src/provider/request_message_runtime.rs`.

This keeps the contract near the current system-message assembly path rather
than spreading it across unrelated runtime-self loaders.

## Core Decision 2: Separate Turn Completion From Task Completion

Long-running work should not be modeled as “one turn that happened to use a
tool”.

Loong should introduce a durable task-level contract.

### New canonical concepts

#### `TaskProgress`

A durable record for one unit of owned work.

Suggested shape:

```text
TaskProgress {
  task_id
  session_id
  status
  intent_summary
  owner_kind
  active_handles[]
  resume_recipe?
  verification_state
  evidence_refs[]
  updated_at
  completed_at?
}
```

Where:

- `task_id` is the canonical operator-facing task identity
- `session_id` is the current runtime/session address that owns execution for
  that task

The model should expose both values together so task surfaces can resolve
canonical task identity to the current backing session without treating session
ids as the durable task contract.

Task-oriented list, search, status, wait, history, and control-plane task
surfaces should deduplicate by `task_id` and then expose the currently selected
`owner_session_id` / `task_session_id` as runtime metadata. When task ownership
moves between sessions, the latest durable task-progress record becomes the
canonical owner for operator-facing reads.

`task_history` should aggregate turns and task-progress events across every
visible session that currently resolves to the same canonical `task_id`, while
still marking which session is the current owner.

`task_history` should aggregate visible lineage sessions that resolve to the
same canonical `task_id`, not only the current owner session. History should
surface both the current owner and the contributing `task_session_id` entries
so operator reads remain task-first even after owner handoff.

`task_status` and `task_wait` should expose the same visible lineage session
metadata so a task handoff is explainable even when the operator is not asking
for full history.

#### `TaskStatus`

Suggested states:

- `active`
- `waiting`
- `blocked`
- `verifying`
- `completed`
- `failed`

These are task states, not model-turn states.

#### `ActiveHandle`

One structured record that explains why the task is still live.

Examples:

- long-running exec process
- ACP session
- watcher cursor
- background task lease
- serve-loop owner token

Suggested shape:

```text
ActiveHandle {
  handle_kind
  handle_id
  state
  last_event_at
  stop_condition
}
```

#### `ResumeRecipe`

A structured runtime recipe for how the task should continue.

Examples:

- wait on session events
- poll status for a specific owner token
- read recent history from a session/task log
- continue a watcher from a known cursor

This should be structured data, not a freeform shell snippet.

### Canonical rule

A task is still in progress while either of the following is true:

1. its status is `active`, `waiting`, or `verifying`
2. it has an active handle whose stop condition has not been satisfied

That rule is the long-running ownership equivalent of “do not stop just because
one reply exists”.

## Core Decision 3: One Canonical Durable Truth, Derived Delivery Layers

Mailbox, inbox, observer UI, and operator summaries are delivery or projection
layers.

They must not become competing sources of truth for assignment or progress.

Loong should keep one canonical task-progress record and derive:

- prompt reminders
- live-surface status
- operator status commands
- background-task summaries
- wait/history projections

from that one record.

That canonical record should also carry the task-to-session mapping. Operators
and higher-level tooling should address work by `task_id`, while session-shaped
runtime seams can continue to use `session_id` as the recoverable execution
address behind that canonical task identity.

The runtime should avoid assignment truth being split across:

- worker identity
- inbox content
- manifest/config metadata
- claim file / lock
- presentation summary

That split is survivable for lightweight fanout but it becomes fragile for
long-running coordination and recovery.

## Core Decision 4: Reuse Existing Loong Runtime Seams

This design should not introduce a separate orchestration subsystem.

It should reuse and extend:

- `ConversationTurnCoordinator` as the main owner of turn-to-task transitions
- runtime-self continuity for prompt continuity
- session events for durable task-progress updates
- checkpoint and repair surfaces for recovery
- safe-lane verification for high-confidence completion checks

### Why `ConversationTurnCoordinator`

The repository's active entry map already converges turn-bearing hosts on
`ConversationTurnCoordinator`.

That makes it the correct seam for:

- starting task ownership
- updating task-progress state after tool results
- attaching or clearing active handles
- entering `verifying`
- deciding whether a turn can finalize while a task remains live

## Core Decision 5: Add Operator Watch Surfaces Instead Of Mid-Turn Poking

Long-running tasks need first-class observation surfaces.

Loong should standardize on structured task/session surfaces such as:

- `status`
- `wait`
- `history`

These surfaces should be the normal way to observe long-running work after a
turn boundary.

That is better than treating every continuation as:

- re-open the full conversation blindly
- re-issue direct backend commands
- inspect internals that were not designed as operator surfaces

## Core Decision 6: Extend Verification Beyond Safe Lane

Loong already has meaningful verification machinery in safe-lane flows.

The gap is that ordinary provider lanes still skew toward:

- tool-loop prevention
- bounded retries
- reply generation

rather than:

- explicit final verification
- task-state-aware completion
- second-pass functional verification for non-trivial work

### Verification split

Loong should distinguish:

#### baseline verification

- build
- lint
- test
- schema/output checks

#### functional verification

- CLI behavior
- API behavior
- browser/UI behavior
- long-running watcher or background-task behavior

Non-trivial work should not be considered complete until the appropriate
verification layer has run or been explicitly recorded as not possible.

## Suggested Rollout

### Phase 1: Prompt contract

- add `Execution Discipline` fragment to system-message assembly
- keep behavior read-only; do not change runtime persistence yet

### Phase 2: Durable task-progress schema

- add `TaskProgress`, `TaskStatus`, `ActiveHandle`, and `ResumeRecipe`
- persist canonical `task_id -> session_id` mapping alongside task status
- persist them through session events / runtime storage

### Phase 3: Coordinator ownership

- make `ConversationTurnCoordinator` attach/update/clear task progress
- distinguish reply-finalization from task-finalization

### Phase 4: Watch/status/history surfaces

- expose canonical operator surfaces for long-running observation and resume

### Phase 5: Verification convergence

- reuse safe-lane verification semantics for ordinary long-running completion
- add second-pass functional verification policy for non-trivial work

## Non-Goals

- do not expand the visible tool surface just to simulate proactivity
- do not solve long-running ownership by only increasing turn-loop round caps
- do not push product execution policy into workspace `TOOLS.md`
- do not create a second orchestration runtime beside the main Loong runtime

## Acceptance Criteria

This design is successful when all of the following are true:

1. the system prompt explicitly teaches active execution discipline, not only
   tool vocabulary
2. the runtime can express that a task is still active after a turn reply
3. one canonical durable record explains why the task is still live
4. operator/status surfaces can wait on or inspect long-running work without
   backend poking
5. completion claims are tied to verification and task state, not only reply
   emission
