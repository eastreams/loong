# Runtime Convergence Design

Date: 2026-04-03

Related:

- issue `#837`
- implementation package:
  `docs/plans/2026-04-03-runtime-convergence-implementation-plan.md`

## Goal

Define one convergence layer above the current leaf plans so the next stage of
LoongClaw runtime work is sequenced explicitly rather than inferred from many
independent documents.

This design uses the public `microsoft/agent-governance-toolkit` repository as
an external reference point, not as a template to copy directly.

## Why This Exists

LoongClaw already has many strong plan documents across governed runtime
closure, memory, approvals, tool surfaces, and protocol/runtime hardening.

The missing artifact is not another leaf plan. The missing artifact is one
explicit sequence layer that answers:

1. which themes belong to the same next-stage runtime arc
2. which existing plans remain authoritative within each theme
3. what order reduces overlap in the app-layer hotspots
4. what lessons from a mature public governance repository are worth borrowing
   without weakening the kernel-first design

## External Reference Summary

The external repository is useful because it proves a different kind of
strength:

- governance as a consumable product surface
- retrofit-style adoption paths
- operator-facing evidence such as benchmarks, control mapping, and release
  artifacts
- ecosystem packaging across multiple adapters and SDKs

LoongClaw should learn from those delivery patterns. It should not abandon its
current architectural advantage:

- typed contract surfaces
- kernel-owned governance seams
- explicit governed versus direct runtime bindings
- stronger long-term runtime-isolation potential

## Current Repo Truth

The convergence layer is based on the current repository, not on old roadmap
claims.

### What Is Strong Today

1. The workspace remains a strict 7-crate DAG.
2. Core tool and memory execution already route through kernel policy and audit
   seams.
3. Conversation and provider paths now use explicit runtime bindings rather than
   relying only on raw optional kernel context.
4. Audit bootstrap is more truthful than some older docs imply: production
   runtime can use durable JSONL and fanout sinks.
5. Runtime evidence is converging across `process_stdio`, `http_json`, and WASM
   lanes.

### What Is Still Incomplete

1. Architecture-truth docs still lag current code in a few places.
2. Compatibility seams remain numerous around optional kernel authority and
   direct-mode normalization.
3. Runtime isolation remains partially delivered rather than fully closed.
4. Governance evidence exists internally but is not yet packaged as an operator
   product surface.

## Convergence Themes

The next stage should be sequenced around five themes.

### Theme 1. Governed Path Closure

Purpose:

Make kernel-first execution more structurally true by continuing to narrow
direct-mode and optional-context seams.

Why it belongs first:

- it protects the core architecture claim
- it reduces ambiguity before later runtime or product-surface work
- it lowers the chance that later features land on permissive compatibility
  paths

### Theme 2. Durable Session And Memory Runtime

Purpose:

Stabilize the memory/session substrate that governed runtime depends on.

Why it comes early:

- governed runtime hardening is less meaningful if session and memory behavior
  still carry avoidable drift or runtime-specific inconsistencies
- several later operator and evidence surfaces depend on stable memory and
  history semantics

### Theme 3. App-Layer Control-Plane Decomposition

Purpose:

Reduce hotspot overlap in the app layer by clarifying which responsibilities
belong to conversation runtime, approval flow, operator/delegate control, and
ACP-style orchestration.

Why it must follow the first two themes:

- otherwise control-plane cleanup can accidentally reopen authority drift
- memory and governed-path truth should be stable before decomposing the higher
  coordination layer

### Theme 4. Tool Productization And Scheduling

Purpose:

Turn strong internal tool/runtime capabilities into more explicit product
surfaces, benchmarked scheduling behavior, and operator-facing evidence.

Why it comes after the earlier runtime closure work:

- external productization works best after the execution contract is stable
- otherwise operator-facing artifacts risk documenting transient behavior

### Theme 5. Approval Surface Unification

Purpose:

Converge approval, consent, risk, and replay behavior into one clearer runtime
surface.

Why it is last in this convergence layer:

- approval sits across governed execution, memory continuity, and control-plane
  behavior
- it benefits from earlier closure of authority seams and runtime decomposition

## Existing Leaf Plans That Remain Authoritative

The convergence layer does not replace the current leaf plans. It sequences
them.

### Theme 1. Governed Path Closure

Primary references:

- `docs/plans/2026-03-15-conversation-runtime-binding-design.md`
- `docs/plans/2026-03-15-conversation-runtime-binding-implementation-plan.md`
- `docs/plans/2026-03-15-provider-binding-normalization-design.md`
- `docs/plans/2026-03-15-provider-binding-normalization-implementation-plan.md`
- `docs/plans/2026-03-16-governed-runtime-path-hardening-design.md`
- `docs/plans/2026-03-16-governed-runtime-path-hardening-implementation-plan.md`

Active issue alignment:

- `#766`
- `#838`
- `#839`

### Theme 2. Durable Session And Memory Runtime

Primary references:

