# Delegate Runtime Envelope V2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make delegate child runtime posture real by persisting a typed child runtime-narrowing contract and applying it to actual kernel-bound core-tool execution for `web.fetch` and `browser.*`.

**Architecture:** Reuse the existing constrained-subagent execution envelope. Add typed runtime-narrowing config under `tools.delegate.child_runtime`, attach it to `SessionContext`, inject it through trusted `_loongclaw` payload context, and derive an effective `ToolRuntimeConfig` inside `execute_tool_core_with_config(...)`.

**Tech Stack:** Rust, serde/serde_json, `ToolRuntimeConfig`, conversation runtime/session context, delegate lifecycle persistence, kernel-bound turn execution, existing delegate/session/tool tests.

---

## Task 1: Add red tests for runtime-narrowed child execution

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Add a failing session inspection test**

Add a test proving `session_status` exposes persisted child `runtime_narrowing` under the
constrained delegate lifecycle metadata.

**Step 2: Add a failing child web policy test**

Add a kernel-bound delegate test where the base runtime allows a broader web surface, but the child
runtime narrowing allows only a stricter host set or denies private hosts. Verify the child tool
execution fails with the narrowed policy.

**Step 3: Add a failing child browser limit test**

Add a test proving a child session uses narrowed browser limits, for example a one-session cap or a
lower link/text ceiling than the parent runtime.

**Step 4: Add a failing trusted-payload forgery test**

Add a test proving untrusted callers cannot inject `_loongclaw.runtime_narrowing` into core-tool
payloads to self-escalate or spoof child posture.

**Step 5: Run one focused red test**

Run a focused delegate-runtime test and confirm it fails for the expected reason before any
production changes.

## Task 2: Implement the typed runtime-narrowing contract

**Files:**
- Modify: `crates/app/src/config/tools.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/conversation/subagent.rs`

**Step 1: Add delegate child runtime config types**

Add nested config types under `tools.delegate.child_runtime` for:

- `web`
- `browser`

Use optional numeric/boolean fields where inheritance must remain distinguishable from explicit
narrowing.

**Step 2: Add runtime-narrowing runtime types**

Add typed narrowing structs to `tools/runtime_config.rs` and implement the parent-to-child merge
logic there.

The merge must be monotonic:

- numeric values clamp downward
- blocked domains union
- allow domains intersect when both sides restrict
- private-host access can only stay allowed when both parent and child allow it

**Step 3: Extend the constrained subagent execution envelope**

Persist the runtime-narrowing snapshot on `ConstrainedSubagentExecution`.

## Task 3: Carry child runtime narrowing into actual core-tool execution

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/turn_engine.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Extend `SessionContext`**

Add optional child runtime narrowing to `SessionContext` and load it from the child session’s
persisted delegate lifecycle event.

**Step 2: Inject trusted internal runtime context**

Update the tool-payload augmentation path so child core-tool calls inject trusted internal
runtime-narrowing context alongside existing internal execution metadata.

**Step 3: Apply the effective runtime config**

Update `execute_tool_core_with_config(...)` to derive an effective runtime config from:

- the base runtime config
- trusted internal child runtime narrowing, when present

Keep untrusted `_loongclaw` rejection behavior intact.

## Task 4: Reuse the persisted contract in inspection and docs

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Modify: `docs/plans/2026-03-18-delegate-runtime-envelope-v2-design.md`
- Modify: `docs/plans/2026-03-18-delegate-runtime-envelope-v2-implementation-plan.md`

**Step 1: Surface `runtime_narrowing` in session inspection**

Update `session_status` / delegate lifecycle extraction to expose the persisted narrowing snapshot.

**Step 2: Keep design/plan docs aligned**

Adjust the design or plan if testing reveals a smaller safe seam than the initial draft.

## Task 5: Verify and ship

**Files:**
- Modify: GitHub issue / PR artifacts after code lands

**Step 1: Run focused tests**

Run the new delegate-runtime and inspection tests exactly.

**Step 2: Run adjacent regressions**

Run delegate, session-status, browser, and web-fetch regression coverage that exercises the touched
paths.

**Step 3: Run repository verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

**Step 4: Prepare GitHub delivery**

Open or reuse a follow-up issue for delegate runtime-envelope v2, assign the operator, and open a
stacked PR that references `#275` and links the new issue explicitly.
