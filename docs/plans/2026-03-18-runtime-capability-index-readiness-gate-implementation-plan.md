# Runtime Capability Index and Readiness Gate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a read-only `runtime-capability index` layer that groups candidate artifacts into deterministic capability families and evaluates readiness as `ready`, `not_ready`, or `blocked`.

**Architecture:** Reuse the existing daemon-side artifact loading pattern from `runtime_capability_cli`, derive family identity from normalized proposal intent instead of creating a new persisted family artifact, and compute readiness from explicit evidence checks that stay fully deterministic and auditable.

**Tech Stack:** Rust, `clap`, `serde`, daemon integration tests, Markdown docs

---

### Task 1: Write the design and planning artifacts

**Files:**
- Create: `docs/plans/2026-03-18-runtime-capability-index-readiness-gate-design.md`
- Create: `docs/plans/2026-03-18-runtime-capability-index-readiness-gate-implementation-plan.md`

**Step 1: Write the design doc**

Describe the capability-family concept, the deterministic family fingerprint,
the compact evidence digest, the readiness checks, the non-goals, and the
testing strategy.

**Step 2: Verify the design artifact exists**

Run: `test -f docs/plans/2026-03-18-runtime-capability-index-readiness-gate-design.md`
Expected: exit 0

**Step 3: Write the implementation plan**

Capture the exact file changes, tests, and verification steps below.

**Step 4: Verify the plan artifact exists**

Run: `test -f docs/plans/2026-03-18-runtime-capability-index-readiness-gate-implementation-plan.md`
Expected: exit 0

### Task 2: Add failing CLI parsing coverage

**Files:**
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Write the failing parse tests**

Extend runtime-capability CLI parsing coverage so `index` parses:

- `--root`
- `--json`

**Step 2: Run the targeted parsing test to verify RED**

Run: `cargo test -p loongclaw-daemon --test integration runtime_capability_cli_parses_propose_review_show_index_and_plan -- --nocapture`
Expected: FAIL because `index` is not parsed yet

### Task 3: Add failing runtime-capability integration coverage

**Files:**
- Modify: `crates/daemon/tests/integration/runtime_capability_cli.rs`

**Step 1: Write the failing index tests**

Add integration tests covering:

- one family aggregates two accepted candidates with the same promotion intent
- the aggregated family reports compact evidence counts and metric ranges
- one accepted candidate alone yields `not_ready`
- a mix of accepted and rejected reviewed candidates yields `blocked`
- non-capability files under the scan root are ignored

**Step 2: Run the targeted runtime-capability tests to verify RED**

Run: `cargo test -p loongclaw-daemon --test integration runtime_capability_ -- --nocapture`
Expected: FAIL because the index surface does not exist yet

### Task 4: Implement the new read-only index surface

**Files:**
- Modify: `crates/daemon/src/runtime_capability_cli.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`

**Step 1: Add the new command and option struct**

Implement:

- `RuntimeCapabilityCommands::Index`
- `RuntimeCapabilityIndexCommandOptions`

**Step 2: Add the family/report model**

Implement:

- family summary types
- evidence digest types
- readiness check types
- readiness status enums

**Step 3: Implement candidate discovery and grouping**

Implement recursive root scanning, supported artifact loading, deterministic
family-id derivation, and grouping by normalized proposal intent.

**Step 4: Implement readiness evaluation**

Implement the explicit check pipeline:

- review consensus
- stability
- accepted-source integrity
- warning pressure

and derive overall readiness from those checks.

**Step 5: Implement JSON/text rendering**

Keep text output compact and review-first while exposing enough evidence to see
why a family is `ready`, `not_ready`, or `blocked`.

**Step 6: Run the targeted tests to verify GREEN**

Run: `cargo test -p loongclaw-daemon --test integration runtime_capability_ -- --nocapture`
Expected: PASS

### Task 5: Update product docs and roadmap references

**Files:**
- Modify: `docs/product-specs/runtime-capability.md`
- Modify: `docs/ROADMAP.md`

**Step 1: Update the product spec**

Document `runtime-capability index` as the aggregation/readiness layer above
individual candidate records and below any future promotion planner.

**Step 2: Update the roadmap**

Describe the new index/readiness layer as the next governed self-evolution step
after candidate records and before any dry-run promotion planning.

**Step 3: Run doc spot checks**

Run: `rg -n "runtime-capability|readiness|promotion" docs/product-specs/runtime-capability.md docs/ROADMAP.md`
Expected: the new index/readiness references appear in the updated docs

### Task 6: Verify and prepare clean delivery

**Files:**
- Review all touched files

**Step 1: Run focused verification**

Run:

- `cargo test -p loongclaw-daemon --test integration runtime_capability_ -- --nocapture`
- `cargo test -p loongclaw-daemon --test integration runtime_capability_cli_parses_propose_review_show_index_and_plan -- --nocapture`

Expected: PASS

**Step 2: Run repo-level verification as capacity allows**

Run the strongest available checks that are not blocked by unrelated global
cargo lock contention:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`

If cargo lock contention persists, record the blocker and supplement with direct
integration-binary verification for the touched daemon surface.

**Step 3: Inspect the final diff**

Run:

- `git status --short`
- `git diff --stat`
- `git diff`

Expected: only runtime-capability index/readiness code, tests, and directly
related docs
