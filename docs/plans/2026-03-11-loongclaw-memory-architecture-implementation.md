# LoongClaw Memory Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a backward-compatible pluggable memory foundation that exposes memory profiles, derives internal memory modes, and routes runtime memory hydration through a shared app-layer orchestrator.

**Architecture:** Extend `MemoryConfig` with explicit backend/profile/profile-note metadata, mirror those fields into `MemoryRuntimeConfig`, and add shared memory hydration helpers in `crates/app/src/memory`. Provider/chat/channel/onboarding paths should depend on those helpers instead of direct SQLite window calls. Keep SQLite as the only backend for now, but make the abstraction real.

**Tech Stack:** Rust, serde/toml config, clap CLI, existing `loongclaw-app` and `loongclaw-daemon` crates, SQLite-backed memory feature, Rust unit tests.

---

### Task 1: Add Memory Domain Types To Config

**Files:**
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/config/mod.rs`
- Test: `crates/app/src/config/tools_memory.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn memory_profile_defaults_to_window_only() {
    let config = MemoryConfig::default();
    assert_eq!(config.backend, MemoryBackendKind::Sqlite);
    assert_eq!(config.profile, MemoryProfile::WindowOnly);
    assert_eq!(config.resolved_mode(), MemoryMode::WindowOnly);
}

