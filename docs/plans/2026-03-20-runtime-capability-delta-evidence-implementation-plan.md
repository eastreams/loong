# Runtime Capability Delta Evidence Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Persist snapshot-backed delta evidence inside runtime-capability artifacts and surface a compact family-level digest in `show`, `index`, and `plan`.

**Architecture:** Reuse the existing runtime-experiment snapshot comparison logic as the single source of truth. Extend the capability artifact with an optional candidate-level delta, aggregate a minimal digest at family level, and keep readiness policy unchanged.

**Tech Stack:** Rust, Cargo, Serde, Clap, daemon integration tests

---

### Task 1: Lock the replacement design into the branch

**Files:**
- Create: `docs/plans/2026-03-20-runtime-capability-delta-evidence-design.md`
- Create: `docs/plans/2026-03-20-runtime-capability-delta-evidence-implementation-plan.md`

**Step 1: Verify the design doc exists**

Run: `test -f docs/plans/2026-03-20-runtime-capability-delta-evidence-design.md`
Expected: exit code 0

**Step 2: Verify the implementation plan exists**

Run: `test -f docs/plans/2026-03-20-runtime-capability-delta-evidence-implementation-plan.md`
Expected: exit code 0

### Task 2: Add failing propose-path tests

**Files:**
- Modify: `crates/daemon/tests/integration/runtime_capability_cli.rs`

**Step 1: Add a fixture helper that finishes an experiment with a real compareable delta**

Reuse the existing snapshot/config helpers instead of building new broad test
infrastructure.

**Step 2: Add a failing test for persisted snapshot delta**

Test name:

`runtime_capability_propose_persists_snapshot_delta_when_recorded_snapshots_exist`

Assertions:

- `candidate.source_run.snapshot_delta.is_some()`
- `changed_surface_count > 0`
- `changed_surfaces` includes at least one expected surface such as
  `provider_active_profile`

**Step 3: Run it to verify RED**

Run:

`cargo test -p loongclaw-daemon runtime_capability_propose_persists_snapshot_delta_when_recorded_snapshots_exist --test integration -- --nocapture`

Expected: FAIL because `snapshot_delta` does not yet exist on the capability
artifact

**Step 4: Add a failing no-snapshot test**

Test name:

`runtime_capability_propose_leaves_snapshot_delta_empty_without_recorded_snapshots`

Assertions:

- `candidate.source_run.snapshot_delta.is_none()`

**Step 5: Run it to verify RED**

Run:

`cargo test -p loongclaw-daemon runtime_capability_propose_leaves_snapshot_delta_empty_without_recorded_snapshots --test integration -- --nocapture`

Expected: FAIL until the new field exists

**Step 6: Add a failing broken-snapshot test**

Test name:

`runtime_capability_propose_rejects_broken_recorded_snapshot_delta`

Setup:

- create a finished run with recorded snapshot paths
- delete or corrupt one referenced snapshot artifact after the run is persisted

Assertion:

- `execute_runtime_capability_propose_command(...)` returns an error mentioning
  the unresolved recorded snapshot path

**Step 7: Run it to verify RED**

Run:

`cargo test -p loongclaw-daemon runtime_capability_propose_rejects_broken_recorded_snapshot_delta --test integration -- --nocapture`

Expected: FAIL until the propose path validates the recorded delta properly

### Task 3: Add failing index, plan, and text-output tests

**Files:**
- Modify: `crates/daemon/tests/integration/runtime_capability_cli.rs`

**Step 1: Add a failing family digest test**

Test name:

`runtime_capability_index_reports_delta_evidence_digest`

Assertions:

- `family.evidence.delta_candidate_count == 2`
- `family.evidence.changed_surfaces` is stable and sorted

**Step 2: Run it to verify RED**

Run:

`cargo test -p loongclaw-daemon runtime_capability_index_reports_delta_evidence_digest --test integration -- --nocapture`

Expected: FAIL because the digest fields do not exist yet

**Step 3: Add a failing plan-output test**

Test name:

`runtime_capability_plan_surfaces_delta_evidence_digest`

Assertions:

- `plan.evidence.delta_candidate_count == 2`
- `plan.evidence.changed_surfaces` matches the family digest

**Step 4: Run it to verify RED**

Run:

`cargo test -p loongclaw-daemon runtime_capability_plan_surfaces_delta_evidence_digest --test integration -- --nocapture`

Expected: FAIL until plan carries the new digest

**Step 5: Add a failing show-text rendering test**

Test name:

`runtime_capability_show_text_renders_snapshot_delta_summary`

Assertions:

- rendered text includes changed surface count
- rendered text includes changed surface names

**Step 6: Run it to verify RED**

Run:

`cargo test -p loongclaw-daemon runtime_capability_show_text_renders_snapshot_delta_summary --test integration -- --nocapture`

Expected: FAIL until text rendering is updated

### Task 4: Implement candidate-level delta persistence

