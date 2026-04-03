# Cross-Repo Runtime Architecture Convergence Design

Date: 2026-04-03
Status: Proposed

## Summary

LoongClaw already has a stronger kernel and contract story than the comparison
set, but the next stage of work should not be "add more surface area". The
next stage should be "make the kernel-first story fully true at runtime, then
upgrade the session, memory, and tool product surfaces without collapsing
those gains back into app-layer monoliths."

The source comparison shows a clear pattern:

1. LoongClaw is strongest on explicit boundaries.
2. The comparison repos are stronger on operator-facing runtime products.
3. The main risk to LoongClaw is app-layer control-plane concentration.

That means the right direction is convergence, not imitation:

1. preserve LoongClaw's kernel-first architecture
2. import the best operational patterns from mature agent runtimes
3. refuse the large-file, boundary-light shapes that those systems often grew into

## Problem

The current repository has three simultaneous truths:

1. The crate DAG, kernel, capability model, and policy model are real and
   structurally strong.
2. The app layer now carries a large and growing amount of runtime truth in
   tools, conversation orchestration, channels, ACP, and onboarding.
3. Several higher-value runtime surfaces remain less mature than the best
   comparison repos:
   - session persistence and recovery
   - memory/queryability/provenance
   - tool scheduling and concurrency policy
   - approval surface unification
   - operator-facing remote/runtime control contracts

If LoongClaw keeps expanding those runtime surfaces inside existing app-layer
hotspots, it risks reproducing the same large-file and mixed-responsibility
patterns visible in the comparison set.

## Goals

1. Converge the runtime toward a fully governed kernel-first execution model.
2. Treat session runtime, memory, tool scheduling, and approvals as first-class
   products instead of scattered app behavior.
3. Sequence the next stage of work so existing leaf plans stay useful and
   additive rather than becoming overlapping debt.
4. Keep future implementation slices reviewable, testable, and bounded.

## Non-goals

1. Do not rewrite the repository around another project's structure.
2. Do not replace existing leaf plans with a single omnibus implementation.
3. Do not introduce speculative new business surfaces beyond the current
   LoongClaw direction.
4. Do not treat public design docs as proof that a runtime contract already
   exists in code.

## Evidence

### Internal evidence

LoongClaw already has the right architectural bones:

- `crates/contracts/src/lib.rs`
- `crates/kernel/src/kernel.rs`
- `crates/kernel/src/policy.rs`
- `crates/app/src/context.rs`
- `crates/app/src/conversation/context_engine.rs`
- `crates/app/src/memory/system.rs`
- `crates/app/src/acp/manager.rs`

The app-layer concentration risk is also clear:

- `crates/app/src/tools/mod.rs`
- `crates/app/src/conversation/turn_coordinator.rs`
- `crates/app/src/channel/mod.rs`
- `crates/app/src/acp/manager.rs`
- `crates/daemon/src/onboard_cli.rs`

### External comparison evidence

The comparison repos point to specific patterns worth adopting:

- `codex`
  - typed tool registry planning
  - config layer precedence
  - MCP-first runtime surfaces
  - separate execution service boundary
- `openclaw`
  - channel/plugin approval capability modeling
  - session routing metadata
  - skill source scanning and policy surfaces
- `pi-mono`
  - small reusable loop core
  - thin executor boundaries
  - SDK-first decomposition
- `nanobot`
  - legal session-history slicing
  - compact tool registry discipline
  - provider compatibility fallbacks
- `hermes-agent`
  - durable session DB
  - memory-provider manager
  - approval bridge adapters
  - explicit iteration and parallelism policy

## Alternatives Considered

### A. Continue landing narrow fixes without a convergence layer

Rejected.

This would preserve momentum on individual slices, but it would leave the
cross-cutting sequencing problem unsolved. The result would be more leaf plans,
more partial overlap, and more pressure on the same hotspots.

### B. Replace existing leaf plans with one new umbrella implementation

Rejected.

The repository already has useful and specific plan artifacts for memory,
governed-path closure, tool productization, and conversation/runtime hardening.
Throwing them away would destroy existing traceability.

### C. Add a convergence design and sequencing plan above the existing leaf plans

Recommended.

This keeps prior work useful, clarifies execution order, and defines which
cross-repo patterns should be imported into which LoongClaw seams.

## Decision

Adopt option C.

The repository should add one convergence-layer design note and one
implementation plan that:

1. names the highest-value runtime themes
2. explains why they matter together
3. maps each theme to existing LoongClaw leaf plans
4. sequences the work into bounded slices
5. records what to learn from other repos without copying their architecture wholesale

## Recommended Convergence Themes

### Theme 1: Governed Path Closure

Intent:

- make kernel-governed execution the runtime default truth
- keep direct paths explicit, audited, and shrinking

Primary internal anchors:

- `crates/app/src/context.rs`
- `crates/app/src/conversation/runtime_binding.rs`
- `crates/app/src/tools/mod.rs`
- `crates/app/src/channel/mod.rs`

Primary existing plans:

- `docs/plans/2026-03-15-kernel-policy-unification-design.md`
- `docs/plans/2026-03-15-kernel-policy-unification-implementation-plan.md`
- `docs/plans/2026-03-16-governed-runtime-path-hardening-design.md`
- `docs/plans/2026-03-16-governed-runtime-path-hardening-implementation-plan.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-implementation-plan.md`

