# Channel Runtime Phase 2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace stringly-typed channel session and target handling with small structured runtime types, and model Feishu/Lark as a single domain-aware channel configuration surface.

**Architecture:** Introduce typed channel platform, session, and outbound target types in the channel module while keeping the existing conversation runtime keyed by a derived session string. Add a domain-aware Feishu config model so `lark` becomes a first-class configuration variant with explicit base URL resolution.

**Tech Stack:** Rust, serde, async-trait, tokio, axum, reqwest

---

### Task 1: Add failing tests for structured channel runtime types

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Test: `crates/app/src/channel/mod.rs`

**Step 1: Write the failing test**

Add tests that prove:

- `ChannelSession` derives a stable session key from platform and conversation id
- optional thread id extends the key deterministically
- `ChannelOutboundTarget` preserves platform, target kind, and id

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::tests::channel_session_key_is_stable`

Expected: FAIL because the structured runtime types do not exist yet.

**Step 3: Write minimal implementation**

Add:

- `ChannelPlatform`
- `ChannelSession`
- `ChannelOutboundTargetKind`
- `ChannelOutboundTarget`

with helper constructors and a `session_key()` method.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::tests::channel_session_key_is_stable`

Expected: PASS

### Task 2: Convert existing channel loop and adapters to structured targets

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/telegram.rs`
- Modify: `crates/app/src/channel/feishu/adapter.rs`
- Modify: `crates/app/src/channel/feishu/payload/types.rs`
- Modify: `crates/app/src/channel/feishu/payload/inbound.rs`
- Modify: `crates/app/src/channel/feishu/webhook.rs`
- Test: `crates/app/src/channel/mod.rs`
- Test: `crates/app/src/channel/telegram.rs`
- Test: `crates/app/src/channel/feishu/payload/tests.rs`

**Step 1: Write the failing test**

Add or update tests to prove:

- `process_channel_batch` delivers through structured outbound targets
- Telegram inbound messages create a telegram conversation target
- Feishu inbound messages create a reply-message target instead of a raw string

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::tests::process_channel_batch_acknowledges_after_successful_delivery`

Expected: FAIL because adapters and loop still depend on raw string targets.

**Step 3: Write minimal implementation**

Update:

- `ChannelInboundMessage`
- `ChannelAdapter::send_text`
- provider handoff to use `message.session.session_key()`
- Telegram parser and sender
- Feishu inbound payload normalization and sender

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app channel::tests::process_channel_batch_acknowledges_after_successful_delivery
cargo test -p loongclaw-app channel::telegram::tests::
cargo test -p loongclaw-app channel::feishu::payload::tests::
```

Expected: PASS

### Task 3: Add failing tests for Feishu/Lark domain-aware configuration

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/config/mod.rs`
- Test: `crates/app/src/config/mod.rs`

**Step 1: Write the failing test**

Add tests that prove:

- default Feishu config resolves to the Feishu domain URL
- `domain = "lark"` resolves to the Lark domain URL when no explicit base URL is set
- explicit `base_url` overrides the domain default

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app config::tests::feishu_lark_domain_uses_lark_base_url_when_base_url_not_set`

Expected: FAIL because the domain model does not exist yet.

**Step 3: Write minimal implementation**

Add:

- `FeishuDomain`
- optional `base_url`
- `resolved_base_url()`

and switch Feishu adapter construction to the resolved URL helper.

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p loongclaw-app config::tests::feishu_defaults_are_stable
cargo test -p loongclaw-app config::tests::feishu_lark_domain_uses_lark_base_url_when_base_url_not_set
cargo test -p loongclaw-app config::tests::feishu_explicit_base_url_overrides_domain_default
```

Expected: PASS

### Task 4: Re-run targeted and broad verification

**Files:**
- Modify: `docs/plans/2026-03-11-channel-runtime-phase-2-design.md`
- Modify: `docs/plans/2026-03-11-channel-runtime-phase-2.md`

**Step 1: Run targeted tests**

Run:

```bash
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-app config::tests::feishu_
```

Expected: PASS

**Step 2: Run broad verification**

Run:

```bash
cargo test -p loongclaw-app
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: PASS
