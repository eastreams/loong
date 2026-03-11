# Channel Account Hardening Phase 9 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reject ambiguous multi-account channel configs and expose default
account selection explicitly in operator-facing channel status.

**Architecture:** Extend structured config validation with channel-account
integrity diagnostics, then annotate channel status snapshots with
default-account visibility while keeping valid runtime selection behavior
unchanged.

**Tech Stack:** Rust, serde, clap

---

### Task 1: Add failing config validation tests

**Files:**
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/config/runtime.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- duplicate normalized Telegram account ids are rejected
- duplicate normalized Feishu account ids are rejected
- invalid `default_account` is rejected when accounts are configured
- structured diagnostics expose the new validation codes and account variables

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-app config_validation_rejects_duplicate_normalized
cargo test -p loongclaw-app validate_file_returns_channel_account_diagnostics
```

Expected: FAIL before implementation.

### Task 2: Add failing default-visibility tests

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/tests/mod.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- channel snapshots mark which configured account is the default
- `channels` text output prints the default-account marker
- single-account fallback does not invent a synthetic configured account from
  `default_account`

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-app channel_status_snapshots_mark_default
cargo test -p loongclaw-daemon render_channel_snapshots_text_reports_default
```

Expected: FAIL before implementation.

### Task 3: Implement channel-account config validation

**Files:**
- Modify: `crates/app/src/config/shared.rs`
- Modify: `crates/app/src/config/channels.rs`

**Step 1: Extend validation code catalog**

Add generic channel-account validation codes for:

- duplicate normalized account ids
- unknown default account

**Step 2: Implement validation helpers**

Use the same normalization logic as runtime routing to:

- detect collisions across raw account keys
- reject unknown defaults when account maps exist

**Step 3: Keep valid behavior stable**

Ensure valid configs still resolve accounts exactly as before.

### Task 4: Implement default-account visibility

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

**Step 1: Annotate snapshots**

Add `is_default_account` to `ChannelStatusSnapshot` and include a note for it.

**Step 2: Render the marker**

Update text and JSON surfaces so operators can see which configured account is
the default selection.

### Task 5: Verification

Run:

```bash
cargo test -p loongclaw-app multi_account
cargo test -p loongclaw-app config::
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-daemon tests::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: PASS

### Expected Remaining Gap After Phase 9

After this phase, the next unresolved channel architecture gap is no longer
account selection integrity. It is the shared long-running runtime substrate
needed for richer adapters such as Discord:

- monitor/supervisor traits
- event stream lifecycle management
- multi-account concurrent runtime orchestration
