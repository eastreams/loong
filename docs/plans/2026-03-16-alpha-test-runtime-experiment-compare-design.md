# Alpha-Test Runtime Experiment Compare Design

## Goal

Add a minimal `runtime-experiment compare` surface that helps an operator judge a
finished experiment run against its baseline without turning the record layer
into an autonomous optimizer.

## Problem

`runtime-experiment start|finish|show` made snapshot-based experiment runs
persistable and inspectable, but an operator still has to manually stitch
together three different artifacts to answer the practical question:

> what actually changed between the baseline runtime state and the result state,
> and does that change support the recorded decision?

Today that answer requires reading raw JSON or separately rendering snapshots.
That is workable for one-off debugging, but not for a reusable middle-layer
experiment service.

## Constraints

- Keep `runtime-experiment` as a record-and-compare layer above
  `runtime-snapshot` and `runtime-restore`
- Do not add automatic mutation, promotion, rollback, or orchestration
- Avoid schema churn in the persisted run artifact unless strictly necessary
- Reuse existing artifact and rendering conventions instead of introducing a
  generic diff engine

## Options Considered

### Option A: Artifact-only compare

Add `runtime-experiment compare --run <path>` and render only the persisted run
fields plus evaluation metrics and warnings.

Pros:

- smallest code change
- no extra inputs
- no schema change

Cons:

- too little new information beyond `show`
- does not explain runtime-surface drift
- weak operator value for promotion/rejection review

### Option B: Run-aware compare with explicit snapshot inputs

Add `runtime-experiment compare --run <path>` and allow the operator to supply
matching `--baseline-snapshot` and `--result-snapshot` artifacts to produce a
targeted runtime delta summary.

Pros:

- no persisted schema change
- materially more useful than `show`
- validates that supplied snapshots match the run's recorded snapshot ids
- keeps portability boundaries clear because local file paths are not embedded
  into persisted artifacts

Cons:

- operators must provide snapshot paths when they want deep comparison
- compare output must choose a narrow set of stable fields instead of dumping
  everything

### Option C: Embed snapshot paths or full snapshots into the run artifact

Extend `start` / `finish` to persist snapshot source paths or full snapshot
documents so `compare --run` can always deep-diff without extra inputs.

Pros:

- most convenient compare UX

Cons:

- couples persisted experiment records to machine-local paths or bloated payloads
- expands artifact versioning and migration burden
- pulls the record layer toward archive management too early

## Decision

Choose Option B.

This is the smallest change that materially improves experiment review while
preserving the current architecture. The operator gets an explicit compare
surface, but the run artifact remains a portable record rather than a bundled
archive.

## Proposed CLI

Add a new subcommand:

```text
loongclaw runtime-experiment compare \
  --run path/to/run.json \
  [--baseline-snapshot path/to/baseline.json] \
  [--result-snapshot path/to/result.json] \
  [--json]
```

Behavior:

- always load the run artifact
- render a decision-oriented comparison summary even when no snapshot artifacts
  are provided
- require `--baseline-snapshot` and `--result-snapshot` together when loading
  runtime-surface deltas
- verify that the supplied snapshot ids match the run's recorded baseline and
  result snapshot summaries
- reject deep comparison when the run has no recorded result snapshot

## Comparison Model

The compare command should not implement a generic JSON diff. It should surface
only the runtime areas that are already stable and meaningful in the existing
snapshot contract:

- run status, decision, mutation summary, evaluation summary, metrics, warnings
- baseline/result snapshot ids, labels, timestamps, and experiment ids
- provider active profile and active model
- context-engine selected backend and compaction policy
- memory-system selected backend and policy mode/profile/backend
- ACP selected backend, enablement, dispatch routing, default agent, and allowed
  agents
- enabled channel ids and enabled service channel ids
- visible tool names plus capability snapshot sha256
- external-skill ids discovered in inventory

For list-like surfaces, compare should emit `added` and `removed` sets. For
scalar surfaces, compare should emit `before` and `after`.

## Output Shape

Text output should keep decision-critical fields first, then show loaded delta
sections:

- `run_id`
- `experiment_id`
- `baseline_snapshot_id`
- `result_snapshot_id`
- `status`
- `decision`
- `evaluation_summary`
- `metrics`
- `warnings`
- `compare_mode` (`record_only` or `snapshot_delta`)
- targeted delta lines

JSON output should expose the same information through an explicit compare
report payload instead of reusing the run artifact schema.

## Error Handling

- fail if only one of `--baseline-snapshot` or `--result-snapshot` is supplied
- fail if the supplied baseline snapshot id does not match the run artifact
- fail if the supplied result snapshot id does not match the run artifact
- fail if deep compare is requested for a `planned` run with no result snapshot
- tolerate missing optional nested fields inside snapshot payloads and render
  `null` / empty deltas rather than panicking

## Non-Goals

- automatic promotion or rollback
- path discovery, snapshot indexing, or experiment-run listing
- arbitrary JSON diff or per-field patch output
- evaluator execution or metric delta computation against a baseline evaluator

## Validation

- CLI parse coverage for the new `compare` subcommand
- integration coverage for record-only compare output
- integration coverage for snapshot-delta compare output
- regression coverage for mismatched snapshot ids and partial snapshot inputs
