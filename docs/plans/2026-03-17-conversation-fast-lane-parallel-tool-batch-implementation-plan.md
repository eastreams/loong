# Conversation Fast-Lane Parallel Tool Batch Execution Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to
> implement this plan task-by-task.

**Goal:** Add an opt-in, fail-closed parallel tool batch executor to the
conversation fast lane while preserving source-order result replay and leaving
safe-lane plan execution unchanged.

**Architecture:** Keep the patch local to conversation config, tool catalog
metadata, `TurnEngine`, and focused regression tests. Use explicit batch
preflight and constrained scheduling metadata instead of hidden executor
whitelists or a broad scheduler transplant.

**Tech Stack:** Rust, Tokio tests, `loongclaw-app`, GitHub issue-first workflow

---

## Task 1: Lock the delivery artifacts before code

**Files:**
- Create: `docs/plans/2026-03-17-conversation-fast-lane-parallel-tool-batch-design.md`
- Create: `docs/plans/2026-03-17-conversation-fast-lane-parallel-tool-batch-implementation-plan.md`

**Step 1: Confirm the target seams**

Run:
- `rg -n "execute_turn_in_context|turn_loop.max_tool_steps_per_round|fast_lane_max_tool_steps_per_turn" crates/app/src/conversation crates/app/src/config`
- `rg -n "ProviderTurn|Vec<ToolIntent>|parse_discovery_followup_leases_from_message_content" crates/app/src/conversation crates/app/src/provider`

Expected: the serial fast-lane executor, the step-limit defaults, and the
follow-up/discovery seams are all enumerated.

**Step 2: Create the GitHub tracking issue**

Open issue `#269` describing:
- the current serial short-circuit execution model
- the fail-closed batch barrier requirement
- the fast-lane-only scope boundary

Expected: issue exists before PR creation and will be linked by the PR body.

## Task 2: Add RED config coverage

**Files:**
- Modify: `crates/app/src/config/conversation.rs`
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/config/runtime.rs`

**Step 1: Add new config fields and helper stubs**

Add placeholders for:
- `fast_lane_parallel_tool_execution_enabled`
- `fast_lane_parallel_tool_execution_max_in_flight`

Do not implement executor behavior yet.

**Step 2: Add failing config tests**

Cover:
- stable defaults
- TOML override for both fields
- clamp behavior for `max_in_flight`
- config template output includes the new defaults

**Step 3: Run the config tests and confirm RED**

Run:
- `cargo test -p loongclaw-app --locked conversation_defaults_are_stable -- --exact`
- `cargo test -p loongclaw-app --locked conversation_fast_lane_parallel_tool_execution_can_be_overridden_from_toml -- --exact`
- `cargo test -p loongclaw-app --locked write_template_includes_fast_lane_parallel_tool_execution_defaults -- --exact`

Expected: FAIL before the implementation is complete.

## Task 3: Add RED batch-execution tests

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/provider/shape.rs`

**Step 1: Add a parallel execution regression test**

Use a controlled custom `AppToolDispatcher` and a batch of two
`sessions_list` intents to assert:
- the second execution starts before the first finishes
- final output lines stay in assistant source order

**Step 2: Add a fail-closed approval-barrier regression**

Use a batch with:
- first intent: `sessions_list`
- second intent: `delegate_async`

Assert the turn returns `NeedsApproval` and the first app-tool intent never
executes.

**Step 3: Add a fail-closed kernel-binding regression**

Use a batch with:
- first intent: `sessions_list`
- second intent: `file.read`
- binding: `ConversationRuntimeBinding::Direct()`

Assert the turn returns `ToolDenied("no_kernel_context")` and the first app
intent never executes.

**Step 4: Add a discovery-first compatibility regression**

Extend the `provider/shape.rs` test coverage so multi-line `[tool_result]`
assistant content with multiple `[ok]` envelopes preserves first-source-order
lease selection.

**Step 5: Run the targeted tests and confirm RED**

