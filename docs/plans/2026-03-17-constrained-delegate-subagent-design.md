# Constrained Delegate Subagent Design

**Problem**

`alpha-test` already has the beginnings of delegated child-session execution:

- `delegate` and `delegate_async` create child sessions and persist lifecycle events
- the conversation runtime exposes `prepare_subagent_spawn(...)` and `on_subagent_ended(...)`
- session inspection can infer queued/running delegate lifecycle from persisted events

But those pieces do not yet compose into a coherent constrained subagent primitive.

Today the production delegate path still behaves like an ad hoc child-session launcher:

- context-engine subagent hooks are never called from the real delegate execution path
- lifecycle payloads are free-form JSON fragments with no typed execution envelope
- there is a depth limit but no concurrent active-child limit
- session inspection can only reconstruct a partial lifecycle view from sparse event fields

That leaves a gap between LoongClaw's reserved kernel/context seams and an actually governed delegate runtime.

**Goal**

Advance `delegate` and `delegate_async` from "persisted child-session launch" toward a first-class constrained subagent primitive without widening this slice into a full OpenClaw-style subagent subsystem.

The bounded target for this slice is:

1. Wire the existing runtime/context-engine subagent lifecycle hooks into the real delegate path.
2. Replace ad hoc lifecycle payload fragments with a typed constrained-subagent execution envelope and terminal taxonomy.
3. Add a direct-child concurrency guard so one parent session cannot fan out unbounded active delegates.
4. Surface the new envelope in `session_status` / delegate lifecycle inspection.

**Non-Goals**

Do not:

- add a new ACP control-plane surface for delegate children in this slice
- redesign session visibility, orphan recovery, or delivery hooks
- add general descendant-tree scheduling or cross-parent global delegate quotas
- widen delegate into an arbitrary custom-agent registry

**Constraints**

- Keep the existing `delegate` / `delegate_async` tool names and request schema stable.
- Preserve the current child-tool allowlist model and shell gating model.
- Preserve the current session persistence model built on `sessions`, `session_events`, and `session_terminal_outcomes`.
- Keep the implementation additive and local to the conversation/session/tooling seam.

**Root Cause**

The root problem is not that delegate lacks more knobs. The root problem is that the current runtime has no explicit contract object for "a constrained subagent execution" even though multiple subsystems already need one:

- the conversation runtime needs a stable spawn/end lifecycle shape
- the context engine needs a governed seam for kernel-bound child sessions
- the session inspection tooling needs more than a timeout number to explain what constraints governed the child

Because the contract is missing, the current code duplicates just enough JSON to make the happy path work and silently leaves the reserved lifecycle hooks unused.

**Approaches Considered**

1. Keep the current event payloads and only add hook calls.
   Rejected because it would fix one symptom while leaving the execution contract implicit. Session inspection would still need to reverse-engineer child constraints from loosely shaped payloads.

2. Build a full OpenClaw-style subagent runtime with its own registry, outcomes, queue lane, announce chain, and orphan recovery semantics.
   Rejected for this slice because it is too large for the actual gap on `alpha-test`. The current repository already has child-session persistence and recovery. The missing value is governance clarity, not a second orchestration subsystem.

3. Introduce a typed constrained-subagent envelope at the existing delegate seam, wire lifecycle hooks, and add a direct-child active limit.
   Recommended because it addresses the architectural gap with the smallest durable change set and keeps the implementation aligned with current LoongClaw structure.

**Chosen Design**

Add a small typed contract module in the conversation layer to describe constrained delegate execution.

The contract will include:

- execution mode: `inline` or `async`
- parent/child lineage depth snapshot:
  - `depth`
  - `max_depth`
- direct active-child budget snapshot:
  - `active_children`
  - `max_active_children`
- execution constraints:
  - `timeout_seconds`
  - `allow_shell_in_child`
  - `child_tool_allowlist`
- terminal reason taxonomy:
  - `completed`
  - `failed`
  - `timed_out`
  - `spawn_failed`

