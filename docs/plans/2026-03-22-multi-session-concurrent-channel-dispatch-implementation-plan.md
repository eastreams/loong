# Multi-Session Concurrent Channel Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a daemon-owned `multi-channel-serve` runtime owner that keeps interactive CLI chat in the foreground while Telegram and Feishu run as supervised background tasks in the same process, with explicit session isolation, cooperative shutdown, and per-task health tracking.

**Architecture:** Keep process orchestration in `crates/daemon` and keep channel execution in `crates/app`. Introduce a new `crates/daemon/src/supervisor.rs` module for the runtime owner/state machine, extract a cancellable concurrent-mode CLI host seam from `crates/app/src/chat.rs`, and add a cooperative stop contract for channel serve flows in `crates/app/src/channel/mod.rs` so background tasks can exit normally and let the existing runtime wrapper call `shutdown()`.

**Tech Stack:** Rust, Tokio, Clap, existing LoongClaw daemon/app crates, SQLite-backed session state, existing daemon integration test harness

---

**Repo contract note:** Before **every commit** in this plan, run `task verify` from the repo root. This repo requires CI-parity checks at every commit, and `task verify` is the local superset gate.

### Task 1: Lock The CLI Contract

**Files:**
- Create: `crates/daemon/src/supervisor.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`
- Test: `crates/daemon/tests/integration/cli_tests.rs`

- [ ] **Step 1: Write the failing parser/help tests for the new command**

```rust
#[test]
fn multi_channel_serve_cli_requires_explicit_cli_session() {
    let error = Cli::try_parse_from(["loongclaw", "multi-channel-serve"])
        .expect_err("missing --session should fail");
    assert!(error.to_string().contains("--session <SESSION>"));
}

#[test]
fn multi_channel_serve_cli_parses_account_selection_flags() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "multi-channel-serve",
        "--session",
        "cli-supervisor",
        "--telegram-account",
        "bot_123456",
        "--feishu-account",
        "alerts",
    ])
    .expect("multi-channel-serve should parse");

    match cli.command {
        Some(Commands::MultiChannelServe {
            session,
            telegram_account,
            feishu_account,
            ..
        }) => {
            assert_eq!(session, "cli-supervisor");
            assert_eq!(telegram_account.as_deref(), Some("bot_123456"));
            assert_eq!(feishu_account.as_deref(), Some("alerts"));
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}
```

- [ ] **Step 2: Run the targeted parser tests and confirm they fail**

Run:

```bash
cargo test -p loongclaw-daemon --test integration multi_channel_serve_cli_
```

Expected: failure because `Commands::MultiChannelServe` does not exist yet.

- [ ] **Step 3: Add the command enum variant and dispatch stub**

Use a narrow initial CLI contract:

```rust
MultiChannelServe {
    #[arg(long)]
    config: Option<String>,
    #[arg(long)]
    session: String,
    #[arg(long)]
    telegram_account: Option<String>,
    #[arg(long)]
    feishu_account: Option<String>,
}
```

Add a daemon entrypoint stub in `crates/daemon/src/lib.rs` and route it from
`crates/daemon/src/main.rs`:

```rust
pub async fn run_multi_channel_serve_cli(...) -> CliResult<()> {
    supervisor::run_multi_channel_serve(...)
        .await
}
```

- [ ] **Step 4: Re-run the parser tests and confirm they pass**

Run:

```bash
cargo test -p loongclaw-daemon --test integration multi_channel_serve_cli_
```

Expected: PASS.

- [ ] **Step 5: Run repo verification for the CLI contract slice**

Run:

```bash
task verify
```

Expected: PASS.

- [ ] **Step 6: Commit the CLI contract slice**

```bash
git add crates/daemon/src/lib.rs crates/daemon/src/main.rs crates/daemon/src/supervisor.rs crates/daemon/tests/integration/cli_tests.rs
git commit -m "feat: add multi-channel-serve cli contract"
```

