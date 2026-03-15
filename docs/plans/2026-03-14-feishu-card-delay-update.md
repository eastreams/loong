# Feishu Card Delay Update Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a secure Feishu delayed card update tool that uses callback tokens captured by the Feishu webhook without exposing those tokens in model-visible ingress context.

**Architecture:** Keep callback transport parsing in `channel/feishu`, add a private tool-only callback context to channel ingress plumbing, implement the delayed update API in `app/src/feishu/resources/*`, and expose a narrowly scoped `feishu.card.update` tool only when Feishu runtime/config is available.

**Tech Stack:** Rust, serde/serde_json, existing Feishu client/runtime, LoongClaw tool registry and ingress injection path

---

### Task 1: Add failing tests for private callback tool context injection

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/feishu/webhook.rs`

**Step 1: Write the failing test**

Add a test proving:

- Feishu callback turns inject private callback metadata into `_loongclaw`
- the model-visible system note does not include the callback token

**Step 2: Run the focused test to verify it fails**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_callback_private_ingress --target-dir target/codex-feishu-card-delay-update-red -- --nocapture
```

Expected:

- FAIL because callback private context does not exist yet

**Step 3: Implement the minimal private context plumbing**

Add channel delivery / ingress private context support without exposing the token in `system_note()`.

**Step 4: Re-run the focused test**

Run the same command and confirm PASS.

### Task 2: Add failing resource-client tests for delayed card update

**Files:**
- Create: `crates/app/src/feishu/resources/cards.rs`
- Modify: `crates/app/src/feishu/resources/mod.rs`
- Modify: `crates/app/src/feishu/resources/types.rs`
- Modify: `crates/app/src/feishu/mod.rs`

**Step 1: Write the failing tests**

Add tests covering:

- request validation for missing token
- request normalization for `open_ids`
- success response parsing for `{ "code": 0, "msg": "ok" }`

**Step 2: Run the focused tests to verify they fail**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_card_update --target-dir target/codex-feishu-card-delay-update-red -- --nocapture
```

Expected:

- FAIL because the delayed update helper does not exist yet

**Step 3: Implement the delayed update helper**

Add:

- request type
- response type
- API call helper
- response parsing helper

**Step 4: Re-run the focused tests**

Run the same command and confirm PASS.

### Task 3: Add failing tool tests for `feishu.card.update`

**Files:**
- Modify: `crates/app/src/tools/feishu.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests covering:

- tool registry/provider schema includes `feishu.card.update`
- tool defaults `account_id` and `callback_token` from Feishu callback ingress
- tool defaults `open_ids` to the callback operator open_id when omitted
- explicit `callback_token` overrides ingress default
- tool uses tenant auth and hits `/open-apis/interactive/v1/card/update`

**Step 2: Run the focused tests to verify failure**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app feishu_card_update_tool --target-dir target/codex-feishu-card-delay-update-red -- --nocapture
```

Expected:

- FAIL because the tool is not registered or executable yet

**Step 3: Implement the tool**

Add:

- tool alias
- payload schema
- example shape
- execution path
- ingress private callback default extraction

**Step 4: Re-run the focused tests**

Run the same command and confirm PASS.

### Task 4: Full verification

**Files:**
- Modify only files already touched above

**Step 1: Run formatter**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all
```

**Step 2: Run full app suite serially**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app --target-dir target/codex-feishu-card-delay-update-full -- --nocapture --test-threads=1
```

Expected:

- PASS

**Step 3: Run full daemon suite**

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-daemon --target-dir target/codex-feishu-card-delay-update-full -- --nocapture
```

Expected:

- PASS

**Step 4: Run diff hygiene**

Run:

```bash
git diff --check
```

Expected:

- clean