This envelope will be serialized into delegate lifecycle events instead of the current hand-built partial payloads.

**Config Change**

Add `tools.delegate.max_active_children` with a conservative default.

This is intentionally a direct-child limit, not a descendant-tree limit. Direct children are the parent-controlled concurrency surface at the current delegate seam. Descendant-tree budgeting can be added later if the design expands into a broader scheduler.

**Runtime Behavior**

For both `delegate` and `delegate_async`:

1. Parse the delegate request.
2. Resolve the parent's current lineage depth.
3. Count the parent's active direct child sessions (`ready` + `running`).
4. Enforce:
   - `next_depth <= max_depth`
   - `active_children < max_active_children`
5. Build a constrained-subagent envelope.
6. If the parent turn is kernel-bound, call `runtime.prepare_subagent_spawn(...)` before creating the child session.
7. Persist child creation / transition events with the typed envelope embedded.

For terminal completion paths:

- persist terminal state and terminal outcome as before
- include typed terminal reason metadata in the terminal event payload
- if the child ran under a kernel-bound parent binding, call `runtime.on_subagent_ended(...)` after terminal persistence

**Hook Failure Policy**

`prepare_subagent_spawn(...)` is fail-closed.

Rationale:
- it runs before child session creation
- a failure means the context engine could not prepare the governed subagent seam
- failing before child creation avoids half-created child sessions

`on_subagent_ended(...)` is also treated as part of the governed lifecycle contract for synchronous delegate execution.

Rationale:
- for kernel-bound execution, a child is not fully reconciled until the context engine has observed completion
- returning a visible error is preferable to silently pretending the governed lifecycle fully succeeded

For async children the end hook still runs in the spawned task path. This slice does not add a separate async hook-failure replay channel; it keeps the bounded scope focused on wiring the lifecycle seam and exposing typed execution metadata. If later evidence shows async post-terminal hook failures need their own persisted recovery event, that should be a follow-up slice.

**Session Inspection Changes**

`session_status` and related delegate lifecycle inspection will stop relying only on:

- inferred queued/started timestamps
- timeout numbers

and will additionally surface the constrained-subagent envelope, so inspection can answer:

- was this child inline or async?
- what depth budget governed it?
- how many active siblings existed when it was launched?
- what child-tool constraints applied?

This should be sourced from the earliest available lifecycle event carrying the typed envelope, rather than recomputing it later from mutable config.

**Why This Is The Smallest Durable Slice**

This change intentionally does not attempt to normalize LoongClaw into OpenClaw's complete subagent system. OpenClaw's design includes dedicated session lanes, orphan recovery machinery, delivery hooks, and registry-level governance. LoongClaw already has a narrower but functional child-session substrate.

The minimum architectural debt payoff here is:

- make the existing reserved lifecycle seam real
- give delegate execution a first-class contract object
- cap parent fan-out at runtime instead of only at nesting depth

That yields durable structure without adding a second orchestration model beside the one the repo already uses.

**Testing Strategy**

Add failing tests first for:

- kernel-bound `delegate` calling `prepare_subagent_spawn(...)` before child execution
- terminal child completion calling `on_subagent_ended(...)`
- direct (non-kernel) delegate execution still working without lifecycle hooks
- `delegate_async` refusing new children when direct active-child budget is exhausted
- `session_status` exposing the typed constrained-subagent envelope from delegate lifecycle events
- deterministic rejection when spawn preparation fails before child session creation

Then verify adjacent delegate/session regressions and full repository validation.

**Risk Assessment**

The highest risk is not functional breakage. It is semantic drift between:

- the new typed envelope
- the session-inspection view
- the actual runtime constraints

That risk is controlled by using one shared typed contract and by persisting the runtime snapshot into lifecycle events at creation/transition time.

The second risk is accidentally widening the scope into a new general subagent framework. This design deliberately avoids that by staying on the current delegate/session seam.