Run:
- `cargo test -p loongclaw-app --locked turn_engine_parallel_safe_app_batch_executes_concurrently_in_source_order -- --exact`
- `cargo test -p loongclaw-app --locked turn_engine_parallel_batch_fails_closed_before_governed_approval -- --exact`
- `cargo test -p loongclaw-app --locked turn_engine_parallel_batch_fails_closed_before_kernel_binding_error -- --exact`
- `cargo test -p loongclaw-app --locked provider_shape_discovery_followup_uses_first_lease_in_multiline_source_order -- --exact`

Expected: FAIL before executor changes land.

## Task 4: Add scheduling metadata to the tool catalog

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`

**Step 1: Define the scheduling enum**

Add:
- `ToolSchedulingClass::SerialOnly`
- `ToolSchedulingClass::ParallelSafe`

**Step 2: Thread scheduling metadata through descriptors**

Extend `ToolDescriptor` and `ToolCatalogEntry` so the scheduling class is
available from the resolved descriptor.

**Step 3: Mark the initial safe subset**

Set `ParallelSafe` for:
- `file.read`
- `tool.search`
- `web.fetch`
- `sessions_list`

Leave everything else `SerialOnly`.

**Step 4: Add or update catalog assertions**

Add focused tests that lock the scheduling class for the intended safe subset
and for a clearly serial-only tool such as `delegate_async`.

## Task 5: Refactor `TurnEngine` into explicit batch phases

**Files:**
- Modify: `crates/app/src/conversation/turn_engine.rs`

**Step 1: Add prepared batch data types**

Introduce internal structs that capture:
- original intent
- effective intent after `tool.invoke` unwrap
- effective request
- execution kind
- scheduling class

**Step 2: Implement sequential batch preparation**

Move all of these into a preflight phase:
- resolution
- ingress augmentation
- `tool.invoke` unwrap
- kernel binding requirement checks
- app approval checks
- scheduling classification

Return an immediate `TurnResult` on the first blocker and do not execute any
tool in that case.

**Step 3: Implement prepared sequential execution**

Keep a sequential executor for:
- feature disabled
- batch size `<= 1`
- any batch containing a `SerialOnly` intent

**Step 4: Implement prepared parallel execution**

Execute prepared intents concurrently with bounded `max_in_flight`, but
reconstruct the final output vector in assistant source order before formatting
the final text.

**Step 5: Keep existing output formatting**

Reuse the current envelope formatting helpers so the final text remains newline
joined `[status] {...}` entries.

## Task 6: Make config drive the fast-lane executor

**Files:**
- Modify: `crates/app/src/config/conversation.rs`
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs` only if needed for
  wiring existing limits cleanly

**Step 1: Finalize config defaults and helpers**

Implement:
- default `enabled = false`
- default `max_in_flight = 4`
- accessor clamp `max_in_flight.max(1)`

**Step 2: Thread config into fast-lane engine construction**

If `TurnEngine` needs constructor parameters for the new parallel settings,
wire them from the fast-lane execution path only. Do not change safe-lane plan
executor semantics in this slice.

## Task 7: Verify, review, commit, and publish

**Files:**
- Modify only the scoped fast-lane/config/catalog/test/docs files

**Step 1: Run targeted verification**

Run:
- `cargo test -p loongclaw-app --locked turn_engine_parallel_safe_app_batch_executes_concurrently_in_source_order -- --exact`
- `cargo test -p loongclaw-app --locked turn_engine_parallel_batch_fails_closed_before_governed_approval -- --exact`
- `cargo test -p loongclaw-app --locked turn_engine_parallel_batch_fails_closed_before_kernel_binding_error -- --exact`
- `cargo test -p loongclaw-app --locked provider_shape_discovery_followup_uses_first_lease_in_multiline_source_order -- --exact`

Expected: PASS

**Step 2: Run broader package verification**

Run:
- `cargo fmt --all`
- `cargo fmt --all -- --check`
- `cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings`
- `cargo test -p loongclaw-app --all-features -- --test-threads=1`

Expected: PASS

**Step 3: Inspect scoped git state**

Run:
- `git status --short`
- `git diff --cached --name-only`
- `git diff --cached`

Expected: only fast-lane parallel batch, config, catalog, tests, and plan/docs
changes are staged.

**Step 4: Commit and publish**

Commit docs and implementation in scoped commits, push
`issue-269-parallel-tool-batch-fast-lane`, open a PR, and include
`Closes #269` in the PR body.