- `docs/plans/2026-03-11-loongclaw-memory-architecture-design.md`
- `docs/plans/2026-03-11-loongclaw-memory-architecture-implementation.md`
- `docs/plans/2026-03-12-memory-context-kernel-unification-design.md`
- `docs/plans/2026-03-12-memory-context-kernel-unification-implementation-plan.md`
- `docs/plans/2026-03-23-durable-recall-bootstrap-implementation-plan.md`
- `docs/plans/2026-03-24-runtime-self-advisory-boundary-design.md`
- `docs/plans/2026-03-24-runtime-self-advisory-boundary-implementation-plan.md`

### Theme 3. App-Layer Control-Plane Decomposition

Primary references:

- `docs/plans/2026-03-17-constrained-delegate-subagent-design.md`
- `docs/plans/2026-03-17-constrained-delegate-subagent-implementation-plan.md`
- `docs/plans/2026-03-22-multi-session-concurrent-channel-dispatch-design.md`
- `docs/plans/2026-03-22-multi-session-concurrent-channel-dispatch-implementation-plan.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-implementation-plan.md`

### Theme 4. Tool Productization And Scheduling

Primary references:

- `docs/plans/2026-03-15-tool-discovery-architecture.md`
- `docs/plans/2026-03-15-product-surface-productization-design.md`
- `docs/plans/2026-03-15-product-surface-productization-implementation-plan.md`
- `docs/plans/2026-03-17-conversation-fast-lane-parallel-tool-batch-design.md`
- `docs/plans/2026-03-17-conversation-fast-lane-parallel-tool-batch-implementation-plan.md`
- `docs/plans/2026-03-18-runtime-capability-index-readiness-gate-design.md`
- `docs/plans/2026-03-18-runtime-capability-index-readiness-gate-implementation-plan.md`
- `docs/plans/2026-03-27-conversation-turn-tool-token-benchmark-design.md`
- `docs/plans/2026-03-27-conversation-turn-tool-token-benchmark-implementation-plan.md`

### Theme 5. Approval Surface Unification

Primary references:

- `docs/plans/2026-03-15-issue-128-approval-attention-rebuild-design.md`
- `docs/plans/2026-03-15-issue-128-approval-attention-rebuild.md`
- `docs/plans/2026-03-18-delegate-runtime-effective-contract-alignment-design.md`
- `docs/plans/2026-03-18-delegate-runtime-effective-contract-alignment-implementation-plan.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-architecture.md`
- `docs/plans/2026-03-26-autonomy-policy-kernel-implementation-plan.md`

## Recommended Execution Order

The preferred order is:

1. governed path closure
2. durable session and memory runtime
3. app-layer control-plane decomposition
4. tool productization and scheduling
5. approval surface unification

## Why This Order Is Preferred

### Governed Path Closure Before Memory And Control-Plane Work

If the governed-path story is still semantically loose, later work risks
landing on ambiguous authority seams.

### Memory Before Control-Plane Decomposition

Control-plane cleanup is easier when session persistence, memory assembly, and
runtime self-continuity are already clearer and more stable.

### Decomposition Before Productization

Operator-facing product surfaces should describe stabilized runtime behavior,
not transient hotspot internals.

### Approval Unification After Earlier Closure

Approval touches governance, runtime sequencing, and operator experience. It is
best converged after the underlying execution and control-plane seams are
clearer.

## What The External Comparison Changes

The comparison does not change LoongClaw's architectural direction.

It does change emphasis in three ways:

1. it raises the priority of architecture-truth sync and operator-facing
   evidence
2. it confirms that retrofit and product-surface packaging should happen after
   core runtime closure, not before
3. it strengthens the case for finishing runtime isolation because that is the
   clearest area where LoongClaw can become stronger than application-layer
   governance middleware

## Non-Goals

1. Do not replace all current leaf plans with this document.
2. Do not turn the repo into a copy of another governance framework.
3. Do not use this document to justify broad delete-first architecture work.
4. Do not claim runtime behavior changed just because the planning layer is now
   clearer.

## Sources

External:

- https://github.com/microsoft/agent-governance-toolkit
- https://raw.githubusercontent.com/microsoft/agent-governance-toolkit/main/docs/ARCHITECTURE.md
- https://raw.githubusercontent.com/microsoft/agent-governance-toolkit/main/BENCHMARKS.md
- https://raw.githubusercontent.com/microsoft/agent-governance-toolkit/main/CHANGELOG.md
- https://raw.githubusercontent.com/microsoft/agent-governance-toolkit/main/docs/OWASP-COMPLIANCE.md
- https://github.com/microsoft/agent-governance-toolkit/releases

Internal:

- `ARCHITECTURE.md`
- `docs/ROADMAP.md`
- `docs/SECURITY.md`
- `docs/QUALITY_SCORE.md`
- `crates/kernel/src/kernel.rs`
- `crates/kernel/src/policy_ext.rs`
- `crates/contracts/src/audit_types.rs`
- `crates/app/src/context.rs`
- `crates/app/src/conversation/runtime_binding.rs`
- `crates/app/src/conversation/runtime.rs`
- `crates/spec/src/spec_runtime.rs`