**Files:**
- Modify: `crates/daemon/src/runtime_capability_cli.rs`
- Modify: `crates/daemon/src/runtime_experiment_cli.rs`

**Step 1: Expose one reusable helper from the runtime-experiment path**

Add a helper that derives the recorded snapshot delta for a finished run without
duplicating compare semantics.

**Step 2: Extend `RuntimeCapabilitySourceRunSummary`**

Add:

```rust
pub snapshot_delta: Option<RuntimeExperimentSnapshotDelta>
```

**Step 3: Populate the field in `build_source_run_summary(...)`**

Rules:

- `None` when there is no recorded result snapshot or no recorded artifact path
- `Some(delta)` when both recorded snapshot artifacts are resolvable
- propagate errors when recorded artifact paths exist but cannot be loaded

**Step 4: Re-run the propose-focused tests**

Run:

- `cargo test -p loongclaw-daemon runtime_capability_propose_persists_snapshot_delta_when_recorded_snapshots_exist --test integration -- --nocapture`
- `cargo test -p loongclaw-daemon runtime_capability_propose_leaves_snapshot_delta_empty_without_recorded_snapshots --test integration -- --nocapture`
- `cargo test -p loongclaw-daemon runtime_capability_propose_rejects_broken_recorded_snapshot_delta --test integration -- --nocapture`

Expected: PASS

### Task 5: Implement the family-level delta digest

**Files:**
- Modify: `crates/daemon/src/runtime_capability_cli.rs`

**Step 1: Extend `RuntimeCapabilityEvidenceDigest`**

Add:

- `delta_candidate_count: usize`
- `changed_surfaces: Vec<String>`

**Step 2: Aggregate the digest in `build_family_evidence_digest(...)`**

Rules:

- count only artifacts with `snapshot_delta.is_some()`
- aggregate a stable sorted union of changed surface names
- leave readiness evaluation unchanged

**Step 3: Re-run the digest tests**

Run:

- `cargo test -p loongclaw-daemon runtime_capability_index_reports_delta_evidence_digest --test integration -- --nocapture`
- `cargo test -p loongclaw-daemon runtime_capability_plan_surfaces_delta_evidence_digest --test integration -- --nocapture`

Expected: PASS

### Task 6: Surface the evidence in show, index, and plan output

**Files:**
- Modify: `crates/daemon/src/runtime_capability_cli.rs`

**Step 1: Update `render_runtime_capability_text(...)`**

Add compact summary lines for:

- changed surface count
- changed surface names

**Step 2: Update `render_runtime_capability_index_text(...)`**

Add compact summary lines for:

- `delta_evidence_candidates`
- `delta_changed_surfaces`

**Step 3: Update `render_runtime_capability_promotion_plan_text(...)`**

Reuse the same compact summary values from the family evidence digest.

**Step 4: Re-run the rendering test**

Run:

`cargo test -p loongclaw-daemon runtime_capability_show_text_renders_snapshot_delta_summary --test integration -- --nocapture`

Expected: PASS

### Task 7: Update docs to describe the shipped evidence layer

**Files:**
- Modify: `docs/product-specs/runtime-capability.md`
- Modify: `docs/ROADMAP.md`

**Step 1: Update the product spec**

Document that capability artifacts can preserve snapshot-backed runtime delta
evidence and that families expose a compact changed-surface digest.

**Step 2: Update the roadmap**

Move the delta-evidence wording from “remaining deliverables” to the delivered
runtime-capability slice.

**Step 3: Verify docs mention the new layer**

Run:

`rg -n "snapshot-backed|delta evidence|changed_surfaces|runtime-capability" docs/product-specs/runtime-capability.md docs/ROADMAP.md`

Expected: matches in both files

### Task 8: Run focused and broader verification

**Files:**
- Modify only the delta-evidence implementation and doc files

**Step 1: Run the focused runtime-capability suite**

Run:

`cargo test -p loongclaw-daemon runtime_capability_ --test integration -- --nocapture`

Expected: PASS

**Step 2: Run format check**

Run:

`cargo fmt --all -- --check`

Expected: PASS

**Step 3: Run strict daemon-targeted lint if needed**

Run:

`cargo clippy -p loongclaw-daemon --tests -- -D warnings`

Expected: PASS

### Task 9: Prepare replacement GitHub delivery

**Files:**
- Modify only the replacement slice files

**Step 1: Inspect isolation**

Run:

- `git status --short`
- `git diff --cached --name-only`
- `git diff --cached`

Expected: only runtime-capability delta-evidence files are included

**Step 2: Commit**

Suggested commit:

`git commit -m "feat: persist runtime capability delta evidence"`

**Step 3: Push to fork and open replacement PR**

Branch:

`issue-346-runtime-capability-delta-evidence`

PR must:

- target `dev`
- include `Closes #346`
- explain that the delta-evidence subset from the old stacked `#348` never
  landed in `dev`
- summarize validation commands exactly
