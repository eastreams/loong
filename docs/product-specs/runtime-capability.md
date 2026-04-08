# Runtime Capability

## User Story

As a LoongClaw operator, I want to derive one explicit capability candidate from
one finished runtime experiment so that I can review how a successful or failed
experiment should be crystallized into a reusable lower-layer capability.

## Acceptance Criteria

- [ ] LoongClaw exposes a `runtime-capability` command family with `propose`,
      `review`, `show`, `index`, `plan`, `apply`, and `activate`
      subcommands.
- [ ] `runtime-capability propose` creates a persisted capability-candidate
      artifact from one finished `runtime-experiment` run.
- [ ] The candidate artifact records one explicit target type:
      `managed_skill`, `programmatic_flow`, or `profile_note_addendum`.
- [ ] The candidate artifact records one bounded scope, normalized tags, and
      normalized required capabilities without mutating live runtime state.
- [ ] When the source run still points at recorded baseline and result snapshot
      artifacts, the candidate artifact persists the snapshot-backed runtime
      delta evidence; when those recorded snapshots are unavailable, the delta
      evidence remains explicitly empty instead of guessed.
- [ ] `runtime-capability review` records one explicit operator decision
      (`accepted` or `rejected`) plus one review summary and optional warnings.
- [ ] `runtime-capability show` round-trips the persisted artifact as JSON and
      renders the review-critical fields first in text output, including a
      compact snapshot-delta summary when one exists.
- [ ] `runtime-capability index` scans persisted candidate artifacts, groups
      matching promotion intent into deterministic capability families, and
      emits a compact evidence digest for each family.
- [ ] Capability-family evidence digests surface how many candidates carried
      snapshot-backed delta evidence plus the union of changed runtime surface
      names across that family.
- [ ] Each capability family reports readiness as `ready`, `not_ready`, or
      `blocked` from explicit evidence checks rather than opaque heuristics.
- [ ] `runtime-capability plan` resolves one indexed family into a dry-run
      promotion plan that describes the target lower-layer artifact, stable
      artifact id, blockers, approval checklist, rollback hints, provenance
      references, the aggregated delta-evidence digest, and one structured
      draft payload preview without mutating runtime state.
- [ ] `runtime-capability apply` materializes one governed draft artifact for a
      promotable family under the target delivery surface without mutating live
      runtime state, reuses the planned payload shape, and remains idempotent
      when the output already matches.
- [ ] `runtime-capability activate` consumes one applied draft artifact and
      either dry-runs or applies one governed activation path for the supported
      target kind, remains idempotent when the live runtime already matches the
      draft, and surfaces explicit verification evidence plus rollback hints.
- [ ] Product docs describe `runtime-capability` as the governed review layer
      above `runtime-experiment`, with `index`/readiness, `plan`, `apply`, and
      `activate` forming the promotion ladder below any future automated
      promotion loop.

## Out of Scope

- Automatically generating or applying managed skills
- Automatically generating or activating programmatic flows
- Automatically mutating `profile_note` or runtime config without an explicit
  operator activation step
- Automatic promotion, rollback, or optimizer orchestration
- Persisted capability-family state or background indexing daemons
- Persisted promotion-plan artifacts or plan caches
- Candidate queues, dashboards, or autonomous ranking systems
