# LoongClaw Prompt Pack Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a native LoongClaw prompt pack with three onboarding personalities while preserving current runtime compatibility.

**Architecture:** Introduce a dedicated prompt renderer in `crates/app` that composes a stable base prompt with a selected personality overlay, then wire config defaults, provider prompt assembly, and onboarding to use the renderer. Keep the first implementation compatible with the current `cli.system_prompt` field while reserving metadata for `prompt_pack_id` and `personality`.

**Tech Stack:** Rust, serde/toml config, clap CLI, existing `loongclaw-app` and `loongclaw-daemon` crates, Rust unit tests.

---

### Task 1: Create Prompt Domain Types And Renderer

**Files:**
- Create: `crates/app/src/prompt/mod.rs`
- Modify: `crates/app/src/lib.rs`
- Test: `crates/app/src/prompt/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn render_prompt_uses_loongclaw_base_and_selected_personality() {
    let rendered = render_system_prompt(PromptRenderInput {
        personality: PromptPersonality::CalmEngineering,
        addendum: None,
    });
    assert!(rendered.contains("You are LoongClaw"));
    assert!(rendered.contains("## Safety Invariants"));
    assert!(rendered.contains("## Personality Overlay: Calm Engineering"));
}

#[test]
fn render_prompt_adds_optional_addendum_at_the_end() {
    let rendered = render_system_prompt(PromptRenderInput {
        personality: PromptPersonality::FriendlyCollab,
        addendum: Some("Always prefer concise summaries.".to_owned()),
    });
    assert!(rendered.contains("Always prefer concise summaries."));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app render_prompt_uses_loongclaw_base_and_selected_personality -- --exact`

Expected: FAIL because `crates/app/src/prompt/mod.rs` and the renderer do not exist yet.

**Step 3: Write minimal implementation**

Create the new module with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptPersonality {
    CalmEngineering,
    FriendlyCollab,
    AutonomousExecutor,
}

pub struct PromptRenderInput {
    pub personality: PromptPersonality,
    pub addendum: Option<String>,
}

pub fn render_system_prompt(input: PromptRenderInput) -> String {
    let mut sections = vec![base_prompt().to_owned(), personality_overlay(input.personality)];
    if let Some(addendum) = input.addendum.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
        sections.push(format!("## User Addendum\n{addendum}"));
    }
    sections.join("\n\n")
}
```

Also export the new module in `crates/app/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app prompt:: -- --nocapture`

Expected: PASS for the new prompt module tests.

**Step 5: Commit**

```bash
git add crates/app/src/prompt/mod.rs crates/app/src/lib.rs
git commit -m "feat: add loongclaw prompt renderer"
```

### Task 2: Extend Config With Prompt Metadata

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/config/mod.rs`
- Test: `crates/app/src/config/runtime.rs`

**Step 1: Write the failing test**

Add config persistence tests like:

```rust
#[test]
#[cfg(feature = "config-toml")]
fn write_persists_prompt_pack_and_personality_metadata() {
    let path = unique_config_path("loongclaw-prompt-config");
    let path_string = path.display().to_string();
    let mut config = LoongClawConfig::default();
    config.cli.prompt_pack_id = "loongclaw-core-v1".to_owned();
    config.cli.personality = PromptPersonality::AutonomousExecutor;

    write(Some(&path_string), &config, true).expect("config write should pass");
    let (_, loaded) = load(Some(&path_string)).expect("config load should pass");

    assert_eq!(loaded.cli.prompt_pack_id, "loongclaw-core-v1");
    assert_eq!(loaded.cli.personality, PromptPersonality::AutonomousExecutor);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app write_persists_prompt_pack_and_personality_metadata -- --exact`

Expected: FAIL because `CliChannelConfig` does not yet have `prompt_pack_id` or `personality`.

**Step 3: Write minimal implementation**

Extend `CliChannelConfig` to include:

```rust
pub struct CliChannelConfig {
    pub enabled: bool,
    pub system_prompt: String,
    pub prompt_pack_id: String,
    pub personality: PromptPersonality,
    pub system_prompt_addendum: Option<String>,
    pub exit_commands: Vec<String>,
}
```

Defaults:

- `prompt_pack_id = "loongclaw-core-v1"`
- `personality = PromptPersonality::CalmEngineering`
- `system_prompt = render_system_prompt(...)`