### Task 2: Build The Supervisor State Machine

**Files:**
- Modify: `crates/daemon/src/supervisor.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/src/supervisor.rs`

- [ ] **Step 1: Write failing unit tests for the in-memory lifecycle model**

Add unit tests covering:

```rust
#[tokio::test]
async fn background_surface_startup_records_start_timestamp_and_running_phase() {
    // start telegram child
    // assert started_at_ms is set and phase becomes Running
}

#[tokio::test]
async fn background_surface_failure_marks_runtime_owner_failed() {
    // start with two background children
    // mark telegram failed
    // assert supervisor transitions to failed and requests shutdown
}

#[tokio::test]
async fn graceful_shutdown_marks_running_children_stopping_then_stopped() {
    // seed running telegram + feishu
    // request shutdown
    // assert stopping -> stopped transitions
}

#[tokio::test]
async fn final_exit_reason_is_recorded_for_failed_child() {
    // fail one child
    // assert exit_reason and stopped_at_ms are populated
}
```

- [ ] **Step 2: Run the targeted supervisor tests and confirm they fail**

Run:

```bash
cargo test -p loongclaw-daemon supervisor::
```

Expected: failure because the state machine types/logic are not implemented yet.

- [ ] **Step 3: Implement the daemon-owned supervisor core**

Add focused types in `crates/daemon/src/supervisor.rs`:

```rust
pub enum BackgroundChannelSurface {
    Telegram { account_id: Option<String> },
    Feishu { account_id: Option<String> },
}

pub enum SurfacePhase {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

pub struct SurfaceState {
    pub surface: BackgroundChannelSurface,
    pub phase: SurfacePhase,
    pub started_at_ms: Option<u64>,
    pub stopped_at_ms: Option<u64>,
    pub last_error: Option<String>,
    pub exit_reason: Option<String>,
}
```

Implement:

- `SupervisorSpec`
- `SupervisorState`
- failure summarization
- startup-transition timestamps
- final-exit summarization
- shutdown-reason tracking
- helper methods that task wrappers can call without knowing CLI details

- [ ] **Step 4: Re-run the supervisor tests and confirm they pass**

Run:

```bash
cargo test -p loongclaw-daemon supervisor::
```

Expected: PASS.

- [ ] **Step 5: Run repo verification for the state machine slice**

Run:

```bash
task verify
```

Expected: PASS.

- [ ] **Step 6: Commit the state machine slice**

```bash
git add crates/daemon/src/supervisor.rs crates/daemon/src/lib.rs
git commit -m "feat: add multi-channel supervisor state machine"
```

### Task 3: Extract A Cancellable Foreground CLI Host

**Files:**
- Modify: `crates/app/src/chat.rs`
- Test: `crates/app/src/chat.rs`
- Test: `crates/daemon/tests/integration/chat_cli.rs`

- [ ] **Step 1: Write failing tests for concurrent-mode CLI session and shutdown behavior**

Add unit tests in `crates/app/src/chat.rs` for:

```rust
#[tokio::test]
async fn concurrent_cli_host_requires_explicit_session_id() {
    // concurrent mode should reject the implicit "default" session fallback
}

#[tokio::test]
async fn concurrent_cli_host_exits_when_shutdown_is_requested() {
    // host loop should stop without waiting forever on raw stdin read_line()
}
```

Add an integration test in `crates/daemon/tests/integration/chat_cli.rs` for
the user-visible contract if needed:

```rust
#[test]
fn multi_channel_serve_requires_explicit_session_flag() {
    // exercise binary parse/help path
}
```

- [ ] **Step 2: Run the targeted chat tests and confirm they fail**

Run:

```bash
cargo test -p loongclaw-app concurrent_cli_host_
```

Expected: failure because the concurrent-mode host seam does not exist yet.

- [ ] **Step 3: Extract the foreground host seam**

