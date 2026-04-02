# PR677 Scope-In Review Convergence Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Converge the remaining scope-in review and lint issues on PR `#677` without expanding into the separately-tracked `tools/mod.rs` structural-budget follow-up.

**Architecture:** Treat the current red signals in two buckets. Scope-in covers the `rust-quality` unused-import regression and the two documentation-alignment comments that match shipped `bash.exec` behavior. Scope-out covers the `governance` / `build` failures rooted in the `crates/app/src/tools/mod.rs` structural-size breach, which the original 2026-04-01 cleanup plan already classifies as a separate follow-up rather than work to absorb in this slice.

**Tech Stack:** Rust, Cargo/Clippy, Markdown plans, GitHub PR checks/review metadata.

---

## File Structure

- Modify: `crates/app/src/tools/bash_governance.rs`
  - Keep `CompiledRuleOrigin` imported only in test scope so strict lint stays green.
- Modify: `docs/plans/2026-03-27-bash-exec-basic-tool-implementation-plan.md`
  - Align bash visibility examples with shipped `is_discoverable()` behavior.
- Modify: `docs/plans/2026-03-29-bash-governance-ast-prefix-rule-implementation-plan.md`
  - Fix the final `git add` example so it stages the plan file it says was edited.
- Modify: `docs/plans/2026-04-02-pr677-scope-in-review-convergence-plan.md`
  - Check off progress and preserve the scope classification for handoff.

## Scope Check

### Scope In

- `rust-quality` failure caused by the non-test import of `CompiledRuleOrigin`
- CodeRabbit comment to switch bash visibility doc examples from `is_runtime_ready()` to `is_discoverable()`
- CodeRabbit comment to include the AST governance implementation plan file in the final staged file list
- Re-validating those edits with strict lint / diff hygiene

### Scope Out

- Any attempt to shrink or restructure `crates/app/src/tools/mod.rs`
- Any refresh of `docs/releases/architecture-drift-2026-04.md` that is only needed because of a `tools/mod.rs` budget change
- Absorbing the `governance` / `build` failures caused by the `tools_mod` structural-size breach into this slice

### Task 1: Reconfirm the current PR failure split against the original cleanup plan

**Files:**
- Modify: `docs/plans/2026-04-02-pr677-scope-in-review-convergence-plan.md`

- [x] **Step 1: Re-read the original cleanup plan scope boundary**

Run:

- `sed -n '1,120p' docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`

Expected:

- the plan still lists docs / PR-artifact convergence as scope-in
- the plan does not authorize absorbing new behavior changes or unrelated structural work

- [x] **Step 2: Reconfirm the live failing checks**

Run:

- `gh pr checks 677 --repo loongclaw-ai/loongclaw`

Expected:

- current failures show `rust-quality`, `governance`, and meta `build`
- `build` is only a downstream summary of upstream failures

- [x] **Step 3: Record the classification used for this slice**

Record in the running status notes for this plan:

- `rust-quality` = scope-in
- `governance` rooted in `tools_mod` budget = blocking scope-out
- `build` = derived failure from upstream checks, not an independent fix target

### Task 2: Converge the scope-in review items already identified

**Files:**
- Modify: `crates/app/src/tools/bash_governance.rs`
- Modify: `docs/plans/2026-03-27-bash-exec-basic-tool-implementation-plan.md`
- Modify: `docs/plans/2026-03-29-bash-governance-ast-prefix-rule-implementation-plan.md`

- [x] **Step 1: Keep `CompiledRuleOrigin` test-only**

The target code shape is:

```rust
use super::bash_rules::{CompiledPrefixRule, PrefixRuleDecision};
```

and inside the `#[cfg(test)]` module:

```rust
use super::super::bash_rules::CompiledRuleOrigin;
```

- [x] **Step 2: Align bash visibility docs with shipped discoverability semantics**

Replace the plan snippets so the examples use:

```rust
ToolVisibilityGate::BashRuntime => config.bash_exec.is_discoverable(),
```

and:

```rust
"bash.exec" => config.bash_exec.is_discoverable(),
```

- [x] **Step 3: Fix the final AST-governance plan staging example**

The final command example must include:

```bash
git add docs/plans/2026-03-29-bash-governance-ast-prefix-rule-implementation-plan.md ...
```

- [x] **Step 4: Run diff hygiene immediately after the edits**

Run:

- `git diff --check`

Expected:

- PASS with no whitespace / patch-format issues

### Task 3: Verify the scope-in slice and stop for review-aware handoff

**Files:**
- Modify: `docs/plans/2026-04-02-pr677-scope-in-review-convergence-plan.md`

- [x] **Step 1: Run strict lint for the scope-in code change**

Run:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected:

- PASS
- the prior `unused import: CompiledRuleOrigin` failure is gone

- [x] **Step 2: Re-check the remaining red status without absorbing scope-out work**

Run:

- `gh pr checks 677 --repo loongclaw-ai/loongclaw`

Expected:

- `rust-quality` is no longer a legitimate blocker after the next CI run
- `governance` / `build` remain attributable to the separate `tools_mod` budget follow-up until the user explicitly expands scope

- [x] **Step 3: Run the required superpower review gate for this slice**

Follow the existing cleanup plan review mode:

- correctness review in `auto-fix`
- style review in `single-pass`

Expected:

- no remaining scope-in high / medium findings for this narrow slice
- any `tools/mod.rs` budget discussion is handed back as scope-out rather than silently fixed here

Current result:

- reviewer outcome: `No scope-in findings.`

## Self-Review

- Spec / request coverage:
  - user-requested plan restored before further implementation: yes
  - mistaken `mod.rs` scope expansion excluded: yes
  - current review comments and lint item captured: yes
- Placeholder scan:
  - no `TBD` / `TODO` placeholders
  - exact files and commands are named
- Type / behavior consistency:
  - the plan consistently treats `tools_mod` budget work as scope-out
  - the plan consistently treats the three CodeRabbit / lint items as scope-in

## Execution Handoff

Plan complete and saved to `docs/plans/2026-04-02-pr677-scope-in-review-convergence-plan.md`.

User confirmation was received, and the scope-in slice was executed with the following verification evidence:

- `git diff --check`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

The remaining `governance` / derived `build` failures stay scope-out for this slice because they track the separately-owned `tools/mod.rs` structural-budget follow-up.