#[test]
fn profile_plus_window_keeps_trimmed_profile_note() {
    let mut config = MemoryConfig::default();
    config.profile = MemoryProfile::ProfilePlusWindow;
    config.profile_note = Some("  imported preferences  ".to_owned());
    assert_eq!(
        config.trimmed_profile_note().as_deref(),
        Some("imported preferences")
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app memory_profile_defaults_to_window_only -- --exact`

Expected: FAIL because the new enums/helpers do not exist yet.

**Step 3: Write minimal implementation**

Add:

- `MemoryBackendKind`
- `MemoryProfile`
- `MemoryMode`
- `summary_max_chars`
- `profile_note`
- helper methods such as `resolved_mode()` and `trimmed_profile_note()`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools_memory:: -- --nocapture`

Expected: PASS for new config-memory tests.

**Step 5: Commit**

```bash
git add crates/app/src/config/tools_memory.rs crates/app/src/config/mod.rs
git commit -m "feat: add loongclaw memory profile config"
```

### Task 2: Extend Runtime Memory Config

**Files:**
- Modify: `crates/app/src/memory/runtime_config.rs`
- Modify: `crates/app/src/context.rs`
- Modify: `crates/app/src/chat.rs`
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Test: `crates/app/src/memory/runtime_config.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn runtime_config_from_memory_config_carries_profile_and_limits() {
    let mut config = MemoryConfig::default();
    config.profile = MemoryProfile::WindowPlusSummary;
    config.summary_max_chars = 900;

    let runtime = MemoryRuntimeConfig::from_memory_config(&config);

    assert_eq!(runtime.backend, MemoryBackendKind::Sqlite);
    assert_eq!(runtime.profile, MemoryProfile::WindowPlusSummary);
    assert_eq!(runtime.mode, MemoryMode::WindowPlusSummary);
    assert_eq!(runtime.summary_max_chars, 900);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app runtime_config_from_memory_config_carries_profile_and_limits -- --exact`

Expected: FAIL because the constructor and fields do not exist yet.

**Step 3: Write minimal implementation**

Add a typed constructor that converts `MemoryConfig` into `MemoryRuntimeConfig`
and update runtime bootstrap paths to use it instead of manually setting only
`sqlite_path`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app memory::runtime_config:: -- --nocapture`

Expected: PASS for runtime-config tests.

**Step 5: Commit**

```bash
git add crates/app/src/memory/runtime_config.rs crates/app/src/context.rs crates/app/src/chat.rs crates/app/src/channel/mod.rs crates/daemon/src/main.rs crates/daemon/src/onboard_cli.rs
git commit -m "feat: propagate typed memory runtime config"
```

### Task 3: Add Shared Memory Hydration Helpers

**Files:**
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/memory/sqlite.rs`
- Test: `crates/app/src/memory/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[cfg(feature = "memory-sqlite")]
#[test]
fn window_plus_summary_includes_condensed_older_context() {
    let config = seeded_runtime_config(MemoryProfile::WindowPlusSummary);
    seed_turns(&config, "s1", &[
        ("user", "turn 1"),
        ("assistant", "turn 2"),
        ("user", "turn 3"),
        ("assistant", "turn 4"),
    ]);

    let hydrated = load_prompt_context("s1", &config).expect("load prompt context");

    assert!(hydrated.iter().any(|entry| entry.kind == MemoryContextKind::Summary));
    assert!(hydrated.iter().any(|entry| entry.content.contains("turn 1")));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn profile_plus_window_includes_profile_note_block() {
    let mut config = seeded_runtime_config(MemoryProfile::ProfilePlusWindow);
    config.profile_note = Some("Imported ZeroClaw preferences".to_owned());

    let hydrated = load_prompt_context("s1", &config).expect("load prompt context");

    assert!(hydrated.iter().any(|entry| entry.kind == MemoryContextKind::Profile));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app window_plus_summary_includes_condensed_older_context -- --exact`

Expected: FAIL because the shared hydration API does not exist yet.

**Step 3: Write minimal implementation**

Add:

- shared prompt-context entry type
- `load_prompt_context(...)`
- deterministic summary builder
- backend dispatch in `execute_memory_core_with_config(...)`
- SQLite helper to load all turns needed for summary generation

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app memory:: -- --nocapture`

Expected: PASS for the new memory orchestration tests.

**Step 5: Commit**

```bash
git add crates/app/src/memory/mod.rs crates/app/src/memory/sqlite.rs
git commit -m "feat: add shared loongclaw memory hydration layer"
```

### Task 4: Route Provider And Chat Through Shared Memory Hydration

**Files:**
- Modify: `crates/app/src/provider/mod.rs`
- Modify: `crates/app/src/chat.rs`
- Test: `crates/app/src/provider/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[cfg(feature = "memory-sqlite")]
#[test]
fn message_builder_includes_summary_block_for_window_plus_summary_profile() {
    let mut config = test_config_with_memory_profile(MemoryProfile::WindowPlusSummary);
    seed_provider_turns(&config, "summary-session");

    let messages = build_messages_for_session(&config, "summary-session", true).expect("messages");

    assert!(messages.iter().any(|msg| msg["role"] == "system" && msg["content"].as_str().unwrap().contains("Memory Summary")));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app message_builder_includes_summary_block_for_window_plus_summary_profile -- --exact`

Expected: FAIL because provider still loads window turns directly from SQLite.

**Step 3: Write minimal implementation**

Refactor provider/chat history rendering to consume the shared memory context
loader rather than direct SQLite reads.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app provider::tests::message_builder_includes_summary_block_for_window_plus_summary_profile -- --exact`

Run: `cargo test -p loongclaw-app provider::tests::message_builder_uses_rendered_prompt_from_pack_metadata -- --exact`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/provider/mod.rs crates/app/src/chat.rs
git commit -m "feat: route runtime prompt hydration through memory profiles"
```

### Task 5: Add Onboarding Support For Memory Profile Selection

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/app/src/config/runtime.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn parse_memory_profile_accepts_supported_ids() {
    assert_eq!(
        crate::onboard_cli::parse_memory_profile("window_only"),
        Some(mvp::config::MemoryProfile::WindowOnly)
    );
    assert_eq!(
        crate::onboard_cli::parse_memory_profile("window_plus_summary"),
        Some(mvp::config::MemoryProfile::WindowPlusSummary)
    );
}
```

Also add a config persistence test proving `memory.profile` survives write/read.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon parse_memory_profile_accepts_supported_ids -- --exact`

Expected: FAIL because onboarding does not expose memory profiles yet.

**Step 3: Write minimal implementation**

Add:

- `--memory-profile`
- interactive memory profile selection
- onboarding summary output for selected memory profile

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon tests::onboard_cli::parse_memory_profile_accepts_supported_ids -- --exact`

Run: `cargo test -p loongclaw-app write_persists_memory_profile_metadata -- --exact`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/daemon/src/main.rs crates/daemon/src/onboard_cli.rs crates/daemon/src/tests/onboard_cli.rs crates/app/src/config/runtime.rs
git commit -m "feat: add onboarding support for loongclaw memory profiles"
```

### Task 6: Update Product Docs And Verify End-To-End

**Files:**
- Modify: `README.md`
- Modify: `docs/product-specs/index.md`
- Create: `docs/product-specs/memory-profiles.md`

**Step 1: Update docs**

Document:

- supported memory profiles
- current backend limitation
- role of `profile_note` for migration/imported identity

**Step 2: Run formatting and targeted verification**

Run:

```bash
cargo fmt --all
cargo test -p loongclaw-app
cargo test -p loongclaw-daemon
OPENAI_API_KEY=dummy cargo run -p loongclaw-daemon --bin loongclawd -- onboard \
  --non-interactive \
  --accept-risk \
  --provider openai \
  --model gpt-5 \
  --api-key-env OPENAI_API_KEY \
  --personality calm_engineering \
  --memory-profile window_plus_summary \
  --skip-model-probe \
  --output /tmp/loongclaw-memory-onboard.toml \
  --force
```

Expected:

- fmt exits 0
- both test suites pass
- onboard smoke run exits 0
- generated TOML includes `memory.profile = "window_plus_summary"`

**Step 3: Commit**

```bash
git add README.md docs/product-specs/index.md docs/product-specs/memory-profiles.md
git commit -m "docs: describe loongclaw memory profiles"
```
