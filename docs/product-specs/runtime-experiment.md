# Runtime Experiment

## User Story

As a LoongClaw operator, I want to record one snapshot-based experiment run so
that I can compare a proposed runtime change against a baseline and make an
explicit promotion or rejection decision.

## Acceptance Criteria

- [ ] LoongClaw exposes a `runtime-experiment` command family with `start`,
      `finish`, `show`, and `compare` subcommands.
- [ ] `runtime-experiment start` creates a persisted experiment-run artifact
      that records a baseline snapshot summary, mutation summary, optional tags,
      and an explicit `planned` / `undecided` starting state.
- [ ] The experiment run inherits `experiment_id` from the baseline snapshot
      when present, and otherwise requires the operator to provide one.
- [ ] `runtime-experiment finish` attaches a result snapshot summary,
      evaluation summary, numeric metrics, warnings, final status, and decision
      without mutating live runtime state.
- [ ] `runtime-experiment show` round-trips the persisted artifact as JSON and
      renders the decision-critical fields first in text output.
- [ ] `runtime-experiment compare` always renders a decision-oriented summary
      from the persisted run artifact and, when both matching snapshot artifacts
      are supplied, reports targeted runtime-surface deltas without mutating
      runtime state or changing the persisted run schema.
- [ ] Product docs describe `runtime-experiment` as the record layer above
      `runtime-snapshot` and `runtime-restore`, not as an autonomous optimizer
      or automatic promotion system.

## Out of Scope

- Running arbitrary shell commands as part of an experiment run
- Automatically mutating skills, providers, or daemon config
- Automatic promotion, rollback, or branch management policy
- Snapshot indexing, artifact discovery, or bundled archive management
- Evaluator pipelines, dashboards, or autonomous skill-optimization loops
