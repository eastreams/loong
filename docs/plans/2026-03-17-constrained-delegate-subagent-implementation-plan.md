# Constrained Delegate Subagent Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn `delegate` / `delegate_async` into a bounded constrained-subagent primitive by wiring runtime lifecycle hooks, introducing a typed execution envelope, enforcing a direct active-child limit, and exposing the new metadata through session inspection.

**Architecture:** Keep the existing child-session persistence model and tool surface. Add a shared constrained-subagent contract in the conversation layer, persist that contract into delegate lifecycle events, enforce `max_active_children` at spawn time, and reuse the persisted envelope in `session_status`.

**Tech Stack:** Rust, serde/serde_json, conversation runtime + coordinator, sqlite-backed `SessionRepository`, session inspection tooling, existing delegate tests in `crates/app/src/conversation/tests.rs` and `crates/app/src/tools/session.rs`.

---

## Task 1: Add failing tests for the missing lifecycle contract

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Add a failing kernel-bound inline delegate lifecycle test**

Add a test proving that a kernel-bound `delegate` turn calls:

- `prepare_subagent_spawn(parent, child)` before the child turn runs
- `on_subagent_ended(parent, child)` after terminal completion

Verify both the recorded context-engine calls and the persisted child session shape.

**Step 2: Add a failing spawn-preparation failure test**

Add a test proving that if `prepare_subagent_spawn(...)` fails:

- the delegate tool returns an error
- no child session row is created
- no queued/started delegate lifecycle events are written

**Step 3: Add a failing direct-child concurrency test**

Add a test proving that a parent with `max_active_children` already-active direct children cannot launch another `delegate_async` child.

The failure should be deterministic and should not create a new child session.

**Step 4: Add a failing session inspection test**

Add a test proving `session_status` exposes the persisted constrained-subagent envelope from delegate lifecycle events, including:

- `mode`
- `depth`
- `max_depth`
- `active_children`
- `max_active_children`
- `timeout_seconds`
- `allow_shell_in_child`
- child tool allowlist metadata

**Step 5: Run one focused red test**

Run:

`cargo test -p loongclaw-app handle_turn_with_runtime_kernel_delegate_calls_subagent_lifecycle_hooks -- --exact --nocapture`

Expected: FAIL because the production delegate path currently never invokes the runtime lifecycle hooks.

## Task 2: Implement the constrained-subagent contract and runtime wiring

**Files:**
- Add: `crates/app/src/conversation/subagent.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Modify: `crates/app/src/config/tools.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/session/repository.rs`
- Modify: `crates/app/src/tools/session.rs`

**Step 1: Add the shared constrained-subagent types**

Create typed serde-backed structs/enums for:

- execution mode
- constraint envelope
- terminal reason

Keep the shape narrow and local to delegate execution.

**Step 2: Add the config knob and repository helper**

Add `tools.delegate.max_active_children` with a default.

Add a repository helper that counts active direct child sessions for a parent in `ready` or `running` state.

**Step 3: Wire inline and async delegate spawn through the new envelope**

Update `execute_delegate_tool(...)` and `execute_delegate_async_tool(...)` to:

- compute depth and active-child counts
- enforce both limits
- build the constrained-subagent envelope
- call `prepare_subagent_spawn(...)` for kernel-bound parents before child creation
- persist the envelope in `delegate_started` / `delegate_queued` payloads

**Step 4: Persist typed terminal reason metadata**

Update delegate terminal completion/failure/timeout and async spawn-failure paths so terminal events carry a typed reason payload instead of only free-form strings.

**Step 5: Reuse the persisted envelope in session inspection**

Update `session_status` / delegate lifecycle extraction to parse the constrained-subagent envelope from delegate lifecycle events and surface it in the inspection JSON.

## Task 3: Verify the slice and prepare GitHub delivery

**Files:**
- Modify: `docs/plans/2026-03-17-constrained-delegate-subagent-design.md`
- Modify: `docs/plans/2026-03-17-constrained-delegate-subagent-implementation-plan.md`

**Step 1: Run focused tests**

Run:

- `cargo test -p loongclaw-app handle_turn_with_runtime_kernel_delegate_calls_subagent_lifecycle_hooks -- --exact --nocapture`
- `cargo test -p loongclaw-app handle_turn_with_runtime_delegate_rejects_spawn_when_prepare_subagent_spawn_fails -- --exact --nocapture`
- `cargo test -p loongclaw-app handle_turn_with_runtime_delegate_async_rejects_when_active_child_limit_is_exhausted -- --exact --nocapture`
- `cargo test -p loongclaw-app session_status_exposes_constrained_delegate_envelope -- --exact --nocapture`

Expected: PASS

**Step 2: Run adjacent regressions**

Run:

- `cargo test -p loongclaw-app handle_turn_with_runtime_executes_delegate -- --nocapture`
- `cargo test -p loongclaw-app handle_turn_with_runtime_executes_delegate_async -- --nocapture`
- `cargo test -p loongclaw-app session_status_ -- --nocapture`

Expected: PASS

**Step 3: Run repository-grade verification**

Run:

- `cargo fmt --all`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`

Expected: PASS

**Step 4: Prepare GitHub delivery**

Reuse issue `#222` and update it with:

- why LoongClaw needed a typed constrained-subagent contract at the delegate seam
- why this slice adds `max_active_children` in addition to `max_depth`
- what remains intentionally out of scope relative to a full OpenClaw-style subagent runtime

Open a PR against `alpha-test` with `Closes #222` only if the implemented slice fully satisfies the issue scope. Otherwise use `Related #222` and describe the remaining follow-up explicitly.