Refactor `crates/app/src/chat.rs` so concurrent mode uses a dedicated host path
instead of embedding raw blocking stdin ownership in the daemon runtime owner:

```rust
pub struct ConcurrentCliHostOptions {
    pub resolved_path: PathBuf,
    pub config: LoongClawConfig,
    pub session_id: String,
    pub shutdown: Arc<Notify>,
}

pub async fn run_concurrent_cli_host(
    options: &ConcurrentCliHostOptions,
) -> CliResult<()> {
    // initialize runtime from preloaded config + resolved path
    // drive input through a controllable loop
    // exit cleanly when shutdown is requested
}
```

Key requirements:

- do not allow implicit `"default"` session in concurrent mode
- keep ordinary `chat` behavior unchanged
- allow the runtime owner to stop the foreground host coherently
- do not reload config inside the concurrent host path

- [ ] **Step 4: Re-run the targeted chat tests and confirm they pass**

Run:

```bash
cargo test -p loongclaw-app concurrent_cli_host_
```

Expected: PASS.

- [ ] **Step 5: Run repo verification for the concurrent CLI host slice**

Run:

```bash
task verify
```

Expected: PASS.

- [ ] **Step 6: Commit the concurrent CLI host slice**

```bash
git add crates/app/src/chat.rs crates/daemon/tests/integration/chat_cli.rs
git commit -m "feat: add cancellable concurrent cli host"
```

### Task 4: Add Cooperative Stop For Channel Serve Tasks

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Test: `crates/app/src/channel/mod.rs`

- [ ] **Step 1: Write the failing cooperative-stop test around the existing serve wrapper**

Add a new targeted test near the existing `with_channel_serve_runtime_*` tests:

```rust
#[tokio::test]
async fn with_channel_serve_runtime_shuts_down_cleanly_after_cooperative_stop() {
    // start runtime wrapper
    // trigger cooperative stop
    // assert wrapper returns normally
    // assert runtime file is no longer running
}
```

- [ ] **Step 2: Run the targeted channel-runtime tests and confirm they fail**

Run:

```bash
cargo test -p loongclaw-app with_channel_serve_runtime_
```

Expected: failure because there is no cooperative stop seam yet.

- [ ] **Step 3: Add the cooperative stop contract in `channel/mod.rs`**

Introduce a small app-layer stop abstraction using existing Tokio primitives:

```rust
pub struct ChannelServeStopHandle {
    stop: Arc<Notify>,
}

async fn run_channel_serve_command_with_stop<R, V, F>(..., stop: ChannelServeStopHandle, run: F) -> CliResult<()>
```

Requirements:

- background serve loops can observe stop intent
- background serve setup uses preloaded config + resolved path via the existing
  `build_telegram_command_context(...)` / `build_feishu_command_context(...)`
  helpers instead of `load_*_command_context(...)`
- normal return path still runs through `with_channel_serve_runtime(...)`
- Tokio task abort remains fallback only, not the primary shutdown path

Do **not** redesign `ChannelOperationRuntimeTracker`; reuse the existing
`runtime.shutdown().await` call that already runs when the serve future returns.

- [ ] **Step 4: Re-run the targeted channel-runtime tests and confirm they pass**

Run:

```bash
cargo test -p loongclaw-app with_channel_serve_runtime_
```

Expected: PASS.

- [ ] **Step 5: Run repo verification for the cooperative-stop slice**

Run:

```bash
task verify
```

Expected: PASS.

- [ ] **Step 6: Commit the cooperative-stop slice**

```bash
git add crates/app/src/channel/mod.rs
git commit -m "feat: add cooperative stop for channel serve tasks"
```

### Task 5: Wire `multi-channel-serve` End To End

**Files:**
- Modify: `crates/daemon/src/supervisor.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/tests/integration/mod.rs`
- Create: `crates/daemon/tests/integration/multi_channel_serve_cli.rs`
- Modify: `crates/app/src/chat.rs`
- Modify: `crates/app/src/channel/mod.rs`

