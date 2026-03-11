# Channel Runtime Context Phase 12 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract shared runtime-entry preparation and serve-lifecycle handling
for Telegram and Feishu/Lark channels.

**Architecture:** Introduce a typed `ChannelCommandContext` for config load,
resolved-account data, and route provenance, then introduce a shared
`with_channel_serve_runtime` wrapper that owns tracker startup/shutdown around
serve bodies.

**Tech Stack:** Rust, tokio, serde

---

### Task 1: Add failing tests for shared context and serve runtime

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/runtime_state.rs`

Add tests that prove:

- command-context builders preserve route metadata and reject disabled accounts
- shared serve runtime persists running state during execution and shutdown state
  after execution

### Task 2: Implement shared command context

**Files:**
- Modify: `crates/app/src/channel/mod.rs`

Add `ChannelCommandContext<R>` plus typed Telegram / Feishu builders that
centralize config load, account resolution, route derivation, and disabled
account rejection.

### Task 3: Implement shared serve runtime wrapper

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/runtime_state.rs`

Add a shared runtime lifecycle wrapper and migrate Telegram / Feishu serve flows
to use it.

### Task 4: Verification

Run:

```bash
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-app config::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
