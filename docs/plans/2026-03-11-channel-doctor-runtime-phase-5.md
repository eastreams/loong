# Channel Doctor Runtime Phase 5 Implementation Plan

**Goal:** Make `loongclawd doctor` consume the same runtime-aware channel
snapshots as `loongclawd channels`, so operator diagnostics reflect both config
readiness and serve-loop liveness.

**Architecture:** Reuse shared channel snapshots from `loongclaw-app::channel`,
then derive two check classes in daemon:

- config checks
- runtime checks

Do not duplicate Telegram/Feishu liveness rules in daemon-specific code beyond
the final doctor severity mapping.

**Tech Stack:** Rust

### Task 1: Add failing doctor tests

Add tests that prove:

- a ready tracked serve operation that is not running becomes a runtime `Warn`
- a ready tracked serve operation with `stale=true` becomes a runtime `Fail`

### Task 2: Split doctor channel checks into config and runtime

Refactor doctor channel logic so:

- config checks still follow `Ready/Disabled/Unsupported/Misconfigured`
- runtime checks are emitted only for ready tracked serve operations
- runtime check names are distinct from config check names

### Task 3: Include useful runtime detail

Runtime doctor details should carry enough evidence for operators to act:

- `pid`
- `busy`
- `active_runs`
- `last_run_activity_at`
- `last_heartbeat_at`

### Task 4: Verify broadly

Run:

```bash
cargo test -p loongclaw-daemon build_channel_surface_checks_
cargo test -p loongclaw-daemon tests::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected result: PASS
