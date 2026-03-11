# Channel Multi-Account Phase 8 Implementation Plan

**Goal:** Add OpenClaw-style multi-account configuration and default-account
selection for Telegram and Feishu/Lark, wire CLI account selection through
runtime startup, and expose per-account channel status in `channels` and
`doctor`.

**Architecture:** Keep Phase 7 runtime identity semantics, but add a new
configured-account selection layer. Resolve one configured account, merge base
config with account overrides, derive runtime identity from the merged config,
and report channel status per configured account.

**Tech Stack:** Rust, serde, clap, tokio

### Task 1: Add failing multi-account config tests

**Files:**
- Modify: `crates/app/src/config/channels.rs`

Add tests that prove:

- Telegram lists configured account ids from `accounts`
- Telegram resolves `default_account`
- Telegram merges top-level config with account overrides
- Feishu does the same
- single-account fallback remains compatible

Run:

```bash
cargo test -p loongclaw-app config::channels::tests::multi_account
```

Expected: FAIL before implementation.

### Task 2: Add failing per-account registry and doctor tests

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/tests/mod.rs`

Add tests that prove:

- `channel_status_snapshots()` emits one snapshot per configured account
- snapshots expose configured account selection ids
- `doctor` scopes duplicate check names by account when multiple accounts exist
- `channels` text rendering shows configured account selection ids

Run:

```bash
cargo test -p loongclaw-app channel::registry::tests::multi_account
cargo test -p loongclaw-daemon multi_account
```

Expected: FAIL before implementation.

### Task 3: Implement config selection helpers

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/config/mod.rs`

Implement:

- `default_account` and `accounts` for Telegram and Feishu
- account override structs
- helpers to list configured account ids
- helpers to resolve default account ids
- helpers to resolve one merged account config

### Task 4: Implement CLI account selection and startup wiring

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/telegram.rs`
- Modify: `crates/app/src/channel/feishu/mod.rs`
- Modify: `crates/app/src/channel/feishu/adapter.rs`
- Modify: `crates/app/src/channel/feishu/webhook.rs`

Implement:

- `--account` on `telegram-serve`, `feishu-send`, and `feishu-serve`
- resolved account selection passed into adapter/runtime startup
- merged config used instead of raw top-level config

### Task 5: Implement per-account registry and doctor surfaces

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/main.rs`

Implement:

- one channel snapshot per configured account
- configured account metadata in JSON/text output
- account-scoped doctor check naming when one platform exposes multiple accounts

### Task 6: Verification

Run:

```bash
cargo test -p loongclaw-app config::
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-daemon tests::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: PASS

### Expected Remaining Gap After Phase 8

Even after Phase 8, LoongClaw will still need a stronger shared monitor/event
substrate before Discord is worth implementing. This plan intentionally stops
before that layer.
