# Runtime Capability

## User Story

As a LoongClaw operator, I want to derive one explicit capability candidate from
one finished runtime experiment so that I can review how a successful or failed
experiment should be crystallized into a reusable lower-layer capability.

## Acceptance Criteria

- [ ] LoongClaw exposes a `runtime-capability` command family with `propose`,
      `review`, and `show` subcommands.
- [ ] `runtime-capability propose` creates a persisted capability-candidate
      artifact from one finished `runtime-experiment` run.
- [ ] The candidate artifact records one explicit target type:
      `managed_skill`, `programmatic_flow`, or `profile_note_addendum`.
- [ ] The candidate artifact records one bounded scope, normalized tags, and
      normalized required capabilities without mutating live runtime state.
- [ ] `runtime-capability review` records one explicit operator decision
      (`accepted` or `rejected`) plus one review summary and optional warnings.
- [ ] `runtime-capability show` round-trips the persisted artifact as JSON and
      renders the review-critical fields first in text output.
- [ ] Product docs describe `runtime-capability` as the governed review layer
      above `runtime-experiment` and below any future automated promotion loop.

## Out of Scope

- Automatically generating or applying managed skills
- Automatically generating or applying programmatic flows
- Automatically mutating `profile_note` or runtime config
- Automatic promotion, rollback, or optimizer orchestration
- Candidate queues, dashboards, or autonomous ranking systems
