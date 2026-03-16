# Runtime Experiment Compare Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a minimal `runtime-experiment compare` CLI that summarizes a run
and, when explicit snapshot artifacts are supplied, reports a targeted runtime
delta between baseline and result snapshots.

**Architecture:** Extend the existing `runtime_experiment_cli` module with a new
subcommand, a small compare-report model, and targeted snapshot readers that
reuse the current runtime snapshot artifact schema. Keep the persisted
experiment-run artifact unchanged and validate snapshot-to-run identity through
recorded snapshot ids.

**Tech Stack:** Rust, `clap`, `serde`, existing daemon integration-test harness

---

### Task 1: Add failing CLI parsing coverage for `runtime-experiment compare`

**Files:**
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Write the failing test**

Add a parsing test that expects:

- `runtime-experiment compare --run artifacts/run.json`
- optional `--baseline-snapshot` and `--result-snapshot`
- `--json`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon --test integration runtime_experiment_cli_parses_start_finish_and_show -- --exact`

Expected: FAIL because the CLI does not parse `compare` yet.

**Step 3: Write minimal implementation**

Add the `Compare` variant and command options to the runtime experiment CLI.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon --test integration runtime_experiment_cli_parses_start_finish_and_show -- --exact`

Expected: PASS.

### Task 2: Add failing compare integration tests

**Files:**
- Modify: `crates/daemon/tests/integration/runtime_experiment_cli.rs`

**Step 1: Write the failing tests**

Add focused tests for:

- record-only compare output from a finished run
- snapshot-delta compare with matching baseline/result artifacts
- rejection when only one snapshot path is supplied
- rejection when supplied snapshot ids do not match the run artifact

**Step 2: Run tests to verify they fail**

Run: `cargo test -p loongclaw-daemon --test integration runtime_experiment_compare -- --nocapture`

Expected: FAIL because compare execution and rendering are missing.

**Step 3: Write minimal implementation**

Add compare execution, validation, snapshot delta extraction, and text / JSON
rendering in `runtime_experiment_cli.rs`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p loongclaw-daemon --test integration runtime_experiment_compare -- --nocapture`

Expected: PASS.

### Task 3: Refactor compare helpers to stay narrow and readable

**Files:**
- Modify: `crates/daemon/src/runtime_experiment_cli.rs`

**Step 1: Clean up helper boundaries**

Extract small helpers for:

- snapshot identity validation
- scalar before/after compare entries
- set added/removed compare entries
- nested JSON field lookups used by compare

**Step 2: Re-run focused tests**

Run: `cargo test -p loongclaw-daemon --test integration runtime_experiment_compare runtime_experiment_show_round_trips_the_persisted_artifact`

Expected: PASS.

### Task 4: Update product docs

**Files:**
- Modify: `docs/product-specs/runtime-experiment.md`
- Modify: `docs/ROADMAP.md`

**Step 1: Update docs**

Document that:

- `runtime-experiment compare` is the decision-support layer above run records
- deep comparison is opt-in through explicit snapshot artifacts
- the command remains non-autonomous and non-mutating

**Step 2: Verify doc references**

Run: `rg -n "runtime-experiment|compare" docs/product-specs/runtime-experiment.md docs/ROADMAP.md`

Expected: matches include the new compare wording.

### Task 5: Run final verification and prepare delivery

**Files:**
- Modify: staged task files only

**Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: PASS.

**Step 2: Run focused test coverage**

Run: `cargo test -p loongclaw-daemon --test integration runtime_experiment`

Expected: PASS.

**Step 3: Run daemon clippy coverage**

Run: `cargo clippy -p loongclaw-daemon --all-targets -- -D warnings`

Expected: PASS.

**Step 4: Commit**

```bash
git add docs/plans/2026-03-16-alpha-test-runtime-experiment-compare-design.md \
  docs/plans/2026-03-16-alpha-test-runtime-experiment-compare-implementation-plan.md \
  crates/daemon/src/runtime_experiment_cli.rs \
  crates/daemon/tests/integration/cli_tests.rs \
  crates/daemon/tests/integration/runtime_experiment_cli.rs \
  docs/product-specs/runtime-experiment.md \
  docs/ROADMAP.md
git commit -m "feat(daemon): add runtime experiment compare"
```
