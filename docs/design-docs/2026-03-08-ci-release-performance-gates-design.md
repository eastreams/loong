# CI Release Performance Gates Design

Date: 2026-03-08  
Status: Proposed

## Context

LoongClaw currently has baseline CI correctness/security checks, but full performance benchmarking is expensive and should not block every PR. We want stronger release confidence without making everyday contribution loops slow.

## Decision

Adopt a split-gate model:

- PR lane: fast, blocking checks (`verify`, `security`, `perf-lint`).
- Nightly lane: full pressure benchmark gate with baseline enforcement.
- Release lane: full pressure benchmark preflight must pass before publish.

This corresponds to option 2 from brainstorming: PR baseline lint only + nightly/release full benchmark.

## Alternatives Considered

1. PR none + nightly/release full benchmark
- Pros: fastest PR feedback.
- Cons: too much risk shifts to nightly/release.

2. PR baseline lint + nightly/release full benchmark (chosen)
- Pros: strong config-integrity checks on PR, deep runtime checks off critical path.
- Cons: some regressions detected later than PR time.

3. PR full benchmark + nightly/release full benchmark
- Pros: maximal early detection.
- Cons: likely too slow and noisy for normal PR throughput.

## Architecture

### Workflows

1. `verify.yml` remains the main correctness gate and required check.
2. New `perf-lint` workflow runs benchmark baseline lint only:
- command: `./scripts/lint_programmatic_pressure_baseline.sh ... true`
- trigger: PR/push with path filters for relevant runtime/spec/benchmark files.
3. New `perf-benchmark` workflow runs full benchmark:
- command: `./scripts/benchmark_programmatic_pressure.sh ... true`
- trigger: nightly cron + `workflow_dispatch`.
- output: benchmark JSON artifact and job summary.
4. `release.yml` adds performance preflight dependency:
- run full benchmark (or invoke reusable workflow) before publish steps.
- fail-stop on benchmark failure.

### Scheduling and Cost Controls

- Add `concurrency` cancellation on PR workflows to stop superseded runs.
- Keep full benchmark out of default PR path.
- Keep heavy docs/build/perf tasks in non-PR or one-lane-only contexts.

## Performance Policy

### PR Policy (Blocking)

- `verify`
- `security`
- `perf-lint`

### Nightly Policy

- Full benchmark gate with baseline enforcement.
- Persist machine-readable report artifact for trend analysis.

### Release Policy

- Full benchmark preflight is mandatory.
- Release publish is blocked unless preflight passes.

## Baseline Governance

- Baseline/schema updates must use:
  - `scripts/update_programmatic_pressure_schema_baseline.sh`
- No mixed PRs combining feature changes and threshold relaxations.
- Threshold updates require benchmark report evidence attachment.

## Failure and Flake Handling

- Nightly failures: allow one automatic rerun before filing incident issue.
- Release preflight failures: hard-stop, no publish.
- PR lint failures: immediate block, fix baseline/matrix consistency first.

## Validation Plan

1. Verify PR path runtime stays acceptable after adding `perf-lint`.
2. Validate nightly benchmark artifact generation and retention.
3. Simulate one benchmark failure and confirm release publish is blocked.
4. Confirm branch protection includes new required check names.

## Out of Scope

- Multi-platform full performance benchmarking in PR.
- Statistical trend dashboards beyond artifact retention.
- Automatic baseline relaxation logic.
