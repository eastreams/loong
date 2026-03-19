# Runtime Capability Delta Evidence Design

## Context

`runtime-experiment compare` already knows how to derive structured
snapshot-backed runtime deltas from recorded baseline and result snapshots.

`runtime-capability`, however, still stops one layer too early:

- `propose` stores source run metadata, review intent, and bounded scope
- `index` aggregates candidate evidence into family-level readiness summaries
- `plan` emits a dry-run promotion artifact with provenance and blockers

What is still missing is the structured delta itself. That means later
promotion-oriented layers would have to recompute compare output on demand or
infer material changes from prose fields such as `mutation_summary`,
`target_summary`, and `bounded_scope`.

Issue `#346` tracks that missing substrate. A previous stacked PR (`#348`)
contained this slice, but it was closed after its base branch was merged and the
base ref was deleted. The delta-evidence portion did not land in `dev`.

## Problem

The current capability artifact is not self-contained enough for the next
promotion-materialization layer.

Without persisted delta evidence:

- operators cannot review the exact changed runtime surfaces from the
  capability artifact itself
- `index` and `plan` cannot surface compact evidence about what changed across a
  family
- later mutation-oriented work would need ad-hoc recomputation from snapshot
  artifacts that may not remain available forever

This is a correctness and auditability gap, not a UI polish issue.

## Goals

This replacement slice should:

1. persist optional snapshot-backed delta evidence inside
   `RuntimeCapabilitySourceRunSummary`
2. aggregate compact family-level delta evidence in `index` and `plan`
3. surface the evidence in `show`, `index`, and `plan` text and JSON outputs
4. stay backward-compatible for existing capability artifacts that do not carry
   delta evidence

## Non-Goals

This slice must not:

- change readiness policy semantics
- add promotion executors or apply paths
- add new CLI flags
- introduce background indexing, caches, or artifact migrations
- broaden runtime-experiment compare semantics beyond extracting a reusable
  helper

## Approaches Considered

### Approach A: Persist delta evidence in capability artifacts

Reuse the existing runtime-experiment compare logic, store the optional delta on
each proposed capability candidate, and aggregate a compact digest at family
level.

Pros:

- fixes the actual missing substrate
- keeps artifacts self-contained
- reuses existing snapshot-compare semantics
- keeps later promotion work deterministic and auditable

Cons:

- extends the persisted capability artifact schema
- requires explicit tests for old-artifact compatibility

### Approach B: Recompute delta evidence in `show`, `index`, and `plan`

Leave capability artifacts unchanged and derive deltas only when commands are
rendered.

Pros:

- smaller write-path change

Cons:

- artifacts remain incomplete
- later layers depend on external snapshot availability
- repeated recomputation invites drift and hidden failure modes

### Recommendation

Choose Approach A.

The gap is in persisted evidence, not in rendering. Recomputing later would
only hide the missing contract.

## Data Model Changes

### Candidate-Level Evidence

Extend `RuntimeCapabilitySourceRunSummary` with:

- `snapshot_delta: Option<RuntimeExperimentSnapshotDelta>`

Rules:

- `None` when the source run has no result snapshot
- `None` when recorded snapshot artifact paths are absent
- `Some(delta)` when recorded baseline/result snapshot paths exist and the delta
  can be derived deterministically
- return an error when recorded snapshot paths exist but the delta cannot be
  derived because artifacts are missing or malformed

### Family-Level Evidence Digest

Extend `RuntimeCapabilityEvidenceDigest` with:

- `delta_candidate_count: usize`
- `changed_surfaces: Vec<String>`

Rules:

- count only candidates with `snapshot_delta.is_some()`
- aggregate a sorted unique union of changed runtime surfaces across those
  candidates
- keep the digest compact; do not inline full before/after payloads at family
  level

## Command Behavior

### `runtime-capability propose`

- load the finished runtime-experiment artifact as today
- derive the optional snapshot delta from the recorded snapshot paths when
  available
- persist it under `source_run.snapshot_delta`

### `runtime-capability show`

- continue to round-trip the full artifact as JSON
- add compact text output for:
  - delta presence
  - changed surface count
  - changed surface names

### `runtime-capability index`

- keep readiness evaluation unchanged
- add family-level delta digest fields to the evidence object
- add text rendering for:
  - `delta_evidence_candidates`
  - `delta_changed_surfaces`

### `runtime-capability plan`

- reuse the family evidence digest unchanged
- surface the same compact delta summary in the plan output

## Testing Strategy

Follow TDD and add failing integration tests first for:

1. `propose` persisting `snapshot_delta` when recorded snapshots are available
2. `propose` leaving `snapshot_delta` empty when recorded snapshots are absent
3. `propose` rejecting broken recorded snapshot references
4. `index` reporting `delta_candidate_count` and `changed_surfaces`
5. `plan` surfacing the same digest
6. `show` rendering compact delta evidence text
7. existing artifacts without the new field continuing to deserialize

## Delivery Notes

This slice should land as the replacement delivery for issue `#346`.

The PR should explicitly state that:

- `#348` was a stacked PR whose base branch was merged and removed
- the delta-evidence subset never reached `dev`
- this replacement PR closes the remaining substrate gap without changing
  readiness policy or adding mutation behavior