Update TOML read/write tests in `runtime.rs` to cover the new fields.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app write_persists_custom_model_and_prompt -- --exact`

Run: `cargo test -p loongclaw-app write_persists_prompt_pack_and_personality_metadata -- --exact`

Expected: PASS for both legacy prompt persistence and new metadata persistence.

**Step 5: Commit**

```bash
git add crates/app/src/config/channels.rs crates/app/src/config/runtime.rs crates/app/src/config/mod.rs
git commit -m "feat: persist loongclaw prompt metadata"
```

### Task 3: Route Runtime Prompt Building Through The Renderer

**Files:**
- Modify: `crates/app/src/provider/mod.rs`
- Test: `crates/app/src/provider/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn message_builder_uses_rendered_prompt_from_pack_metadata() {
    let mut config = LoongClawConfig::default();
    config.cli.personality = PromptPersonality::FriendlyCollab;
    config.cli.system_prompt = String::new();

    let messages = build_messages_for_session(&config, "noop-session", true).expect("build messages");
    let system_content = messages[0]["content"].as_str().expect("system content");

    assert!(system_content.contains("## Personality Overlay: Friendly Collaboration"));
    assert!(system_content.contains("[available_tools]"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app message_builder_uses_rendered_prompt_from_pack_metadata -- --exact`

Expected: FAIL because `build_messages_for_session` still reads `config.cli.system_prompt` directly.

**Step 3: Write minimal implementation**

Refactor prompt selection behind a helper:

```rust
fn resolved_system_prompt(config: &LoongClawConfig) -> String {
    let inline = config.cli.system_prompt.trim();
    if !inline.is_empty() {
        return inline.to_owned();
    }
    render_system_prompt(PromptRenderInput {
        personality: config.cli.personality,
        addendum: config.cli.system_prompt_addendum.clone(),
    })
}
```

Use that helper in `build_messages_for_session` before appending the capability
snapshot.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app message_builder_includes_system_prompt -- --exact`

Run: `cargo test -p loongclaw-app build_messages_includes_capability_snapshot_block -- --exact`

Run: `cargo test -p loongclaw-app message_builder_uses_rendered_prompt_from_pack_metadata -- --exact`

Expected: PASS for all prompt-related provider tests.

**Step 5: Commit**

```bash
git add crates/app/src/provider/mod.rs
git commit -m "feat: render loongclaw prompt during message assembly"
```

### Task 4: Add Personality Selection To Onboarding

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Test: `crates/daemon/src/tests/onboard_cli.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn parse_personality_accepts_supported_ids() {
    assert_eq!(
        crate::onboard_cli::parse_prompt_personality("calm_engineering"),
        Some(mvp::prompt::PromptPersonality::CalmEngineering)
    );
    assert_eq!(
        crate::onboard_cli::parse_prompt_personality("friendly_collab"),
        Some(mvp::prompt::PromptPersonality::FriendlyCollab)
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon parse_personality_accepts_supported_ids -- --exact`

Expected: FAIL because onboarding does not yet understand prompt personalities.

**Step 3: Write minimal implementation**

- Add `--personality <id>` to the `Onboard` command in `crates/daemon/src/main.rs`.
- Extend `OnboardCommandOptions` with `personality: Option<String>`.
- Add parser helpers in `onboard_cli.rs`.
- Replace the current "system prompt" onboarding step with:
  - personality selection
  - optional prompt addendum
  - explicit advanced override only when `--system-prompt` is supplied

Minimal shape:

```rust
fn parse_prompt_personality(raw: &str) -> Option<mvp::prompt::PromptPersonality> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "calm_engineering" | "engineering" => Some(mvp::prompt::PromptPersonality::CalmEngineering),
        "friendly_collab" | "friendly" => Some(mvp::prompt::PromptPersonality::FriendlyCollab),
        "autonomous_executor" | "autonomous" => Some(mvp::prompt::PromptPersonality::AutonomousExecutor),
        _ => None,
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon onboard_cli -- --nocapture`

Expected: PASS for existing onboarding tests and the new personality parsing tests.

**Step 5: Commit**

```bash
git add crates/daemon/src/main.rs crates/daemon/src/onboard_cli.rs crates/daemon/src/tests/onboard_cli.rs
git commit -m "feat: add onboarding personality presets"
```

### Task 5: Document The New Prompt Model

**Files:**
- Modify: `README.md`
- Modify: `docs/product-specs/index.md`
- Modify: `docs/plans/2026-03-11-loongclaw-prompt-pack-design.md`

**Step 1: Write the failing doc checklist**

Add a simple checklist to the PR description or local notes:

```text
- README explains LoongClaw base prompt and personality presets
- onboarding docs mention --personality and addendum behavior
- product docs explain that safety invariants are shared across personalities
```

**Step 2: Run doc search to verify current content is missing**

Run: `rg -n "personality|prompt pack|LoongClaw AI" README.md docs`

Expected: missing or incomplete references for the new native prompt model.

**Step 3: Write minimal documentation updates**

- Add a short "Prompt and Personality" section to `README.md`
- Add the feature to the product docs index
- Update the design doc if implementation details changed during coding

**Step 4: Run doc checks**

Run: `rg -n "Prompt and Personality|calm_engineering|friendly_collab|autonomous_executor" README.md docs`

Expected: all new terms appear in the expected docs.

**Step 5: Commit**

```bash
git add README.md docs/product-specs/index.md docs/plans/2026-03-11-loongclaw-prompt-pack-design.md
git commit -m "docs: describe loongclaw prompt personalities"
```

### Task 6: Final Validation

**Files:**
- No new files
- Validate changes across app and daemon crates

**Step 1: Run focused tests**

Run:

```bash
cargo test -p loongclaw-app prompt::
cargo test -p loongclaw-app message_builder_includes_system_prompt -- --exact
cargo test -p loongclaw-app build_messages_includes_capability_snapshot_block -- --exact
cargo test -p loongclaw-daemon onboard_cli -- --nocapture
```

Expected: PASS for all new prompt/onboarding coverage.

**Step 2: Run broader crate tests**

Run:

```bash
cargo test -p loongclaw-app
cargo test -p loongclaw-daemon
```

Expected: PASS, except for any already-known unrelated baseline failures that must be documented explicitly if still present.

**Step 3: Smoke test onboarding and chat**

Run:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- onboard --non-interactive --accept-risk --provider openai --model gpt-5 --api-key-env OPENAI_API_KEY --personality calm_engineering --skip-model-probe --force
cargo run -p loongclaw-daemon --bin loongclawd -- chat --config ~/.loongclaw/config.toml
```

Expected:

- onboarding writes config with prompt metadata
- chat starts with the rendered LoongClaw prompt

**Step 4: Commit final validation-only adjustments if needed**

```bash
git add -A
git commit -m "test: validate loongclaw prompt pack integration"
```

**Step 5: Stop and report**

Document:

- what passed
- any baseline failures not caused by this work
- whether the runtime is still compatible with explicit `--system-prompt` overrides
