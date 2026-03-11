# Channel Reliability Phase 1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate the current Telegram and Feishu silent-drop paths and add a minimal channel delivery acknowledgement seam for future channel-runtime work.

**Architecture:** Add a lightweight delivery metadata path to `ChannelInboundMessage` plus default adapter acknowledgement hooks. Use those hooks to delay Telegram offset persistence until successful processing and to convert Feishu dedupe from fire-once to retry-safe processing state.

**Tech Stack:** Rust, async-trait, tokio, axum, reqwest, serde_json

---

### Task 1: Document and expose the minimal delivery contract

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Test: `crates/app/src/channel/mod.rs`

**Step 1: Write the failing test**

Add a unit test that constructs a `ChannelInboundMessage` with delivery metadata and verifies the
default adapter acknowledgement hooks are no-op success paths.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::tests::channel_adapter_default_ack_hooks_are_noop`

Expected: FAIL because the delivery metadata and acknowledgement hooks do not exist yet.

**Step 3: Write minimal implementation**

Add:

- a delivery metadata struct on `ChannelInboundMessage`
- default no-op `ack_inbound` and `complete_batch` methods on `ChannelAdapter`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::tests::channel_adapter_default_ack_hooks_are_noop`

Expected: PASS

### Task 2: Make Telegram offset persistence retry-safe

**Files:**
- Modify: `crates/app/src/channel/telegram.rs`
- Modify: `crates/app/src/channel/mod.rs`
- Test: `crates/app/src/channel/telegram.rs`

**Step 1: Write the failing test**

Add tests that prove:

- polling a batch does not immediately persist the next offset
- acknowledging one delivered message advances offset only through that message
- completing an empty or partially ignored batch advances the trailing offset

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::telegram::tests::telegram_batch_offset_is_not_persisted_until_ack`

Expected: FAIL because polling currently writes offset immediately.

**Step 3: Write minimal implementation**

Update Telegram polling state to keep batch offset pending, acknowledge successful messages, and
flush the trailing cursor only on batch completion.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::telegram::tests::telegram_batch_offset_is_not_persisted_until_ack`

Expected: PASS

### Task 3: Make Feishu dedupe retry-safe

**Files:**
- Modify: `crates/app/src/channel/feishu/webhook.rs`
- Test: `crates/app/src/channel/feishu/webhook.rs`

**Step 1: Write the failing test**

Add tests that prove:

- the cache distinguishes `processing` and `completed`
- duplicate events are suppressed while an event is in progress
- a failed event can be released and accepted on retry

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::feishu::webhook::tests::recent_cache_releases_failed_events_for_retry`

Expected: FAIL because the cache only supports seen/not-seen semantics.

**Step 3: Write minimal implementation**

Replace the simple `insert_if_new` cache with processing/completed state helpers, and wire the
webhook handler to confirm success or release on error.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::feishu::webhook::tests::recent_cache_releases_failed_events_for_retry`

Expected: PASS

### Task 4: Wire the new delivery hooks into the Telegram loop

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Test: `crates/app/src/channel/mod.rs`

**Step 1: Write the failing test**

Add a test adapter that records hook calls and verify the channel loop acknowledges each message
only after successful send and completes the batch after a successful pass.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::tests::telegram_loop_acknowledges_after_successful_delivery`

Expected: FAIL because the loop does not invoke acknowledgement hooks.

**Step 3: Write minimal implementation**

Call `ack_inbound` after each successful message send and `complete_batch` when the loop finishes
the batch without error.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::tests::telegram_loop_acknowledges_after_successful_delivery`

Expected: PASS

### Task 5: Verify the scoped change set

**Files:**
- Modify: `docs/plans/2026-03-11-channel-reliability-design.md`
- Modify: `docs/plans/2026-03-11-channel-reliability-phase-1.md`

**Step 1: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app channel::telegram::
cargo test -p loongclaw-app channel::feishu::webhook::
cargo test -p loongclaw-app channel::tests::
```

Expected: PASS

**Step 2: Run broader crate verification**

Run:

```bash
cargo test -p loongclaw-app
```

Expected: PASS

**Step 3: Run formatting**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: PASS

**Step 4: Commit**

```bash
git add docs/plans/2026-03-11-channel-reliability-design.md \
        docs/plans/2026-03-11-channel-reliability-phase-1.md \
        crates/app/src/channel/mod.rs \
        crates/app/src/channel/telegram.rs \
        crates/app/src/channel/feishu/webhook.rs
git commit -m "fix: harden channel delivery reliability"
```
