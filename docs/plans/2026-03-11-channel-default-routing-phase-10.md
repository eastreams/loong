# Channel Default Routing Phase 10 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make LoongClaw's default-account routing source explicit and surface
the risky implicit multi-account fallback through `channels` and `doctor`.

**Architecture:** Add a reusable default-account selection result with
provenance in channel config, propagate that provenance into status snapshots,
and emit doctor warnings only when multiple configured accounts rely on a
sorted fallback default.

**Tech Stack:** Rust, serde, clap

---

### Task 1: Add failing default-selection provenance tests

**Files:**
- Modify: `crates/app/src/config/channels.rs`

Add tests that prove:

- explicit `default_account` resolves to `explicit_default`
- a configured `default` account resolves to `mapped_default`
- single-account compatibility resolves to `runtime_identity`
- multi-account implicit routing resolves to `fallback`

Run:

```bash
cargo test -p loongclaw-app default_account_selection_source
```

Expected: FAIL before implementation.

### Task 2: Add failing operator-surface tests

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/tests/mod.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

Add tests that prove:

- snapshots serialize `default_account_source`
- `channels` text output shows the source
- `doctor` warns when multi-account routing depends on fallback default

Run:

```bash
cargo test -p loongclaw-app default_account_source
cargo test -p loongclaw-daemon default_account_source
```

Expected: FAIL before implementation.

### Task 3: Implement default-account selection provenance

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/config/mod.rs`

Implement a reusable selection result and source enum shared by Telegram and
Feishu/Lark default-account resolution.

### Task 4: Implement channel status and doctor warnings

**Files:**
- Modify: `crates/app/src/channel/registry.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

Propagate selection provenance into channel snapshots, notes, text output, and
doctor checks.

### Task 5: Verification

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
