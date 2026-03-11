# Channel Registry Phase 3 Implementation Plan

**Goal:** Add a shared channel catalog and readiness snapshot layer, then expose it through daemon CLI and doctor checks.

**Architecture:** Centralize channel identity, aliases, operation metadata, and readiness evaluation in `loongclaw-app::channel`. Consume that surface from `loongclaw-daemon` instead of duplicating Telegram/Feishu channel knowledge in multiple places.

**Tech Stack:** Rust, serde, clap

### Task 1: Add registry and readiness tests

Add tests that prove:

- `lark` normalizes to the Feishu surface
- the catalog keeps Lark as a Feishu alias
- Telegram readiness becomes `ready` when token and allowlist are configured
- Feishu direct send and webhook serve can report different readiness states

### Task 2: Implement shared channel registry

Add:

- catalog metadata
- alias normalization
- per-operation health snapshots

Make the shared snapshot represent the actual operational surfaces:

- Telegram: `serve`
- Feishu/Lark: `send`, `serve`

### Task 3: Expose channel status in daemon CLI

Add a `channels` command that prints:

- channel identity
- aliases
- transport
- API base URL
- per-operation health, command name, and issues

Also add a daemon test that proves the new text output includes aliases and operation status.

### Task 4: Replace duplicated doctor channel checks

Use the shared readiness snapshots to drive doctor channel checks so the daemon no longer reimplements Telegram/Feishu config logic separately.

### Task 5: Verify locally

Run:

```bash
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-daemon tests::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
