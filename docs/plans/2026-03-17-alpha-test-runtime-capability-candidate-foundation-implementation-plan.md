# Runtime Capability Candidate Foundation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a minimal `runtime-capability` record layer that derives one reusable capability candidate from one finished `runtime-experiment` run and records one explicit operator review decision.

**Architecture:** Reuse the existing daemon-side record-artifact pattern from `runtime_experiment_cli`, keep the new surface file-based and operator-controlled, and do not mutate live runtime config, managed skills, or profile notes. The new artifact stores a compact source-run summary plus proposal/review metadata so later automation has a stable contract.

**Tech Stack:** Rust, `clap`, `serde`, daemon CLI integration tests, Markdown product docs

---

## Implementation Tasks

### Task 1: Write the design and planning artifacts

**Files:**
- Create: `docs/plans/2026-03-17-alpha-test-runtime-capability-candidate-foundation-design.md`
- Create: `docs/plans/2026-03-17-alpha-test-runtime-capability-candidate-foundation-implementation-plan.md`

**Step 1: Write the design doc**

Describe the problem, goal, non-goals, command surface, artifact schema,
testing strategy, and recommendation.

**Step 2: Verify the design artifact exists**

Run: `test -f docs/plans/2026-03-17-alpha-test-runtime-capability-candidate-foundation-design.md`
Expected: exit 0

**Step 3: Write the implementation plan**

Capture the exact file changes, tests, commands, and verification steps below.

**Step 4: Verify the plan artifact exists**

Run: `test -f docs/plans/2026-03-17-alpha-test-runtime-capability-candidate-foundation-implementation-plan.md`
Expected: exit 0

### Task 2: Add failing CLI parsing coverage

**Files:**
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Write the failing parse tests**

Add parse coverage for:

- `runtime-capability propose`
- `runtime-capability review`
- `runtime-capability show`

The tests should assert:

- `--run`, `--output`, `--target`, `--target-summary`, `--bounded-scope`,
  repeated `--required-capability`, repeated `--tag`, and `--json` parse into
  the correct option struct for `propose`
- `--candidate`, `--decision`, `--review-summary`, repeated `--warning`, and
  `--json` parse into the correct option struct for `review`
- `--candidate` and `--json` parse into the correct option struct for `show`

**Step 2: Run the targeted parse tests to verify RED**

Run: `cargo test -p loongclaw-daemon --test integration runtime_capability_cli_ -- --nocapture`
Expected: FAIL because the command family and types do not exist yet

### Task 3: Add failing runtime-capability integration tests

**Files:**
- Create if needed: `crates/daemon/tests/integration/runtime_capability_cli.rs`
- Modify if preferred by repo convention: `crates/daemon/tests/integration/runtime_experiment_cli.rs`

**Step 1: Write the failing propose/show/review tests**

Cover:

- `propose` persists a capability-candidate artifact from a finished run
- proposal tags and required capabilities are normalized and deduplicated
- source run summary carries run id, experiment id, decision, mutation summary,
  metrics, warnings, and optional artifact path
- `propose` rejects a `planned` run
- `propose` rejects an unknown capability string
- `review` records one terminal decision and review summary
- `review` rejects double review
- `show` round-trips the persisted artifact

**Step 2: Run the targeted integration tests to verify RED**

Run: `cargo test -p loongclaw-daemon --test integration runtime_capability_ -- --nocapture`
Expected: FAIL because the runtime-capability code path does not exist yet

### Task 4: Implement the new daemon CLI surface

**Files:**
- Create: `crates/daemon/src/runtime_capability_cli.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`

**Step 1: Add the new command family and option structs**

Implement:

- `RuntimeCapabilityCommands`
- `RuntimeCapabilityProposeCommandOptions`
- `RuntimeCapabilityReviewCommandOptions`
- `RuntimeCapabilityShowCommandOptions`

Also add the command family to top-level daemon CLI parsing and dispatch.

**Step 2: Add the artifact structs and helpers**

Implement:

- schema, proposal, source-run summary, review, and artifact document structs
- normalized target enum
- normalized review-decision enum
- helper functions for timestamps, text validation, repeated-value normalization,
  capability parsing, artifact persistence, and text rendering

**Step 3: Implement `propose` with minimal run-derived logic**

`propose` should:

- load a finished `runtime-experiment` artifact
- reject unfinished runs or runs without evaluation
- build a compact source-run summary
- normalize tags and required capabilities
- compute a stable `candidate_id`
- persist the new artifact to the requested output path

**Step 4: Implement `review` and `show`**

`review` should:

- load the candidate artifact
- reject non-proposed candidates
- persist `reviewed_at`, terminal decision, summary, and warnings

`show` should:

- load the artifact
- print JSON or a stable text summary without mutation

**Step 5: Run targeted daemon tests to verify GREEN**

Run: `cargo test -p loongclaw-daemon --test integration runtime_capability_ runtime_experiment_cli_ -- --nocapture`
Expected: PASS

### Task 5: Update product docs and roadmap references

**Files:**
- Create: `docs/product-specs/runtime-capability.md`
- Modify: `docs/product-specs/index.md`
- Modify: `docs/ROADMAP.md`
- Modify if needed: `AGENTS.md`
- Modify if needed: `CLAUDE.md`

**Step 1: Add the product spec**

Describe `runtime-capability` as the review layer above `runtime-experiment`
and below future automation. Keep automatic mutation explicitly out of scope.

**Step 2: Wire the product spec index**

Add `Runtime Capability` to `docs/product-specs/index.md`.

**Step 3: Update the roadmap**

Mention the new capability-candidate record layer as a prerequisite for later
skill-optimization loops or governed promotion work.

**Step 4: Run doc spot checks**

Run: `rg -n "runtime-capability|runtime capability|skill-optimization" docs/product-specs docs/ROADMAP.md`
Expected: matches in the new/updated docs only

### Task 6: Verify, isolate, and commit

**Files:**
- Review all touched files

**Step 1: Run focused verification**

Run:

- `cargo test -p loongclaw-daemon --test integration runtime_capability_ runtime_experiment_cli_ -- --nocapture`
- `cargo test -p loongclaw-daemon runtime_capability -- --nocapture`

Expected: PASS

**Step 2: Run repo-level CI-parity checks**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --all-features`

Expected: PASS

**Step 3: Inspect the final diff**

Run:

- `git status --short`
- `git diff --cached --name-only`
- `git diff --cached`

Expected: only runtime-capability and directly related doc/test changes

**Step 4: Commit**

```bash
git add docs/plans/2026-03-17-alpha-test-runtime-capability-candidate-foundation-design.md \
  docs/plans/2026-03-17-alpha-test-runtime-capability-candidate-foundation-implementation-plan.md \
  docs/product-specs/runtime-capability.md \
  docs/product-specs/index.md \
  docs/ROADMAP.md \
  crates/daemon/src/runtime_capability_cli.rs \
  crates/daemon/src/lib.rs \
  crates/daemon/src/main.rs \
  crates/daemon/tests/integration/cli_tests.rs \
  crates/daemon/tests/integration/runtime_capability_cli.rs
git commit -m "feat(daemon): add runtime capability candidate records"
```
