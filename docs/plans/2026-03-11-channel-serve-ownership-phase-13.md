# Channel Serve Ownership Phase 13 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prevent duplicate active `serve` workers for the same channel account.

**Architecture:** Reuse account-scoped runtime-state summaries before tracker
startup and block `with_channel_serve_runtime(...)` when an active owner already
holds the serve slot. Permit startup when prior state is stale.

**Tech Stack:** Rust, tokio, serde

---

### Task 1: Add failing ownership-gate tests

**Files:**
- Modify: `crates/app/src/channel/mod.rs`

Add tests that prove:

- duplicate active runtime blocks shared serve wrapper startup
- stale runtime state does not block startup

### Task 2: Implement ownership gate

**Files:**
- Modify: `crates/app/src/channel/mod.rs`

Implement a shared pre-start ownership check and apply it inside the shared
serve runtime wrapper.

### Task 3: Verification

Run:

```bash
cargo test -p loongclaw-app channel::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
