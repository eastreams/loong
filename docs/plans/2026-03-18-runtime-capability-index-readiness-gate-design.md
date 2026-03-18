# Runtime Capability Index and Readiness Gate Design

## Problem

`runtime-capability` currently gives LoongClaw one governed record per finished
experiment run:

- one proposal
- one explicit target type
- one explicit operator review decision

That closes the "should this single run become a reusable lower-layer artifact?"
gap, but it still leaves the next governance gap open.

Operators still cannot answer, in one deterministic view:

- which candidate records are really the same underlying promotion intent?
- how many independent runs now support that intent?
- is the evidence strong enough to treat the intent as promotion-ready?
- what compact digest should later automation use instead of reopening every
  source run artifact?

Without that middle layer, every future promotion flow would have to reason over
ad-hoc sets of individual candidate files. That is expensive, noisy, and easy
to get wrong.

## Goal

Add the next strictly read-only layer above individual capability candidates:

1. aggregate compatible candidates into deterministic capability families
2. summarize evidence into a compact, auditable digest
3. evaluate readiness as `ready`, `not_ready`, or `blocked`
4. keep the entire slice local-first, deterministic, and operator-readable
5. avoid automatic promotion or mutation in this phase

## Non-Goals

- automatically generating or applying managed skills
- automatically generating or applying programmatic flows
- automatically mutating runtime config or profile notes
- semantic clustering powered by an LLM or fuzzy matching
- background queues, schedulers, or continuously running index daemons
- changing `runtime-experiment` to become the source of truth for family state
- introducing a promotion executor in the same change

## Constraints

- The source of truth remains the persisted `runtime-capability` candidate
  artifacts.
- The new layer must stay additive and keep existing candidate artifacts valid.
- The grouping rule must be deterministic and explainable from stored fields.
- Readiness must come from explicit evidence checks, not opaque heuristics.
- The first slice should optimize for auditability and token savings over
  autonomy.

## Approaches Considered

### Option A: Add an index command that derives families from proposal intent

Scan candidate artifacts under one root, derive a stable family id from the
normalized promotion-intent fields, and emit one read-only report.

Pros:

- additive and backward-compatible
- keeps candidate artifacts unchanged
- deterministic and easy to diff
- minimal code surface

Cons:

- only exact promotion-intent matches aggregate together
- does not repair older artifacts with inconsistent free-form wording

### Option B: Add a new persisted family artifact and update candidates to point to it

Introduce a new artifact type, persist one family record, and embed a stable
family id into every candidate.

Pros:

- explicit family storage
- faster later lookups

Cons:

- more schema churn immediately after the foundation landed
- requires deciding write/update ownership for a new artifact type
- adds mutation semantics to a slice that can stay read-only for now

### Option C: Use semantic clustering to infer families automatically

Let a model or fuzzy matcher group candidates with similar summaries and scopes.

Pros:

- groups near-duplicate wording together

Cons:

- opaque and unstable
- hard to audit
- introduces model dependence into a governance gate

## Decision

Choose Option A.

The next slice should stay deterministic, read-only, and cheap. A capability
family is defined as "the same normalized promotion intent":

- same target type
- same target summary
- same bounded scope
- same normalized tag set
- same normalized required-capability set

That intent fingerprint is strict by design. If an operator changes the intended
promotion summary or scope, LoongClaw should treat that as a different family
instead of silently merging it.

## Proposed Surface

### Command family

Keep the existing commands and add one new read-only command:

- `loongclaw runtime-capability propose`
- `loongclaw runtime-capability review`
- `loongclaw runtime-capability show`
- `loongclaw runtime-capability index`

### Index command

```bash
loongclaw runtime-capability index \
  --root artifacts/runtime-capability \
  [--json]
```

Behavior:

- recursively scan `--root`
- load JSON files that decode as supported `runtime-capability` artifacts
- derive one stable family id from normalized proposal intent
- group candidates by family id
- build one compact evidence digest per family
- evaluate readiness through explicit deterministic checks
- render text or JSON without mutating any artifact

## Family Model

### Family id

Compute `family_id` from a SHA-256 hash of:

- `proposal.target`
- `proposal.summary`
- `proposal.bounded_scope`
- normalized `proposal.tags`
- normalized `proposal.required_capabilities`

This intentionally excludes:

- `candidate_id`
- timestamps
- source run ids
- labels
- operator review text

The family key therefore represents promotion intent, not one specific piece of
evidence.

### Compact evidence digest

Each family summary should expose compact evidence instead of replaying every
source artifact:

- candidate count
- reviewed / undecided counts
- accepted / rejected counts
- distinct source run count
- distinct experiment id count
- latest candidate timestamp
- latest review timestamp
- source decision rollup
- unique warnings observed across accepted candidates
- per-metric min/max rollups

This gives later automation a stable digest to reason over without reopening all
candidate and experiment files.

## Readiness Gate

Readiness is derived from explicit check results. Each check reports:

- dimension name
- status: `pass`, `needs_evidence`, or `blocked`
- summary

Overall readiness is:

- `blocked` if any check is `blocked`
- `ready` if every check passes
- `not_ready` otherwise

### Checks

#### 1. Review consensus

- `pass`: every candidate in the family has been reviewed and accepted
- `needs_evidence`: at least one candidate remains undecided and there are no
  rejected candidates
- `blocked`: any candidate in the family was rejected

Rationale: promotion cannot be considered ready if the family already contains a
negative operator decision or unresolved candidate evidence.

#### 2. Stability

- `pass`: the family contains evidence from at least two distinct source runs
- `needs_evidence`: fewer than two distinct source runs exist

Rationale: one successful run is promising; two independent runs is the minimum
signal that the capability is repeatable rather than accidental.

#### 3. Accepted-source integrity

Evaluate only accepted candidates:

- `pass`: every accepted candidate came from a `completed` run whose source run
  decision was `promoted` and whose result snapshot id is present
- `needs_evidence`: there are no accepted candidates yet
- `blocked`: any accepted candidate violates those integrity conditions

Rationale: accepted promotion evidence should not be built on aborted or
incomplete runtime results.

#### 4. Warning pressure

- `pass`: accepted candidates carry no source warnings
- `needs_evidence`: accepted candidates exist but still carry warnings

Rationale: warnings are not automatically fatal, but they should prevent the
family from being considered promotion-ready in the first slice.

## Why This Is Simpler Than a Persisted Family Artifact

The index report can be regenerated from candidate artifacts at any time.
Nothing new needs to be synchronized, updated, or rolled back. That keeps the
current change:

- lightweight
- deterministic
- backward-compatible
- easier to test

If later stages need persisted promotion plans, they can consume the family
report contract without forcing this slice to mutate disk state beyond existing
candidate records.

## Testing Strategy

1. Extend CLI parsing coverage for `runtime-capability index`.
2. Add integration coverage proving:
   - same promotion intent across two candidate artifacts collapses into one
     family
   - the family becomes `ready` only after repeated accepted evidence
   - a family becomes `not_ready` when evidence is incomplete
   - a family becomes `blocked` when review decisions conflict
   - non-capability JSON files under the root are ignored
3. Re-run targeted daemon integration tests and direct integration binary
   verification.

## Recommendation

Ship the capability-family index and readiness gate now, but keep it read-only.

That gives LoongClaw the missing governed middle layer:

- experiments answer "what changed?"
- candidates answer "what lower-layer capability could exist?"
- the family index answers "is there enough stable evidence to plan promotion?"

Automatic promotion should remain a later, dry-run-only step built on top of
this index contract instead of being entangled with it.
