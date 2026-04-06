# Agent Role Resolution Slice Design

**Problem**

`#970` asks for an agent-role system with depth-aware resolution and TOML layering, but the
current branch only has fragments of that model:

- `DelegateBuiltinProfile` (`research`, `plan`, `verify`) shapes child policy presets in
  `crates/app/src/tools/delegate.rs`
- `ConstrainedSubagentProfile` derives orchestration posture (`orchestrator` vs `leaf`) from
  depth in `crates/app/src/conversation/subagent.rs`
- child execution metadata is already persisted and surfaced through constrained-subagent contract
  views, session inspection, and delegate lifecycle events

Those pieces work, but they still blur two different concerns:

- what kind of child the system meant to create
- what orchestration rights the child has because of lineage depth

Today the runtime still uses delegate builtin profiles as the closest thing to a semantic child
identity. That keeps the child model implicit and makes the next step, config-backed role overlays,
harder than it should be.

**Goal**

Introduce an explicit internal `AgentRole` model as the first bounded slice of `#970`.

This slice should:

1. Add a typed `AgentRole` enum for semantic child identity.
2. Resolve that role from the existing delegate builtin profile and depth context.
3. Persist and surface the resolved role through constrained-subagent execution metadata and
   session inspection.
4. Keep current delegate request schema and current enforcement knobs stable.

The result should be one shared role field that prompt/runtime/session surfaces can rely on without
re-inferring child intent from `DelegateBuiltinProfile`.

**Non-Goals**

Do not:

- add TOML `[[roles]]` overlays in this first slice
- replace `DelegateBuiltinProfile` as the public delegate request surface
- redesign child-tool allowlists, shell gating, or runtime narrowing semantics
- change the depth-limit and active-child enforcement model
- widen this work into mailbox, batch delegation, spawn reservations, or descendant scheduling

**Root Cause**

The current implementation already separates some child-session concerns, but not the important
one for `#970`.

It already has:

- a public launch preset (`DelegateBuiltinProfile`)
- an execution envelope (`ConstrainedSubagentExecution`)
- an orchestration posture (`ConstrainedSubagentProfile`)

What it does not have is a distinct semantic role layer between “requested preset” and “resolved
child contract”.

That gap causes two problems:

1. Builtin profiles do double duty as both user-facing request values and de facto internal role
   identifiers.
2. Depth-derived `orchestrator` / `leaf` posture looks like a role even though it is really an
   orchestration-rights decision.

Without an explicit internal role, config overlays would have no clear merge target and
observability would continue depending on preset-specific inference.

**Approaches Considered**

1. Implement the full issue in one pass: internal role model, depth-aware resolution, TOML
   overlays, and enforcement migration.
   Rejected for this slice because it mixes naming cleanup, config surface growth, and policy
   behavior changes into one refactor.

2. Add only `AgentRole`, resolve it centrally, persist it, and keep existing enforcement knobs in
   place.
   Recommended because it creates the missing semantic layer with the smallest durable change set.

3. Skip `AgentRole` and document `DelegateBuiltinProfile` as the role system.
   Rejected because it preserves the existing ambiguity between request presets and resolved child
   identity.

**Chosen Design**

Add a new internal enum in the conversation layer:

- `default`
- `explorer`
- `worker`
- `verifier`

The role model is semantic. It answers “what kind of child is this?” It does not answer whether the
child may delegate further.

Role resolution rules for this slice:

- root runtime context with no delegate profile projects `default`
- `research` resolves to `explorer`
- `plan` resolves to `worker`
- `verify` resolves to `verifier`

Depth remains a separate concern:

- `ConstrainedSubagentProfile` continues to express orchestration posture:
  - `orchestrator`
  - `leaf`
- hitting `max_depth` only degrades orchestration posture to `leaf`
- hitting `max_depth` does **not** silently rewrite `explorer`, `worker`, or `verifier` into some
  other semantic role

This keeps semantic identity stable while preserving current lineage-based control behavior.

**Execution Contract Changes**