- [ ] **Step 1: Write failing daemon integration tests for concurrent ownership**

Create `crates/daemon/tests/integration/multi_channel_serve_cli.rs` and register
it in `crates/daemon/tests/integration/mod.rs`.

Add tests for:

```rust
#[tokio::test]
async fn multi_channel_serve_starts_telegram_and_feishu_background_tasks() {}

#[tokio::test]
async fn multi_channel_serve_background_failure_exits_foreground_cli_host_with_summarized_shutdown_reason() {}

#[tokio::test]
async fn multi_channel_serve_keeps_cli_session_distinct_from_channel_sessions() {}

#[tokio::test]
async fn multi_channel_serve_loads_config_once_before_spawning_children() {}

#[tokio::test]
async fn multi_channel_serve_ctrl_c_waits_for_background_joins_and_reports_shutdown_reason() {}

#[tokio::test]
async fn multi_channel_serve_cooperative_stop_clears_channel_runtime_running_state() {}
```

Use test doubles / injected runners rather than real network traffic.

- [ ] **Step 2: Run the new integration tests and confirm they fail**

Run:

```bash
cargo test -p loongclaw-daemon --test integration multi_channel_serve_
```

Expected: failure because the runtime owner is not wired end to end yet.

- [ ] **Step 3: Implement the runtime owner orchestration**

Wire the daemon entrypoint to:

- load config once at the runtime-owner root
- construct `SupervisorSpec` from CLI flags and the preloaded config
- start background Telegram and Feishu tasks
- start the foreground concurrent CLI host
- forward child failure into root shutdown
- forward root shutdown into:
  - foreground CLI host stop
  - cooperative channel stop handles

Representative shape:

```rust
pub async fn run_multi_channel_serve(...) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let mut owner = Supervisor::from_loaded_config(resolved_path, config, ...)?;
    owner.spawn_telegram_if_enabled().await?;
    owner.spawn_feishu_if_enabled().await?;
    owner.run_foreground_cli_host().await
}
```

- [ ] **Step 4: Re-run the new integration tests and confirm they pass**

Run:

```bash
cargo test -p loongclaw-daemon --test integration multi_channel_serve_
```

Expected: PASS.

- [ ] **Step 5: Run repo verification for the end-to-end runtime owner slice**

Run:

```bash
task verify
```

Expected: PASS.

- [ ] **Step 6: Commit the end-to-end runtime owner slice**

```bash
git add crates/daemon/src/supervisor.rs crates/daemon/src/lib.rs crates/daemon/src/main.rs crates/daemon/tests/integration/mod.rs crates/daemon/tests/integration/multi_channel_serve_cli.rs crates/app/src/chat.rs crates/app/src/channel/mod.rs
git commit -m "feat: add multi-channel serve runtime owner"
```

### Task 6: Update Operator Docs And Run Final Verification

**Files:**
- Modify: `docs/PRODUCT_SENSE.md`
- Modify: `README.md`
- Modify: `README.zh-CN.md`

- [ ] **Step 1: Update the user-facing docs**

Document the new surface consistently:

- `docs/PRODUCT_SENSE.md`
  - add `multi-channel-serve` to the command table
- `README.md`
  - explain the one-process concurrent runtime use case
- `README.zh-CN.md`
  - keep the Chinese surface aligned

- [ ] **Step 2: Run the canonical repo verification gate**

Run:

```bash
task verify
```

Expected:

- `task verify` exits 0
- CI-parity cargo checks pass under the repo Taskfile
- docs, architecture, dependency, and harness checks pass

- [ ] **Step 3: Commit the docs + verification slice**

```bash
git add docs/PRODUCT_SENSE.md README.md README.zh-CN.md
git commit -m "docs: document multi-channel serve runtime"
```
