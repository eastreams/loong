# Channel Runtime Multiprocess Phase 6 Implementation Plan

**Goal:** Prevent same-operation channel serve instances from overwriting each
other's persisted runtime state while preserving the current operator-facing
runtime view.

**Architecture:** Keep `ChannelOperationRuntime` stable, but change persistence
to pid-scoped files and teach the loader to select the preferred current runtime
candidate across multiple files.

**Tech Stack:** Rust

### Task 1: Add failing tests for pid-scoped runtime files

Add tests that prove:

- runtime tracker writes `platform-operation-pid.json`
- loader prefers a live pid-scoped runtime over a newer stopped instance
- loader still reads legacy single-file runtime state

### Task 2: Change runtime persistence layout

Update runtime tracker start/write paths so the file name is keyed by:

- platform
- operation
- pid

### Task 3: Add multi-file runtime loading

Update runtime loading so it:

- scans all matching runtime files for an operation
- accepts both pid-scoped and legacy file names
- picks the preferred current candidate deterministically

### Task 4: Verify broadly

Run:

```bash
cargo test -p loongclaw-app channel::runtime_state::tests::
cargo test -p loongclaw-app channel::
cargo test -p loongclaw-daemon tests::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected result: PASS
