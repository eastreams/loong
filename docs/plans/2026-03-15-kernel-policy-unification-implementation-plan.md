# Kernel Policy Unification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make tool-executing conversation paths in `alpha-test` kernel-mandatory by separating turn validation from kernel-bound execution and by removing optional kernel context from the inner fast-lane and safe-lane tool execution surfaces.

**Architecture:** Keep the current coordinator and turn-loop public entrypoints, but refactor the inner tool path into three explicit phases: validate the provider turn, bind kernel authority at the coordinator boundary, then execute tools only through kernel-required helpers. Preserve current failure-code semantics while tightening the runtime contract.

**Tech Stack:** Rust, async conversation runtime traits, `loongclaw-app` tests, cargo test, cargo clippy

---

### Task 1: Lock the design and target scope in docs

**Files:**
- Create: `docs/plans/2026-03-15-kernel-policy-unification-design.md`
- Create: `docs/plans/2026-03-15-kernel-policy-unification-implementation-plan.md`

**Step 1: Re-read the current execution hotspots**

Run: `rg -n "evaluate_turn|execute_turn\\(|execute_single_tool_intent|no_kernel_context|kernel_context_required" crates/app/src/conversation`
Expected: the validation/binding/execution split points are enumerated.

**Step 2: Confirm the new docs exist**

Run: `ls docs/plans/2026-03-15-kernel-policy-unification-design.md docs/plans/2026-03-15-kernel-policy-unification-implementation-plan.md`
Expected: both files exist.

### Task 2: Write failing tests for the new turn-engine contract

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Test: `crates/app/src/conversation/tests.rs`

**Step 1: Add a failing validation test for tool turns**

Add a test asserting that a known tool turn validates to an explicit execution-required state
instead of returning `kernel_context_required`.

**Step 2: Add a failing validation test for no-tool turns**

Add a test asserting that no-tool turns validate directly to final text.

**Step 3: Run the targeted tests and confirm RED**

Run: `cargo test -p loongclaw-app turn_engine_known_tool_validates_to_execution_required -- --exact --test-threads=1`
Expected: FAIL because the new validation result type does not exist yet.

### Task 3: Refactor `TurnEngine` into validation plus kernel-bound execution

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Introduce the new validation result type**

Add the smallest new enum needed to represent:
- final text
- execution required

Keep failure details in `TurnFailure`.

**Step 2: Replace the current validation API**

Refactor `evaluate_turn(...)` into a pure validation method that:
- returns final text directly
- returns execution-required for valid tool turns
- still denies unknown tools and step-limit violations

**Step 3: Make tool execution kernel-mandatory**

Change the inner execution API so the tool-executing method requires `&KernelContext`.

**Step 4: Run targeted turn-engine tests**

Run: `cargo test -p loongclaw-app turn_engine_ -- --test-threads=1`
Expected: PASS.

### Task 4: Bind kernel authority explicitly in fast-lane and safe-lane callers

**Files:**
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Add the failing safe-lane/coordinator regression test if needed**

If no existing test covers the new binding point clearly, add one focused regression that proves a
tool turn without kernel context is denied before execution begins.

**Step 2: Refactor fast-lane execution**

Update `turn_loop.rs` to:
- validate first
- require kernel only for execution-required turns
- preserve `no_kernel_context` as the external denial reason

**Step 3: Refactor safe-lane execution**

Update `turn_coordinator.rs` so:
- the safe-lane node executor stores `&KernelContext`, not `Option<&KernelContext>`
- the missing-kernel fallback happens before node execution starts

**Step 4: Run targeted coordinator coverage**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime_safe_lane -- --test-threads=1`
Expected: PASS.

### Task 5: Update stale security docs

**Files:**
- Modify: `docs/SECURITY.md`

**Step 1: Reconcile the security model with current code**

Update the "Current coverage" section so it reflects:
- kernel-bound shell and file policy extensions
- tool execution as a kernel-mediated path
- remaining optional-kernel gaps as architectural debt instead of outdated false statements

**Step 2: Check the doc diff**

Run: `git diff -- docs/SECURITY.md`
Expected: only the intended accuracy updates are present.

### Task 6: Run broader verification and prepare delivery

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/conversation/turn_loop.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `docs/SECURITY.md`
- Create: `docs/plans/2026-03-15-kernel-policy-unification-design.md`
- Create: `docs/plans/2026-03-15-kernel-policy-unification-implementation-plan.md`

**Step 1: Run focused crate tests**

Run: `cargo test -p loongclaw-app turn_engine_ -- --test-threads=1`
Expected: PASS.

**Step 2: Run coordinator-focused tests**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime_safe_lane -- --test-threads=1`
Expected: PASS.

**Step 3: Run package verification**

Run: `cargo test -p loongclaw-app -- --test-threads=1`
Expected: PASS.

**Step 4: Run lint**

Run: `cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings`
Expected: PASS.

**Step 5: Review the final scoped diff**

Run: `git diff -- crates/app/src/conversation/turn_engine.rs crates/app/src/conversation/turn_loop.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs docs/SECURITY.md docs/plans/2026-03-15-kernel-policy-unification-design.md docs/plans/2026-03-15-kernel-policy-unification-implementation-plan.md`
Expected: only the intended kernel-policy unification slice is present.
