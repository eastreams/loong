# Runtime Capability Promotion Plan Dry-Run Design

## Problem

`runtime-capability` now gives LoongClaw two governed layers above finished
runtime experiments:

- one capability candidate per finished run
- one deterministic family index with compact evidence and readiness

That closes the "what reusable intent emerged?" and "is this family ready?"
questions, but one operational gap still remains.

Even when a family is indexed and evaluated, the operator still cannot answer in
one deterministic view:

- what exact lower-layer artifact should this family become?
- what stable identifier should that artifact carry?
- is the family actually promotable right now, or only plan-able?
- what blockers still stop promotion?
- what approval checklist and rollback notes should the eventual mutation step
  follow?

Without that dry-run layer, any later promotion executor would still need to
re-open raw family summaries and improvise a plan at mutation time. That keeps
the most safety-sensitive step underspecified.

## Goal

Add the smallest read-only promotion-planning layer above capability-family
readiness:

1. resolve one indexed family into one deterministic promotion plan
2. describe the exact lower-layer artifact kind and stable identifier that would
   be created
3. expose whether the family is promotable now without mutating disk or runtime
4. surface blockers, approval checklist items, rollback hints, and provenance in
   one compact operator-readable report
5. keep the entire slice local-first, deterministic, and auditable

## Non-Goals

- automatically generating or applying managed skills
- automatically generating or applying programmatic flows
- automatically mutating `profile_note` or runtime config
- persisting a separate promotion-plan artifact
- background daemons, queues, or schedulers
- semantic ranking or LLM-based plan synthesis
- solving multi-family prioritization in this slice

## Constraints

- The source of truth remains the persisted `runtime-capability` candidate
  artifacts plus the derived family index view.
- This slice must stay additive and read-only.
- The plan contract must be deterministic from stored fields, not from external
  state or model inference.
- Planner output should be cheap enough that later automation can consume it
  instead of replaying every candidate artifact.
- The planner must still render a useful report for `not_ready` and `blocked`
  families; failure to be promotable is data, not a command error.

## Approaches Considered

### Option A: Add a read-only `plan` command above `index`

Compute the family index on demand, resolve one family by id, and derive a
deterministic promotion-plan report.

Pros:

- additive and read-only
- reuses the existing readiness/evidence contract
- no new persisted state to synchronize
- keeps later mutation layers honest by making the plan explicit first

Cons:

- requires re-scanning candidate artifacts for each planning request
- still leaves actual mutation to a later slice

### Option B: Persist a separate promotion-plan artifact

Store one plan document on disk and update it whenever the family changes.

Pros:

- faster lookups later
- explicit artifact path for every plan

Cons:

- adds a new write/update lifecycle immediately
- forces ownership rules for stale plan invalidation
- turns a read-only slice into another persistence surface

### Option C: Jump directly to automatic promotion

Let the system read a ready family and immediately generate the lower-layer
artifact.

Pros:

- closest to the long-term autonomous loop

Cons:

- collapses planning and mutation into one step
- reduces auditability exactly where safety matters most
- introduces unnecessary slop debt before the plan contract is proven

## Decision

Choose Option A.

The next governed self-evolution slice should make the would-be promotion
explicit before anything is allowed to mutate state. The planner should consume
the deterministic family view, preserve that same strictness, and add only the
missing operator-facing details:

- planned lower-layer artifact kind
- stable artifact identifier
- promotability boolean
- blockers
- approval checklist
- rollback hints
- provenance references

## Proposed Surface

### Command family

Keep the existing commands and add one new read-only command:

- `loongclaw runtime-capability propose`
- `loongclaw runtime-capability review`
- `loongclaw runtime-capability show`
- `loongclaw runtime-capability index`
- `loongclaw runtime-capability plan`

### Plan command

```bash
loongclaw runtime-capability plan \
  --root artifacts/runtime-capability \
  --family-id <family-id> \
  [--json]
```

Behavior:

- recursively scan `--root` using the same supported-artifact rules as `index`
- derive the same deterministic family summaries
- select exactly one family by `family_id`
- derive one deterministic promotion plan from that family
- render text or JSON without mutating any artifact

Command errors should be limited to:

- unreadable root
- malformed artifact load
- missing family id
- unknown family id

`not_ready` and `blocked` families should still produce a plan report with
`promotable=false`.

## Promotion Plan Model

### Report shape

The plan report should contain:

