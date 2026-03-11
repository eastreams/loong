# Channel Account Identity Phase 7 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Telegram and Feishu/Lark channel runtime, session, and offset
state account-aware while preserving backward compatibility for existing config
and persisted state.

**Architecture:** Add a resolved account identity model at the config layer,
thread it into channel session keys and runtime persistence, then surface that
identity through registry and doctor output. Keep the config shape single-account
for now, but make the identity model future-compatible with later multi-account
expansion.

**Tech Stack:** Rust, Tokio, serde

---

### Task 1: Add failing config/account identity tests

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/config/mod.rs`

**Step 1: Write the failing test**

Add tests that prove:

- Telegram resolves `account_id` from explicit config when set
- Telegram otherwise derives `bot_<bot_id>` from a token
- Feishu resolves `account_id` from explicit config when set
- Feishu otherwise derives `<domain>_<app_id>`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app config::`

Expected: FAIL because account identity resolution does not exist yet

**Step 3: Write minimal implementation**

Add:

- optional `account_id` on Telegram/Feishu config
- resolved account identity helpers
- tests for sanitization and fallback behavior

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app config::`

Expected: PASS

### Task 2: Add failing session-key and Telegram offset tests

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/telegram.rs`

**Step 1: Write the failing test**

Add tests that prove:

- `ChannelSession` includes account identity in `session_key()`
- Telegram offset files are written under an account-specific path
- Telegram can still read legacy `telegram.offset` when account-specific state
  is absent

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::telegram::tests:: channel::tests::`

Expected: FAIL because account identity is not threaded into sessions or offset
storage

**Step 3: Write minimal implementation**

Add account identity to `ChannelSession` and update Telegram offset path
resolution with legacy fallback.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::telegram::tests:: channel::tests::`

Expected: PASS

### Task 3: Add failing account-aware runtime and registry tests

**Files:**
- Modify: `crates/app/src/channel/runtime_state.rs`
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/tests/mod.rs`

**Step 1: Write the failing test**

Add tests that prove:

- runtime files are written/read using account-aware keys
- registry snapshots expose the resolved account identity
- channels text rendering includes account information
- doctor runtime details include account information

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app channel::runtime_state::tests:: channel::registry::tests::`

Expected: FAIL because runtime and registry are still platform-scoped only

**Step 3: Write minimal implementation**

Thread resolved account identity into runtime tracker construction, runtime
loading, registry snapshots, and CLI/doctor rendering.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app channel::runtime_state::tests:: channel::registry::tests::`

Expected: PASS

### Task 4: Run targeted verification

**Files:**
- Modify only as needed by previous tasks

**Step 1: Run app channel/config tests**

Run:

```bash
cargo test -p loongclaw-app config::
cargo test -p loongclaw-app channel::
```

Expected: PASS

**Step 2: Run daemon tests**

Run:

```bash
cargo test -p loongclaw-daemon tests::
```

Expected: PASS

### Task 5: Run full quality gates

**Files:**
- No new files expected

**Step 1: Run workspace tests**

Run:

```bash
cargo test --workspace --all-features
```

Expected: PASS

**Step 2: Run lint and format gates**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: PASS
