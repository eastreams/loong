# CI Governance Consolidation Design

Date: 2026-03-13
Branch: `feat/ci-governance-phase1-20260313`
Scope: Phase A CI consolidation for `alpha-test`
Status: Approved for implementation

## Problem

The repository currently splits the main PR gate across `ci.yml` and `verify.yml`, while the newer
local governance checks still live outside the primary CI path. That creates duplicated Rust work,
unclear gate ownership, and leaves durable governance checks easier to skip than core test jobs.

## Goals

1. Make `ci.yml` the single main PR gate.
2. Add an explicit governance job for shell-based architecture, release-doc, and harness checks.
3. Keep Rust quality, default tests, and all-features tests as separate jobs for clearer failure
   ownership.
4. Preserve `security.yml`, `codeql.yml`, `perf-lint.yml`, and `perf-benchmark.yml` as dedicated
   workflows.
5. Keep the solution compatible with the current `alpha-test` codebase instead of depending on
   unmerged provider-hardening changes.

## Non-Goals

1. No CI changes to `security.yml`, `codeql.yml`, `perf-lint.yml`, or `perf-benchmark.yml`.
2. No provider-runtime refactor bundled into this CI slice.
3. No new external service or cache layer.

## Decision

Phase A consolidates the gate into `ci.yml` with four always-on jobs plus docs build:

1. `governance`
2. `rust-quality`
3. `rust-test-default`
4. `rust-test-all-features`
5. `docs-build`

`verify.yml` becomes redundant and should be removed after the split is complete.

## Governance Job Scope

The governance job should run:

1. shell syntax checks for the tracked governance scripts
2. harness regression scripts for release artifacts, drift reporting, and stress summary behavior
3. release artifact bootstrap before strict doc-governance validation
4. strict architecture boundary checks
5. dependency graph contract checks
6. whitespace / merge-conflict style validation via `git diff --check`

## Validation Strategy

Minimum validation for this slice:

1. all added shell scripts pass `bash -n`
2. governance regression scripts pass locally
3. strict doc-governance passes after bootstrap
4. strict architecture gate passes on `alpha-test`
5. Rust quality and test jobs still pass locally before PR submission