- `generated_at`
- `root`
- `family_id`
- `promotable`
- `proposal`
- `evidence`
- `readiness`
- `planned_artifact`
- `blockers`
- `approval_checklist`
- `rollback_hints`
- `provenance`

The planner should reuse the existing family proposal/evidence/readiness model
instead of duplicating those semantics in a second schema.

### Planned artifact

Each plan resolves to one lower-layer artifact description:

- `target_kind`
- `artifact_kind`
- `artifact_id`
- `delivery_surface`
- `summary`
- `bounded_scope`
- `required_capabilities`
- `tags`

`artifact_id` should be deterministic and human-scannable:

- start with a target-specific prefix
- append a slug derived from `proposal.summary`
- suffix with a short prefix of `family_id` for collision resistance

Example shapes:

- `managed_skill` -> `artifact_kind=managed_skill_bundle`
- `programmatic_flow` -> `artifact_kind=programmatic_flow_spec`
- `profile_note_addendum` -> `artifact_kind=profile_note_addendum`

The planner should not guess filesystem paths yet. The executor layer can choose
the final write location later. This slice only promises the exact logical
artifact identity and delivery lane.

### Promotable

`promotable` should be `true` only when the family's readiness status is
`ready`.

This keeps the rule obvious:

- `ready` -> the family is promotable in principle
- `not_ready` -> the family has missing evidence
- `blocked` -> the family currently should not be promoted

### Blockers

`blockers` should be derived from readiness checks that are not `pass`.

- `needs_evidence` checks become actionable missing-evidence blockers
- `blocked` checks become hard-stop blockers

This avoids inventing a second blocker engine.

### Approval checklist

The planner should emit a short deterministic checklist that the eventual
mutation step must satisfy. The checklist should stay generic enough to cover
all target kinds:

- confirm the summary and bounded scope still describe exactly one lower-layer
  artifact
- confirm required capabilities remain least-privilege for that artifact
- confirm provenance references still represent the intended behavior to codify
- confirm the chosen delivery surface matches the target kind

One extra target-specific item should be added:

- `managed_skill`: confirm the behavior belongs in a reusable managed skill
- `programmatic_flow`: confirm the behavior can be expressed as a deterministic
  programmatic flow
- `profile_note_addendum`: confirm the behavior belongs in advisory profile
  guidance rather than executable logic

### Rollback hints

Rollback hints should stay high-signal and executor-neutral:

- capture the current state of the target delivery surface before applying
- remove or revert the promoted artifact by `artifact_id` if downstream
  validation fails
- keep the candidate ids and source-run references attached to the rollback
  record so the revert stays auditable

### Provenance

The plan should carry the minimum references needed to audit where it came from:

- candidate ids
- source run ids
- experiment ids
- source run artifact paths when recorded on the candidate
- latest candidate / review timestamps

This is enough to trace the plan without reopening every file during normal
review.

## Text Output

Text output should remain review-first and compact.

Recommended order:

1. family identity and promotability
2. planned artifact description
3. readiness/check summary
4. blockers
5. approval checklist
6. rollback hints
7. provenance references

Example sketch:

```text
family_id=...
promotable=true
target=managed_skill
artifact_kind=managed_skill_bundle
artifact_id=managed-skill-browser-preview-onboarding-3f2a9c7d1b2e
delivery_surface=managed_skills
required_capabilities=invoke_tool,memory_read
blockers=-
checks=review_consensus:pass:... | stability:pass:...
approval_checklist=confirm bounded scope | confirm least-privilege capabilities | ...
rollback_hints=capture current managed_skills state | remove artifact_id on failure | ...
provenance_candidate_ids=...
provenance_source_run_ids=...
```

## Why This Is Simpler Than Persisted Plan State

The planner can always be regenerated from candidate artifacts. That preserves
the current self-evolution pyramid:

- experiments create evidence
- candidates capture promotion intent
- family index summarizes readiness
- promotion planner describes the would-be lower layer
- a future executor can mutate only after all prior layers are explicit

Nothing new needs to be synchronized or garbage-collected in this slice.

## Testing Strategy

1. Extend CLI parsing coverage for `runtime-capability plan`.
2. Add failing integration tests for:
   - planning a ready family
   - planning a not-ready family and surfacing blockers
   - planning a blocked family and surfacing blockers
   - rejecting unknown family ids
   - emitting target-specific artifact metadata and provenance references
3. Implement the smallest plan derivation logic needed to satisfy those tests.
4. Update product docs and roadmap so the runtime-capability ladder is described
   as candidate -> index/readiness -> dry-run planner -> future executor.
