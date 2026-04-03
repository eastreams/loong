# Runtime Convergence Implementation Plan

Date: 2026-04-03

## Goal

Translate `docs/plans/2026-04-03-runtime-convergence-design.md` into a bounded
execution plan that sequences the next runtime stage without replacing the
existing leaf plans.

This is a documentation-only implementation plan. It defines order, ownership
of themes, and verification expectations. It does not claim runtime behavior
changes.

## Requirements Summary

The convergence layer should produce these outcomes:

1. maintainers can see one explicit next-stage execution order
2. existing leaf plans remain authoritative within their narrow seams
3. runtime work stops overlapping casually in the same app-layer hotspots
4. external-reference lessons are captured without changing LoongClaw's
   kernel-first architecture
5. the repository truth sources stay aligned with the actual governed-path
   reality

## Acceptance Criteria

1. The repository contains one convergence design note and one convergence
   implementation plan in `docs/plans/`.
2. The convergence docs identify the five runtime themes and explain why they
   must be sequenced together.
3. Each theme maps to the existing leaf plans that remain authoritative.
4. The docs state a recommended execution order and explain why that order is
   preferred.
5. The resulting PR is documentation-only and does not claim runtime behavior
   changed.

## Workstreams

### Workstream 1. Truth-Sync Artifacts

Scope:

- align `docs/ROADMAP.md` with current governed-path reality
- align `docs/QUALITY_SCORE.md` with current security and observability state
- add the convergence design and implementation plan documents

Why first:

- all other work depends on truthful baseline docs

Exit condition:

- repo-facing docs no longer describe retired tool-policy and audit defaults as
  if they were current

### Workstream 2. Governed Path Closure

Scope:

- continue work under the existing governed-path and binding-first plans
- treat `#766`, `#838`, and `#839` as the most immediate active issue lane

Required closure before advancing:

- direct-mode and optional-context behavior is narrower and more explicit at
  production entrypoints
- new work does not reintroduce ambiguous authority semantics deeper in the
  runtime

### Workstream 3. Durable Session And Memory Runtime

Scope:

- continue the memory and durable recall tracks after governed-path closure is
  stable enough to stop reopening authority questions

Required closure before advancing:

- session and memory runtime behavior is stable enough that later control-plane
  cleanup is not fighting persistence ambiguity

### Workstream 4. App-Layer Control-Plane Decomposition

Scope:

- decompose conversation/operator/delegate/control-plane hotspots only after the
  first two workstreams are materially settled

Required closure before advancing:

- hotspot reduction is happening without reintroducing governance drift

### Workstream 5. Tool Productization And Scheduling

Scope:

- convert mature internal runtime/tool behavior into clearer product and
  benchmark surfaces

Required closure before advancing:

- artifacts describe stabilized behavior rather than in-flight churn

### Workstream 6. Approval Surface Unification

Scope:

- unify approval and consent semantics after the earlier runtime and
  control-plane seams are clearer

Required closure for this stage:

- approval no longer has to compensate for unresolved authority and lifecycle
  drift in deeper runtime layers

## Sequencing Rules

1. Do not start broad control-plane decomposition before governed-path closure
   and memory stabilization have stopped moving the authority contract.
2. Do not lead with ecosystem/product packaging before the runtime contract is
   stable enough to document honestly.
3. Do not use approval work as a substitute for missing governed-path closure.
4. Keep each later PR linked back to both the convergence layer and the narrow
   leaf plan it is actually implementing.

## Issue Mapping

The current active or relevant issues already show where the first execution
pressure exists:

- `#766` for binding-first seam tightening
- `#838` for fail-closed app-tool execution under missing kernel context
- `#839` for production conversation entrypoint hardening
- `#48` for the long-term handle-model discussion that remains separate from
  this convergence layer
- `#458` for governance-simplification classification, which remains adjacent
  context rather than the execution parent for this runtime sequence

## Deliverable Shape For Later PRs

Each later implementation PR under this convergence layer should do four
things:

1. name the specific theme it belongs to
2. reference the narrow leaf plan it implements
3. state what changed and what did not change
4. provide verification evidence proportionate to the touched runtime seam

## Verification Plan

For this documentation-only slice:

1. verify the new convergence docs are present and internally consistent
2. verify roadmap and quality-score edits match the current code and security
   docs
3. run repo checks appropriate for a documentation-only commit, plus the
   repository's required CI-parity gates before commit

For later implementation slices:

1. governed-path changes require runtime regression tests
2. memory/runtime work requires persistence and history-path validation
3. control-plane decomposition requires hotspot and behavior regression checks
4. productization work requires artifact generation and documentation truth
   validation
5. approval work requires explicit negative and replay-path controls

## Non-Goals

1. This plan does not replace the existing leaf plans.
2. This plan does not introduce new product goals outside the current
   LoongClaw direction.
3. This plan does not authorize broad runtime rewrites without a narrow parent
   issue and evidence-backed scope.
