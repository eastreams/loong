# Alpha-Test Runtime Capability Candidate Foundation Design

## Problem

`runtime-snapshot` and `runtime-experiment` now give LoongClaw a clean,
operator-controlled record layer for:

- one baseline runtime state
- one candidate result state
- one explicit experiment decision
- one optional runtime-surface delta report

That closes the "what changed?" gap, but it still leaves the next
self-evolution gap open.

After an experiment run is compared and accepted or rejected, the operator still
has no first-class artifact that answers:

- what reusable capability candidate did this run produce?
- should the candidate become a `managed_skill`, a `programmatic_flow`, or a
  `profile_note_addendum`?
- what bounded scope and required capabilities should that candidate carry?
- did an operator later accept or reject that capability candidate itself?

Today those answers live in ad-hoc notes, issue comments, or operator memory.
That is workable for one-off exploration, but it is not a durable foundation for
governed self-evolution.

## Goal

Define the smallest LoongClaw-native `runtime-capability` record layer that:

1. derives one capability candidate from one finished `runtime-experiment` run
2. keeps the first slice local-first, deterministic, and operator-readable
3. records the intended promotion target and bounded scope without mutating live
   runtime state
4. adds an explicit review gate above experiment results and below any future
   automatic promotion system
5. stays small enough to ship before any autonomous skill/script generation

## Non-Goals

- automatically generating or applying `managed_skill` artifacts
- automatically generating or applying `programmatic_flow` definitions
- mutating `profile_note` or runtime config during candidate creation/review
- automatically deciding which target type to use
- evaluating arbitrary shell commands or free-form plans inside this command
- replacing `runtime-experiment` as the source of truth for experiment evidence
- building a dashboard, queue, or optimizer loop in the first slice

## Constraints

- `runtime-experiment` remains the source of truth for experiment evidence and
  runtime-state comparison.
- The first artifact format must remain plain JSON, operator-readable, and easy
  to diff in git or local storage.
- The first command surface must not require live runtime mutation, daemon
  writes, or background services.
- Capability candidates must stay portable; embedded data should be summaries,
  not full copied run payloads.
- The target types should align with shipped LoongClaw concepts, not introduce a
  new parallel taxonomy.

## Approaches Considered

### Option A: Extend `runtime-experiment` artifacts directly

Add capability-target metadata and candidate review fields directly to the
existing run artifact schema.

Pros:

- smallest apparent surface area
- no new command family
- direct linkage to the experiment record

Cons:

- mixes "experiment evidence" with "promotion candidate" concerns
- makes the run artifact responsible for two different lifecycle stages
- increases schema churn on the record layer that was intentionally kept small

### Option B: Add a separate `runtime-capability` artifact and CLI

Create a new `runtime-capability` command family with `propose`, `review`, and
`show`, backed by a separate capability-candidate artifact derived from one
finished run.

Pros:

- preserves the boundary between experiment evidence and promotion intent
- creates the missing middle layer between run comparison and future automation
- stays operator-controlled and local-first
- keeps the first slice additive and easy to test

Cons:

- introduces another JSON artifact type to explain
- duplicates a small subset of source run metadata for portability
- does not yet perform any automatic promotion

### Option C: Jump directly to automatic skill/script promotion

Let the system translate selected experiment runs directly into reusable
artifacts and apply them automatically.

Pros:

- closest to the long-term self-evolving runtime vision
- highest headline value

Cons:

- couples generation, review, policy, mutation, and rollback too early
- creates immediate safety and audit questions around incorrect promotion
- risks slop debt before the candidate record contract is proven

## Decision

Choose Option B.

`alpha-test` should first gain a separate capability-candidate artifact and CLI
surface. That keeps `runtime-experiment` focused on "what happened in one run"
while `runtime-capability` answers "what reusable capability candidate should be
considered next."

The first slice should remain record-oriented and explicit. It should not
generate code, install skills, or mutate config.

## Proposed Surface

### Command family

- `loongclaw runtime-capability propose`
- `loongclaw runtime-capability review`
- `loongclaw runtime-capability show`

### Propose command

Create one capability-candidate artifact from one finished experiment run.

```bash
loongclaw runtime-capability propose \
  --run artifacts/runtime-experiment.json \
  --output artifacts/runtime-capability.json \
  --target managed_skill \
  --target-summary "Codify browser preview onboarding as a reusable managed skill" \
  --bounded-scope "Browser preview onboarding and companion readiness checks only" \
  --required-capability invoke_tool \
  --required-capability memory_read \
  --tag browser \
  --tag onboarding
```

Behavior:

- load the source run artifact
- require the run to be finished (`completed` or `aborted`), not `planned`
- require the run to include an evaluation payload
- persist a new capability-candidate artifact with:
  - stable `candidate_id`
  - source run summary
  - explicit target type
  - bounded scope
  - normalized tags
  - normalized required capabilities
  - `proposed` / `undecided` starting state

### Review command

Attach one explicit operator review decision to a proposed capability candidate.

```bash
loongclaw runtime-capability review \
  --candidate artifacts/runtime-capability.json \
  --decision accepted \
  --review-summary "Promotion target is bounded and evidence supports manual codification" \
  --warning "still requires manual implementation"
```

Behavior:

- load the existing candidate artifact
- require the current candidate status to be `proposed`
- record one terminal review decision (`accepted` or `rejected`)
- record one review summary and optional warnings
- persist the updated artifact in place

### Show command

Render one persisted capability-candidate artifact without mutation.

```bash
loongclaw runtime-capability show --candidate artifacts/runtime-capability.json [--json]
```

Text output should keep review-critical fields first:

- `candidate_id`
- `status`
- `decision`
- `target`
- `target_summary`
- `bounded_scope`
- `source_run_id`
- `source_run_decision`
- `source_run_metrics`
- `review_summary`

## Proposed Artifact Model

```json
{
  "schema": {
    "version": 1,
    "surface": "runtime_capability",
    "purpose": "promotion_candidate_record"
  },
  "candidate_id": "candidate-...",
  "created_at": "2026-03-17T08:00:00Z",
  "reviewed_at": null,
  "label": "browser-preview-skill-candidate",
  "status": "proposed",
  "decision": "undecided",
  "proposal": {
    "target": "managed_skill",
    "summary": "Codify browser preview onboarding as a reusable managed skill",
    "bounded_scope": "Browser preview onboarding and companion readiness checks only",
    "tags": ["browser", "onboarding"],
    "required_capabilities": ["invoke_tool", "memory_read"]
  },
  "source_run": {
    "run_id": "run-...",
    "experiment_id": "exp-42",
    "label": "browser-preview-a",
    "status": "completed",
    "decision": "promoted",
    "mutation_summary": "enable browser preview skill",
    "baseline_snapshot_id": "snapshot-a",
    "result_snapshot_id": "snapshot-b",
    "evaluation_summary": "provider and tool policy updated",
    "metrics": {
      "cost_delta": -0.2,
      "task_success": 1.0
    },
    "warnings": ["manual verification only"],
    "artifact_path": "/abs/path/runtime-experiment.json"
  },
  "review": null
}
```

### Key model choices

- `source_run` stores a compact experiment summary so the candidate remains
  readable even when the original run file is not open.
- `artifact_path` remains optional traceability metadata; it is not the source
  of truth.
- `proposal.target` is intentionally limited to shipped LoongClaw terms:
  `managed_skill`, `programmatic_flow`, and `profile_note_addendum`.
- `required_capabilities` are normalized string identifiers so the artifact can
  remain language-agnostic and diff-friendly.
- `review` is absent until the operator performs an explicit review action.

## Why Not Reuse Experiment Decisions Directly

`runtime-experiment` answers whether one runtime mutation attempt looked good or
bad. That is necessary evidence, but it is not the same as deciding how the
result should be crystallized into a reusable capability.

Keeping `runtime-capability` separate preserves this boundary:

- experiment run = one evaluated runtime change
- capability candidate = one proposed reusable codification of that change

## Error Handling

- missing run file -> hard error
- unsupported run artifact schema version -> hard error
- `propose` from a `planned` run -> hard error
- `propose` from a run without evaluation -> hard error
- empty `target_summary` or `bounded_scope` -> hard error
- unknown required capability string -> hard error naming the offending value
- `review` on an already reviewed candidate -> hard error
- malformed candidate review summary -> hard error

## Testing Strategy

1. CLI parsing for `runtime-capability propose|review|show`
2. propose flow persists a normalized candidate artifact from a finished run
3. propose rejects unfinished runs
4. propose rejects unknown capability strings
5. review flow updates the candidate once and persists review data
6. review rejects double review
7. `show --json` round-trips the persisted artifact
8. text rendering surfaces decision-critical fields first

## Rollout

1. Land the artifact schema and CLI surface.
2. Update product docs and roadmap references so `runtime-capability` is
   explained as the review gate above experiment runs and below future
   automation.
3. Keep actual skill/script/profile-note mutation out of scope until the record
   contract is proven in real operator use.

## Recommendation

Implement `runtime-capability` as the next governed self-evolution slice for
`alpha-test`.

It adds the missing middle layer without compromising the existing safety model:

- experiments stay about evidence
- candidates stay about reusable intent
- future automation can build on explicit records instead of ad-hoc operator
  memory
