# Architecture Drift Review Design

Date: 2026-03-13
Branch: `feat/provider-boundary-hardening-20260311`
Scope: architecture SLO reporting, release refactor-budget policy, and release docs governance
Status: Implemented and validated on the current task branch

## Problem

The checklist's long-term governance items were still aspirational:

1. there was no automated monthly architecture drift artifact under `docs/releases/`
2. the release process did not require an explicit refactor-budget entry
3. hotspot and boundary metrics were visible only through the local architecture gate, not through a
   durable monthly record

That meant the repo had local guardrails, but not a stable governance trail.

## Goals

1. Generate a tracked monthly architecture drift report under `docs/releases/`.
2. Base that report on the same hotspot files and boundary checks already used by the architecture
   gate.
3. Support future month-over-month comparisons without adding a heavy data store.
4. Make release docs carry an explicit refactor-budget item.
5. Keep the solution local, scriptable, and easy to verify.

## Non-Goals

1. No CI workflow changes.
2. No new external service or database for architecture metrics.
3. No automatic failure of normal developer flows on the first month without a baseline report.

## Approaches Considered

### A. Document the policy only

Pros:

- lowest implementation cost

Cons:

- no automated artifact
- no durable month-over-month comparison path
- release budget policy still depends on human memory

### B. Add a shell-based report generator backed by the current architecture gate scope

Pros:

- low operational cost
- directly reuses the hotspot/boundary scope the repo already trusts
- easy to commit into `docs/releases/`

Cons:

- shell parsing requires deliberate stable output/marker design

### C. Introduce a new structured metrics service or JSON pipeline

Pros:

- more formal machine-readable history

Cons:

- far too heavy for the repo's current maturity and governance gap

## Decision

Implement Approach B.

The narrow sustainable move is:

1. centralize hotspot budget definitions in a small shell library
2. keep `scripts/check_architecture_boundaries.sh` as the live gate
3. add `scripts/generate_architecture_drift_report.sh` to emit a monthly markdown artifact with
   embedded machine-readable markers for future baselines
4. add a release-doc `Refactor Budget` requirement and enforce it in `scripts/check-docs.sh`

## Target Design

### 1. Shared hotspot scope

Move hotspot budget definitions into a shared shell helper so:

1. the live architecture gate
2. the monthly drift report generator

both use the same tracked hotspot files and thresholds.

### 2. Monthly drift artifact

Generate `docs/releases/architecture-drift-YYYY-MM.md` with:

1. summary metadata
2. hotspot metrics
3. boundary-check status
4. SLO assessment
5. refactor-budget policy pointers
6. embedded marker comments for future baseline parsing

### 3. Release budget policy

Make release docs include:

1. `Refactor budget item:` in `## Process`
2. a dedicated `## Refactor Budget` section

Backfill existing tracked release docs so governance checks remain green.

## Validation Strategy

Minimum validation for this slice:

1. `scripts/check_architecture_boundaries.sh` remains green
2. new generator script has syntax checks and script-level regression coverage
3. `scripts/check-docs.sh` remains green in local mode
4. a real `docs/releases/architecture-drift-2026-03.md` artifact is generated and committed
