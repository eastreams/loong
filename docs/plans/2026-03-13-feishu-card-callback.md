# Feishu Card Callback Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Feishu card callback webhook support that accepts new and legacy callback payloads, normalizes them into LoongClaw ingress, and returns Feishu-compliant callback responses without breaking existing message webhook handling.

**Architecture:** Keep callback transport semantics inside `channel/feishu`. Extend the webhook parser with a callback branch, add an internal callback event/response model, and reuse existing conversation processing only for normalized ingress and text reasoning. The webhook layer remains responsible for serializing Feishu callback responses within the 3-second contract.

**Tech Stack:** Rust, Axum webhook handler, serde/serde_json, existing LoongClaw conversation runtime, Feishu webhook transport code

---

### Task 1: Add failing parser tests for callback payload variants

**Files:**
- Modify: `crates/app/src/channel/feishu/payload/tests.rs`
- Modify: `crates/app/src/channel/feishu/payload/types.rs`

**Step 1: Write the failing tests**

Add tests covering:

- new callback payload with `header.event_type = "card.action.trigger"`
- legacy callback payload with top-level `open_message_id` and `action`
- token verification for new callback payloads via `header.token`

Expected assertions:

- parser returns a callback-specific action, not `Ignore`
- callback action preserves operator open_id, open_message_id, open_chat_id, and action metadata

**Step 2: Run tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_card_callback --target-dir target/codex-feishu-card-callback-red -- --nocapture
```

Expected:

- FAIL because callback event types are ignored or token lookup is wrong

**Step 3: Write minimal type scaffolding**

Add callback-specific enum/type placeholders in `payload/types.rs` so the tests can compile against intended shapes.

**Step 4: Run tests again**

Run the same command and confirm failures now point to behavior, not missing symbols.

**Step 5: Commit**

```bash
git add crates/app/src/channel/feishu/payload/tests.rs crates/app/src/channel/feishu/payload/types.rs
git commit -m "test: add feishu card callback parser coverage"
```

### Task 2: Implement callback payload parsing and normalization

**Files:**
- Modify: `crates/app/src/channel/feishu/payload/inbound.rs`
- Modify: `crates/app/src/channel/feishu/payload/types.rs`
- Modify: `crates/app/src/channel/feishu/payload/mod.rs`

**Step 1: Write one more failing test for ingress/session normalization**

Add a test that proves callback payloads normalize into:

- Feishu session/account context
- source message id
- callback summary text with action name/value

**Step 2: Run the focused test to verify failure**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_card_callback_payload_normalizes --target-dir target/codex-feishu-card-callback-red -- --nocapture
```

Expected:

- FAIL because callback parsing/normalization is not implemented

**Step 3: Implement parser support**

Implement:

- detection for new callback shape
- detection for legacy callback shape
- shared token verification helper accepting `header.token` and legacy top-level `token`
- normalized callback event builder

Keep parsing logic inside `payload/inbound.rs`.

**Step 4: Run focused parser tests**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_card_callback --target-dir target/codex-feishu-card-callback-red -- --nocapture
```

Expected:

- PASS for callback parser tests

**Step 5: Commit**

```bash
git add crates/app/src/channel/feishu/payload/inbound.rs crates/app/src/channel/feishu/payload/types.rs crates/app/src/channel/feishu/payload/mod.rs
git commit -m "feat: parse feishu card callback payloads"
```

### Task 3: Add failing webhook tests for callback HTTP response behavior

**Files:**
- Modify: `crates/app/src/channel/feishu/webhook.rs`

**Step 1: Write failing webhook integration tests**

Add tests covering:

- callback webhook returns `HTTP 200` with `{}` or valid toast body
- duplicate callback events are deduped with a safe success body
- callback provider failure still returns a transport-safe body if designed, or a clearly asserted failure response if not

Prefer one test that reuses the current mock-provider harness.

**Step 2: Run the focused webhook tests to verify failure**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_webhook_card_callback --target-dir target/codex-feishu-card-callback-red -- --nocapture
```

