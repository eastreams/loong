# Cross-Repo Runtime Architecture Convergence Implementation Plan

**Goal:** Sequence the next stage of LoongClaw runtime work so governed-path
closure, session/memory upgrades, app-layer decomposition, tool
productization, and approval unification land in the right order without
duplicating or invalidating existing leaf plans.

**Architecture:** Add one convergence-layer execution plan above the existing
leaf plans. Keep implementation work in bounded slices. Reuse prior design and
implementation artifacts where they still describe the right local seam.

**Tech Stack:** Rust, Cargo workspace, Tokio tests, repository docs,
GitHub issue-first workflow

---

## Why This Slice Exists

The repository already contains many strong leaf plans for memory, governed
paths, conversation runtime, and tool productization. What it does not yet
contain is one explicit sequencing artifact that says:

1. which themes matter together
2. which existing plans remain authoritative
3. what should land first
4. what should not be merged into one oversized patch

This slice fills that gap.

## Evidence / Inputs

Primary internal anchors:

- `crates/kernel/src/kernel.rs`
- `crates/kernel/src/policy.rs`
- `crates/app/src/context.rs`
- `crates/app/src/conversation/runtime_binding.rs`
- `crates/app/src/conversation/context_engine.rs`
- `crates/app/src/memory/system.rs`
- `crates/app/src/tools/mod.rs`
- `crates/app/src/conversation/turn_coordinator.rs`
- `crates/app/src/channel/mod.rs`
- `crates/app/src/acp/manager.rs`

Primary comparison inputs:

- `codex`
- `openclaw`
- `pi-mono`
- `nanobot`
- `hermes-agent`

Primary comparison lessons:

1. typed tool/runtime products matter
2. durable session and memory storage matter
3. approval contracts matter
4. control-plane monoliths become long-term debt

## Execution Tasks

### Task 1: Land the convergence-layer docs

**Files:**
- Create: `docs/plans/2026-04-03-cross-repo-runtime-architecture-convergence-design.md`
- Create: `docs/plans/2026-04-03-cross-repo-runtime-architecture-convergence-implementation-plan.md`

**Step 1: Write the scope and sequencing docs**

Capture:

1. the five convergence themes
2. the execution order
3. the mapping from each theme to existing leaf plans
4. the comparison-repo lessons worth importing
5. the anti-patterns worth avoiding

**Step 2: Review for duplication**

Confirm the new docs do not restate leaf-plan detail that already exists in:

- governed-path plans
- memory architecture plans
- session runtime reintegration plans
- tool productization plans
- approval plans

**Step 3: Keep the docs additive**

Ensure the new docs behave as a sequencing layer, not a replacement layer.

**Step 4: Verify docs formatting**

Run:

- `cargo fmt --all -- --check`

Expected:

- PASS

### Task 2: Create the tracking issue before PR delivery

**Files:**
- Create temporary issue body file outside the repository tree

**Step 1: Search for an existing issue**

Search for an existing issue that already tracks:

- governed path closure
- session/memory runtime redesign
- tool scheduling/productization
- approval unification

Expected:

- either reuse the existing issue or confirm no equivalent umbrella issue exists

**Step 2: Open a feature issue using the repository template**

The issue should describe:

1. the runtime sequencing problem
2. why existing leaf plans need a convergence layer
3. the five themes
4. the expected operator and contributor value

**Step 3: Add the operator as an assignee**

Use additive assignment only.

Expected:

- the issue exists
- the operator is assigned
- the issue body is English
- the issue body is template-compliant

### Task 3: Review leaf-plan references and tighten wording

**Files:**
- Modify: `docs/plans/2026-04-03-cross-repo-runtime-architecture-convergence-design.md`
- Modify: `docs/plans/2026-04-03-cross-repo-runtime-architecture-convergence-implementation-plan.md`

**Step 1: Check each referenced plan path**

Validate that every referenced leaf plan exists and belongs to the correct
theme.

**Step 2: Remove any duplicated implementation detail**

If a section repeats detailed steps already present in a leaf plan, replace the
detail with a short summary and a direct reference.

**Step 3: Re-check sequence logic**

Verify the order:

1. governed paths
2. session/memory substrate
3. control-plane decomposition
4. tool productization
5. approval unification

still holds after wording cleanup.

### Task 4: Open a PR that links and closes the tracking issue

**Files:**
- Use the repository PR template

**Step 1: Commit the scoped doc change**

Stage only:

- `docs/plans/2026-04-03-cross-repo-runtime-architecture-convergence-design.md`
- `docs/plans/2026-04-03-cross-repo-runtime-architecture-convergence-implementation-plan.md`

**Step 2: Draft the PR body from the template**

The PR should make these points:

1. Problem:
   the repository has strong leaf plans, but no cross-theme sequencing layer
2. Why it matters:
   without sequencing, later runtime work risks overlap and hotspot growth
3. What changed:
   added a convergence design note and an umbrella implementation plan
4. What did not change:
   no runtime behavior changed
   no leaf plan was replaced

**Step 3: Add the closing clause**

Use:

- `Closes #<issue-number>`

Expected:

- PR body is template-compliant
- PR scope remains documentation only
- issue linkage is explicit

## Validation

Run:

- `cargo fmt --all -- --check`
- `git status --short`
- `git diff --cached --name-only`
- `git diff --cached`

Expected:

- formatting gate stays green
- staged diff contains only the two new plan docs

## Delivery

Issue-first delivery order:

1. create or reuse the tracking issue
2. assign the operator
3. commit the scoped doc slice
4. open the PR with an explicit closing clause

Review expectations:

1. reviewers should check whether the sequencing logic is defensible
2. reviewers should check whether the new docs are additive instead of duplicative
3. reviewers should check whether the five themes match the current repository risk surface
