# Channel Runtime State Phase 4 Implementation Plan

**Goal:** Add persisted serve-operation runtime state, merge it into channel
registry snapshots, and expose it through daemon CLI without widening the
current channel architecture beyond Telegram and Feishu/Lark.

**Architecture:** Keep OpenClaw's separation of concerns:

- channel registry owns identity and readiness
- runtime tracker owns liveness and activity
- daemon CLI owns operator presentation

Store one runtime file per tracked operation so independent serve processes do
not trample each other.

**Tech Stack:** Rust, serde, tokio, axum, clap

### Task 1: Add runtime tracker and failing tests

Add tests that prove:

- runtime tracker persists run activity and shutdown state
- stale heartbeat data is reported as `running=false` and `stale=true`

Implement:

- `ChannelOperationRuntime`
- `ChannelOperationRuntimeTracker`
- persisted runtime load/write helpers

### Task 2: Merge runtime into registry snapshots

Extend channel catalog operations with `tracks_runtime` and merge runtime state
into tracked operations only.

Required behavior:

- Telegram `serve` exposes runtime
- Feishu `serve` exposes runtime
- Feishu `send` keeps `runtime=None`
- missing runtime files degrade to a stable default runtime view

### Task 3: Instrument long-running serve operations

Wire runtime tracking into:

- `run_telegram_channel(...)`
- `process_channel_batch(...)`
- `run_feishu_channel(...)`
- `feishu_webhook_handler(...)`

Required behavior:

- lifecycle start/shutdown updates are persisted
- in-flight run counts are incremented and decremented correctly
- failed Feishu webhook processing still releases dedupe state for retry

### Task 4: Expose runtime in operator surfaces

Update `loongclaw channels` text output to include:

- `running`
- `stale`
- `busy`
- `active_runs`
- `last_run_activity_at`
- `last_heartbeat_at`
- `pid`

Update daemon tests so the new runtime text surface stays stable.

### Task 5: Hardening and verification

Harden the runtime tracker so background state locks do not panic the process.

Run:

```bash
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-daemon tests::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected result: PASS
