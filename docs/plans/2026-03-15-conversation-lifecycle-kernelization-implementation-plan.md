# Conversation Lifecycle Kernelization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make conversation lifecycle hooks kernel-bound by removing optional kernel context from `ConversationRuntime` and `ConversationContextEngine` lifecycle methods, while keeping direct fallback behavior for context assembly, provider requests, and turn persistence.

**Architecture:** Change lifecycle trait signatures to require `&KernelContext`, then update conversation call sites to make the skip-vs-run decision explicitly at the runtime binding boundary. Keep `build_context`, `build_messages`, `request_turn`, `request_completion`, and `persist_turn` unchanged in this slice.

**Tech Stack:** Rust, async conversation runtime traits, `loongclaw-app` tests, cargo test, cargo clippy

---

### Task 1: Lock phase 2 scope in docs

**Files:**
- Create: `docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md`
- Create: `docs/plans/2026-03-15-conversation-lifecycle-kernelization-implementation-plan.md`

**Step 1: Re-read the lifecycle surfaces**

Run: `rg -n "bootstrap\\(|ingest\\(|after_turn\\(|compact_context\\(|prepare_subagent_spawn\\(|on_subagent_ended\\(" crates/app/src/conversation`
Expected: all lifecycle call sites and trait declarations are enumerated.

**Step 2: Confirm the new docs exist**

Run: `ls docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md docs/plans/2026-03-15-conversation-lifecycle-kernelization-implementation-plan.md`
Expected: both files exist.

### Task 2: Add failing tests for the new lifecycle contract

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Write the failing runtime lifecycle delegation test**

Add a test that passes a real `KernelContext` and proves `bootstrap`, `ingest`,
`prepare_subagent_spawn`, and `on_subagent_ended` still delegate through the runtime.

**Step 2: Write the failing no-kernel skip regression**

Add a focused regression proving that no-kernel persistence still writes turns but no longer calls
ingest.

**Step 3: Run the targeted tests and confirm RED**

Run: `cargo test -p loongclaw-app default_runtime_delegates_bootstrap_and_ingest_to_context_engine_with_kernel -- --exact --test-threads=1`
Expected: FAIL because the lifecycle signatures and call sites still accept `Option<&KernelContext>`.

### Task 3: Change lifecycle trait signatures

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/context_engine.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Change runtime lifecycle methods to require `&KernelContext`**

Update `ConversationRuntime` and `DefaultConversationRuntime` for:
- `bootstrap`
- `ingest`
- `after_turn`
- `compact_context`
- `prepare_subagent_spawn`
- `on_subagent_ended`

**Step 2: Change context-engine lifecycle methods to require `&KernelContext`**

Update `ConversationContextEngine` and boxed forwarding impls for the same lifecycle set.

**Step 3: Update fake runtimes and recording engines**

Adjust test doubles to match the new kernel-bound signatures.

### Task 4: Move skip-vs-run decisions to callers

**Files:**
- Modify: `crates/app/src/conversation/persistence.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/tests.rs`

**Step 1: Gate ingestion at the persistence boundary**

Keep `persist_turn(...)` running regardless of kernel, but only call `runtime.ingest(...)` when a
kernel is present.

**Step 2: Gate bootstrap / after-turn / compaction / subagent lifecycle at callers**

Update runtime call sites so they:
- call lifecycle hooks with `&KernelContext` when present
- skip cleanly when kernel is absent

**Step 3: Run targeted lifecycle tests**

Run: `cargo test -p loongclaw-app default_runtime_delegates_ -- --test-threads=1`
Expected: PASS.

### Task 5: Re-run targeted conversation flows

**Files:**
- Modify: `crates/app/src/conversation/tests.rs` only if additional coverage is needed

**Step 1: Run turn-finalization and lifecycle-sensitive tests**

Run: `cargo test -p loongclaw-app handle_turn_with_runtime -- --test-threads=1`
Expected: PASS.

**Step 2: Run any additional checkpoint or subagent lifecycle coverage if needed**

Run: `cargo test -p loongclaw-app turn_checkpoint -- --test-threads=1`
Expected: PASS if affected.

### Task 6: Run full verification and refresh GitHub delivery

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/context_engine.rs`
- Modify: `crates/app/src/conversation/persistence.rs`
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/tests.rs`
- Create: `docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md`
- Create: `docs/plans/2026-03-15-conversation-lifecycle-kernelization-implementation-plan.md`

**Step 1: Run package tests**

Run: `cargo test -p loongclaw-app -- --test-threads=1`
Expected: PASS.

**Step 2: Run lint**

Run: `cargo clippy -p loongclaw-app --all-targets --all-features -- -D warnings`
Expected: PASS.

**Step 3: Review the scoped diff**

Run: `git diff -- crates/app/src/conversation/runtime.rs crates/app/src/conversation/context_engine.rs crates/app/src/conversation/persistence.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs docs/plans/2026-03-15-conversation-lifecycle-kernelization-design.md docs/plans/2026-03-15-conversation-lifecycle-kernelization-implementation-plan.md`
Expected: only the intended lifecycle-kernelization slice is present.