Expected:

- FAIL because webhook only handles `Inbound` message events

**Step 3: Commit**

```bash
git add crates/app/src/channel/feishu/webhook.rs
git commit -m "test: add feishu webhook callback response coverage"
```

### Task 4: Implement callback response transport in the webhook layer

**Files:**
- Modify: `crates/app/src/channel/feishu/webhook.rs`
- Modify: `crates/app/src/channel/feishu/payload/types.rs`
- Modify: `crates/app/src/channel/mod.rs` only if a minimal helper is genuinely required

**Step 1: Add the minimal callback response model**

Implement a Feishu-only response enum like:

```rust
enum FeishuCallbackResponse {
    Noop,
    Toast { kind: String, content: String },
    Card { toast: Option<...>, card: Value },
}
```

Keep it transport-local.

**Step 2: Wire callback branch into webhook handling**

Implement webhook flow:

- parse callback event
- dedupe by callback event id
- normalize ingress
- call conversation processing
- serialize a safe Feishu callback body

Initial MVP rule:

- return `{}` by default
- allow deterministic toast use only where transport chooses it explicitly

**Step 3: Run focused webhook tests**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_webhook_card_callback --target-dir target/codex-feishu-card-callback-green -- --nocapture
```

Expected:

- PASS

**Step 4: Commit**

```bash
git add crates/app/src/channel/feishu/webhook.rs crates/app/src/channel/feishu/payload/types.rs
git commit -m "feat: handle feishu card callback webhooks"
```

### Task 5: Update observability and documentation signals

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `docs/plans/2026-03-13-feishu-card-callback-design.md`

**Step 1: Write failing assertions for registry/doctor notes if needed**

Add a test proving Feishu channel status notes mention callback support once implemented.

**Step 2: Run focused tests to verify failure**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_status --target-dir target/codex-feishu-card-callback-green -- --nocapture
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-daemon doctor_feishu --target-dir target/codex-feishu-card-callback-green -- --nocapture
```

Expected:

- FAIL or missing callback support notes

**Step 3: Implement minimal note updates**

Add notes describing:

- callback webhook support enabled
- supported callback versions
- current response mode limitations if any

**Step 4: Re-run focused tests**

Run the same commands and expect PASS.

**Step 5: Commit**

```bash
git add crates/app/src/channel/registry.rs crates/daemon/src/doctor_cli.rs docs/plans/2026-03-13-feishu-card-callback-design.md
git commit -m "docs: surface feishu card callback support"
```

### Task 6: Full verification

**Files:**
- Modify only files already touched above

**Step 1: Run formatter**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all
```

Expected:

- success

**Step 2: Run app tools/channel test slices**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app channel::feishu --target-dir target/codex-feishu-card-callback-full -- --nocapture
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app conversation::tests::handle_turn_with_runtime_and_ingress_injects_system_note_and_persists_event --target-dir target/codex-feishu-card-callback-full -- --nocapture
```

Expected:

- PASS

**Step 3: Run full app suite serially**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app --target-dir target/codex-feishu-card-callback-full -- --nocapture --test-threads=1
```

Expected:

- PASS

**Step 4: Run daemon suite**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-daemon --target-dir target/codex-feishu-card-callback-full -- --nocapture
```

Expected:

- PASS

**Step 5: Run diff hygiene check**

Run:

```bash
git diff --check
```

Expected:

- no whitespace or merge-marker issues

**Step 6: Commit**

```bash
git add crates/app/src/channel/feishu crates/app/src/channel/registry.rs crates/daemon/src/doctor_cli.rs docs/plans/2026-03-13-feishu-card-callback-design.md docs/plans/2026-03-13-feishu-card-callback.md
git commit -m "feat: support feishu card callback webhooks"
```
