# Runtime Capability Promotion Plan Dry-Run Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a read-only `runtime-capability plan` layer that resolves one indexed capability family into a deterministic dry-run promotion plan without mutating runtime state.

**Architecture:** Reuse the existing candidate loading and family-readiness logic in `runtime_capability_cli`, derive one promotion-plan report from one family summary, and keep the new surface strictly read-only so later mutation layers can consume an explicit plan contract instead of improvising promotion details.

**Tech Stack:** Rust, `clap`, `serde`, daemon integration tests, Markdown docs

---

### Task 1: Write the design and planning artifacts

**Files:**
- Create: `docs/plans/2026-03-18-runtime-capability-promotion-plan-dry-run-design.md`
- Create: `docs/plans/2026-03-18-runtime-capability-promotion-plan-dry-run-implementation-plan.md`

**Step 1: Write the design doc**

Describe the planner contract, the command surface, the planned-artifact model,
the promotable rule, blockers, approval checklist, rollback hints, provenance,
and why this slice stays read-only.

**Step 2: Verify the design artifact exists**

Run: `test -f docs/plans/2026-03-18-runtime-capability-promotion-plan-dry-run-design.md`
Expected: exit 0

**Step 3: Write the implementation plan**

Capture the exact file changes, tests, docs, and verification steps below.

**Step 4: Verify the plan artifact exists**

Run: `test -f docs/plans/2026-03-18-runtime-capability-promotion-plan-dry-run-implementation-plan.md`
Expected: exit 0

### Task 2: Add failing CLI parsing coverage

**Files:**
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Write the failing parse test**

Extend runtime-capability CLI parsing coverage so `plan` parses:

- `--root`
- `--family-id`
- `--json`

**Step 2: Run the targeted parsing test to verify RED**

Run: `cargo test -p loongclaw-daemon runtime_capability_cli_parses_propose_review_show_index_and_plan --test integration -- --exact integration::cli_tests::runtime_capability_cli_parses_propose_review_show_index_and_plan --nocapture`
Expected: FAIL because the `plan` subcommand does not exist yet

### Task 3: Add failing runtime-capability planner integration coverage

**Files:**
- Modify: `crates/daemon/tests/integration/runtime_capability_cli.rs`

**Step 1: Write the failing planner tests**

Add integration tests covering:

- planning a ready managed-skill family
- planning a `not_ready` family and surfacing missing-evidence blockers
- planning a `blocked` family and surfacing hard-stop blockers
- rejecting an unknown `family_id`
- exposing deterministic artifact metadata, approval checklist, rollback hints,
  and provenance references

**Step 2: Run the targeted runtime-capability tests to verify RED**

Run: `cargo test -p loongclaw-daemon runtime_capability_plan --test integration -- --nocapture`
Expected: FAIL because the planner surface does not exist yet

### Task 4: Implement the read-only planner surface

**Files:**
- Modify: `crates/daemon/src/runtime_capability_cli.rs`
- Modify: `crates/daemon/src/lib.rs`

**Step 1: Add the new command and option struct**

Implement:

- `RuntimeCapabilityCommands::Plan`
- `RuntimeCapabilityPlanCommandOptions`

**Step 2: Add the promotion-plan report model**

Implement:

- promotion-plan report type
- planned-artifact type
- provenance type

Reuse existing proposal, evidence, and readiness types instead of cloning that
logic into a second schema.

**Step 3: Implement family lookup and plan derivation**

Implement:

- family resolution by `family_id`
- deterministic planned-artifact identity derivation
- promotable boolean derived from readiness
- blockers derived from non-pass readiness checks
- approval checklist and rollback hints derived from target kind plus generic
  governance rules
- provenance extraction from indexed candidate evidence

**Step 4: Implement JSON/text rendering**

Keep text output compact and review-first while exposing:

- promotability
- target/artifact description
- blockers
- approval checklist
- rollback hints
- provenance references

**Step 5: Run the targeted tests to verify GREEN**

Run:

- `cargo test -p loongclaw-daemon runtime_capability_plan --test integration -- --nocapture`
- `cargo test -p loongclaw-daemon runtime_capability_cli_parses_propose_review_show_index_and_plan --test integration -- --exact integration::cli_tests::runtime_capability_cli_parses_propose_review_show_index_and_plan --nocapture`

Expected: PASS

### Task 5: Update product docs and roadmap references

**Files:**
- Modify: `docs/product-specs/runtime-capability.md`
- Modify: `docs/ROADMAP.md`

**Step 1: Update the product spec**

Document `runtime-capability plan` as the dry-run planning layer above family
readiness and below any future promotion executor.

**Step 2: Update the roadmap**

Describe the planner as the next shipped governed self-evolution step after the
family index/readiness layer.

**Step 3: Run doc spot checks**

Run: `rg -n "runtime-capability|promotion planner|dry-run" docs/product-specs/runtime-capability.md docs/ROADMAP.md`
Expected: the new planner references appear in the updated docs

### Task 6: Verify and prepare clean delivery

**Files:**
- Review all touched files

**Step 1: Run focused verification**

Run:

- `cargo test -p loongclaw-daemon runtime_capability_ --test integration -- --nocapture`

Expected: PASS

**Step 2: Run repo-level verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --all-features`

Expected: PASS

**Step 3: Inspect the final diff**

Run:

- `git status --short`
- `git diff --stat`
- `git diff`

Expected: only runtime-capability planner code, tests, docs, and directly
related GitHub delivery text are present
