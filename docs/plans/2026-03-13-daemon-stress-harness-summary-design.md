# Daemon Stress Harness Summary Design

Date: 2026-03-13
Branch: `feat/provider-boundary-hardening-20260311`
Scope: `scripts/stress_daemon_tests.sh`, harness validation, and reliability docs
Status: Implemented and validated on the current task branch

## Problem

The daemon stress helper was good enough for fail-fast local smoke runs, but weak for resilience
matrix collection:

1. it exited on the first failing row
2. it did not emit a stable summary artifact
3. collecting a full matrix required ad hoc wrapper shells outside the script itself

That made the runtime resilience work slower and less reproducible than it needed to be.

## Goals

1. Preserve the current fail-fast default behavior.
2. Add an explicit continue-on-failure mode for matrix collection.
3. Emit a summary artifact on every run, regardless of pass or fail.
4. Keep the change local to the harness script with a script-level regression test.

## Non-Goals

1. No CI or workflow changes.
2. No JSON report format yet.
3. No change to the actual daemon test commands being executed.

## Approaches Considered

### A. Keep the script fail-fast only

Pros:

- no code change

Cons:

- keeps matrix collection dependent on one-off wrapper scripts
- leaves no first-class artifact describing which rows ran and which failed

### B. Add an explicit continue-on-failure mode plus a deterministic summary artifact

Pros:

- preserves fail-fast as the default
- supports full matrix collection when explicitly requested
- keeps evidence generation inside the canonical harness

Cons:

- slightly more harness logic

### C. Replace the shell script with a new Rust or Python harness

Pros:

- more structure

Cons:

- too much change for a small operational gap
- higher maintenance cost than the problem warrants

## Decision

Implement Approach B.

The gap is operational, not architectural. The narrow sustainable fix is to keep the existing shell
harness, add a single explicit env flag for collection mode, and always write a stable summary file.

## Target Design

1. Add `LOONGCLAW_STRESS_CONTINUE_ON_FAILURE=true|false`, defaulting to `false`.
2. Always write `summary.txt` under the chosen log directory.
3. Record per-run status lines and per-mode result lines in the summary.
4. In continue mode, stop the current mode on first failure but continue to later modes.
5. Still exit non-zero at the end if any mode failed.
6. Add a dedicated shell regression test that validates fail-fast behavior, continue behavior, and
   all-pass summary behavior using a fake `cargo` stub.

## Validation Strategy

Minimum validation for this slice:

1. `bash -n` for the harness and its test
2. `bash scripts/test_stress_daemon_tests.sh`
3. one real cargo-backed harness smoke run producing a summary file