### Theme 2: Durable Session And Memory Runtime

Intent:

- treat session persistence, searchability, provenance, and hydration as
  first-class runtime surfaces

Primary internal anchors:

- `crates/app/src/memory/system.rs`
- `crates/app/src/conversation/context_engine.rs`
- `crates/app/src/conversation/session_history.rs`
- `crates/app/src/conversation/persistence.rs`

Primary existing plans:

- `docs/plans/2026-03-11-loongclaw-memory-architecture-design.md`
- `docs/plans/2026-03-11-loongclaw-memory-architecture-implementation.md`
- `docs/plans/2026-03-12-memory-context-kernel-unification-design.md`
- `docs/plans/2026-03-12-memory-context-kernel-unification-implementation-plan.md`
- `docs/plans/2026-03-14-loongclaw-pluggable-memory-systems-design.md`
- `docs/plans/2026-03-14-loongclaw-pluggable-memory-systems-implementation.md`
- `docs/plans/2026-03-23-durable-recall-bootstrap-implementation-plan.md`

### Theme 3: App-Layer Control-Plane Decomposition

Intent:

- move from large mixed-responsibility runtime files toward explicit services
  without weakening the kernel boundary

Primary internal anchors:

- `crates/app/src/tools/mod.rs`
- `crates/app/src/conversation/turn_coordinator.rs`
- `crates/app/src/channel/mod.rs`
- `crates/app/src/acp/manager.rs`

Primary existing plans:

- `docs/plans/2026-03-14-alpha-session-runtime-reintegration-design.md`
- `docs/plans/2026-03-14-alpha-session-runtime-reintegration.md`
- `docs/plans/2026-03-15-conversation-runtime-binding-design.md`
- `docs/plans/2026-03-15-conversation-runtime-binding-implementation-plan.md`
- `docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md`
- `docs/plans/2026-03-15-conversation-lifecycle-kernelization-implementation-plan.md`

### Theme 4: Tool Productization And Scheduling

Intent:

- model discoverability, approval mode, scheduling class, and runtime
  eligibility as explicit tool product metadata

Primary internal anchors:

- `crates/app/src/tools/catalog.rs`
- `crates/app/src/tools/mod.rs`
- `crates/app/src/tools/tool_search.rs`
- `crates/app/src/conversation/turn_engine.rs`

Primary existing plans:

- `docs/plans/2026-03-15-product-surface-productization-design.md`
- `docs/plans/2026-03-15-product-surface-productization-implementation-plan.md`
- `docs/plans/2026-03-15-tool-discovery-architecture-design.md`
- `docs/plans/2026-03-15-tool-discovery-architecture.md`
- `docs/plans/2026-03-17-conversation-fast-lane-parallel-tool-batch-design.md`
- `docs/plans/2026-03-17-conversation-fast-lane-parallel-tool-batch-implementation-plan.md`

### Theme 5: Approval Surface Unification

Intent:

- converge CLI, conversation runtime, channels, ACP, and future remote-runtime
  approval behavior around one shared contract

Primary internal anchors:

- `crates/kernel/src/policy.rs`
- `crates/app/src/tools/approval.rs`
- `crates/app/src/conversation/turn_engine.rs`
- `crates/app/src/channel/mod.rs`
- `crates/app/src/acp`

Primary existing plans:

- `docs/plans/2026-03-15-issue-128-approval-attention-rebuild-design.md`
- `docs/plans/2026-03-15-issue-128-approval-attention-rebuild.md`
- `docs/plans/2026-03-15-kernel-policy-unification-design.md`
- `docs/plans/2026-03-15-kernel-policy-unification-implementation-plan.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-implementation-plan.md`

## Recommended Execution Order

The convergence order should be:

1. governed path closure
2. durable session and memory runtime
3. app-layer control-plane decomposition
4. tool productization and scheduling
5. approval surface unification

This order is intentional.

Governed-path closure comes first because every later runtime surface becomes
less trustworthy if execution meaning is still split between governed and
convenience paths.

Durable session and memory comes second because it unlocks a more truthful
runtime substrate for conversation, ACP, channels, and remote control.

Control-plane decomposition comes third because it reduces the risk of landing
the later product work into oversized files.

Tool productization and approval unification should follow after the runtime
substrate and file boundaries are stronger.

## Design Constraints For Implementation

1. Prefer additive slices over repository-wide rewrites.
2. Prefer explicit contracts over hidden compatibility fallbacks.
3. Prefer one named responsibility per service/module.
4. Keep new runtime surfaces test-first and reviewable.
5. Do not import comparison-repo abstractions if they weaken LoongClaw's
   kernel-first direction.

## Expected Outcome

If the convergence plan is followed, LoongClaw should gain:

1. a more defensible kernel-first runtime story
2. a stronger session and memory substrate
3. smaller and clearer app-layer responsibilities
4. more explicit tool runtime behavior
5. a unified approval model across runtime surfaces

The important outcome is not feature parity with another agent runtime.
The important outcome is that LoongClaw becomes both more operationally mature
and more structurally coherent at the same time.