Extend constrained-subagent execution metadata to carry the resolved role.

The new role should be stored in:

- `ConstrainedSubagentExecution`
- `ConstrainedSubagentContractView`
- delegate lifecycle payloads that already serialize the execution envelope

This means child-session observability will no longer need to infer semantic identity from:

- `DelegateBuiltinProfile`
- prompt text fragments
- depth-derived orchestration posture

Instead, the system will persist one resolved role value at child creation time and then reuse it in:

- session inspection
- runtime session context
- prompt/runtime guidance layers that need to describe the child

**Public API Boundary**

Keep `DelegateBuiltinProfile` as the public tool-facing request schema in `delegate` and
`delegate_async`.

Why this boundary is correct for the first slice:

- it avoids breaking existing prompts and tool requests
- it keeps current preset behavior stable
- it gives the runtime an internal normalization target without changing the external contract

This means the delegate tools still accept:

- `research`
- `plan`
- `verify`

but child execution contracts and downstream consumers can use `AgentRole` as the internal
semantic identity.

**Behavior Boundary**

This slice is intentionally additive.

Current enforcement still comes from the resolved delegate policy:

- timeout
- child tool allowlist
- shell allowance
- runtime narrowing

The role system in this slice does not yet become the enforcement source of truth.

Instead, it provides:

- explicit semantic identity now
- one stable merge point for future TOML role overlays
- one stable observability field for session/runtime surfaces

That keeps the first implementation slice small and reduces the risk of subtle behavior drift.

**Runtime And Inspection Integration**

Use role resolution when building child execution envelopes in the delegate spawn path.

The expected flow becomes:

1. Parse delegate request.
2. Resolve existing delegate policy from the builtin preset.
3. Resolve child orchestration posture from depth.
4. Resolve child semantic role from the builtin preset and depth context.
5. Persist both the orchestration posture and the semantic role into the constrained-subagent
   execution envelope.
6. Surface that role in contract views and session inspection payloads.

This preserves the current separation:

- policy shaping remains in `tools/delegate.rs`
- orchestration posture remains in `conversation/subagent.rs`
- spawn-time envelope construction remains in `conversation/turn_coordinator.rs`

**Verify Role Semantics**

For this slice, `verifier` becomes a real semantic role even though the current builtin `verify`
preset still carries the existing policy behavior.

That means:

- the role name should appear in persisted child execution metadata
- child prompts and session inspection can describe the child as a verifier
- actual shell/tool/runtime restrictions remain whatever the current resolved delegate policy
  already enforces

This avoids mixing semantic cleanup with a wider policy rewrite.

**Testing Strategy**

Add focused tests for:

- root/default role projection
- `research -> explorer`
- `plan -> worker`
- `verify -> verifier`
- depth-boundary cases where orchestration posture degrades to `leaf` while role remains stable
- constrained-subagent execution serialization/deserialization including the new role field
- delegate spawn paths persisting and surfacing the resolved role in session inspection and runtime
  contract views

Regression coverage should confirm that:

- existing delegate request parsing remains unchanged
- existing policy shaping remains unchanged
- existing nested delegate behavior remains unchanged

**Risk Assessment**

The main risk is semantic duplication between:

- `DelegateBuiltinProfile`
- `AgentRole`
- `ConstrainedSubagentProfile`

This is controlled by giving each one one purpose only:

- `DelegateBuiltinProfile`: public request preset
- `AgentRole`: semantic child identity
- `ConstrainedSubagentProfile`: orchestration posture

The second risk is over-claiming `#970` while only implementing its first bounded slice. That is
controlled by keeping the design explicit that TOML overlay support is a follow-up after the
internal role model is proven stable.

**Why This Slice Is Worth Doing**

The repo already has most of the constrained-delegate substrate that later agent-team work needs.
What it lacks is a clean semantic role layer.

Adding that layer now pays down ambiguity without widening into a new config surface or a policy
rewrite. It makes the current child model easier to understand, gives later overlay work a real
target, and keeps the implementation aligned with the existing delegate/session architecture.
