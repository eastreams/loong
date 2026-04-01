# Personalize Command Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Ship an optional `loongclaw personalize` command that saves typed operator preferences into the advisory session-profile lane and promotes the flow from onboarding, welcome, and doctor without interrupting the first-answer path.

**Architecture:** Add a typed personalization payload under memory-owned advisory config, thread it through `MemoryRuntimeConfig`, and render it into `## Session Profile` only through the existing advisory projection path. Implement a dedicated daemon CLI flow that reuses extracted prompt primitives from onboarding, then add a new `SetupNextActionKind::Personalize` so onboarding success, welcome, and doctor can surface the command consistently. Keep first-chat auto-suggestion out of this slice to avoid adding new interruption semantics before the core storage and CLI flow are stable.

**Tech Stack:** Rust, serde/TOML config encoding, existing dialoguer/TUI prompt helpers, daemon integration tests, app runtime/config tests, cargo fmt, clippy, workspace tests.

---

## Implementation Tasks

### Task 1: Add red tests for typed personalization storage and projection

**Files:**
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/runtime_identity.rs`
- Modify: `crates/app/src/memory/context.rs`

**Step 1: Add a config round-trip test**

Add a new `config-toml` test in `crates/app/src/config/runtime.rs` named:

```rust
fn write_persists_typed_personalization_metadata()
```

The test should:

- build a `LoongClawConfig::default()`
- set `memory.profile = ProfilePlusWindow`
- populate typed personalization fields
- write the config
- reload it
- assert the typed personalization payload survives round-trip

**Step 2: Add a positive session-profile projection test**

Add a new test in `crates/app/src/memory/context.rs` named:

```rust
fn profile_plus_window_includes_typed_personalization_section()
```

The test should:

- build a `MemoryRuntimeConfig` with `profile = ProfilePlusWindow`
- populate typed personalization fields
- load prompt context
- assert the projected system content contains `## Session Profile`
- assert it contains the saved operator preference fields
- assert it does not contain `## Resolved Runtime Identity`

**Step 3: Add a negative gating test**

Add a new test in `crates/app/src/memory/context.rs` named:

```rust
fn window_only_ignores_typed_personalization_section()
```

The test should prove that typed personalization is not silently projected when
the active memory mode is still `WindowOnly`.

**Step 4: Add a rendering test for mixed advisory content**

Add a new test in `crates/app/src/runtime_identity.rs` named:

```rust
fn render_session_profile_section_merges_personalization_without_identity_promotion()
```

The test should pass:

- a typed personalization payload
- a `profile_note` containing ordinary advisory text

and assert that the combined session profile keeps both sources advisory.

**Step 5: Run the targeted tests to confirm red**

Run:

```bash
cargo test -p loongclaw-app write_persists_typed_personalization_metadata -- --exact
cargo test -p loongclaw-app profile_plus_window_includes_typed_personalization_section -- --exact
cargo test -p loongclaw-app window_only_ignores_typed_personalization_section -- --exact
cargo test -p loongclaw-app render_session_profile_section_merges_personalization_without_identity_promotion -- --exact
```

Expected:

- the new tests fail because the typed personalization config and projection do
  not exist yet

### Task 2: Implement typed personalization config under the advisory memory lane

**Files:**
- Modify: `crates/app/src/config/memory.rs`
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/memory/runtime_config.rs`

**Step 1: Add the typed config model**

Add a typed nested config under `MemoryConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PersonalizationConfig {
    pub preferred_name: Option<String>,
    pub response_density: Option<ResponseDensity>,
    pub initiative_level: Option<InitiativeLevel>,
    pub standing_boundaries: Option<String>,
    pub timezone: Option<String>,
    pub locale: Option<String>,
    pub prompt_state: PersonalizationPromptState,
    pub schema_version: u32,
    pub updated_at_epoch_seconds: Option<u64>,
}
```

Add the supporting enums:

```rust
pub enum ResponseDensity { Concise, Balanced, Thorough }
pub enum InitiativeLevel { AskBeforeActing, Balanced, HighInitiative }
pub enum PersonalizationPromptState { Pending, Deferred, Suppressed, Configured }
```

Keep all enums `snake_case` in TOML/serde.

**Step 2: Add trim and emptiness helpers**

Implement helpers that:

- trim optional strings
- treat an all-empty personalization payload as absent
- preserve deterministic defaults for `schema_version` and `prompt_state`

Do not overload `profile_note` string parsing for this slice.

**Step 3: Thread personalization through runtime config**

Extend `MemoryRuntimeConfig` to carry:

```rust
pub personalization: Option<crate::config::PersonalizationConfig>
```

Populate it from `MemoryConfig`.

Do not add new environment-variable overrides in v1.

**Step 4: Keep config encoding stable**

Update any config encode/decode and re-export wiring so:

- the nested TOML block writes cleanly
- config defaults stay backward-compatible
- old configs without personalization still load unchanged

**Step 5: Run the targeted app tests**

Run:

```bash
cargo test -p loongclaw-app write_persists_typed_personalization_metadata -- --exact
```

Expected:

- config round-trip passes

### Task 3: Project typed personalization into `Session Profile`

**Files:**
- Modify: `crates/app/src/runtime_identity.rs`
- Modify: `crates/app/src/memory/mod.rs`
- Modify: `crates/app/src/memory/context.rs`

**Step 1: Extend session-profile rendering**

Change the session-profile projection API from:

```rust
render_session_profile_section(profile_note: Option<&str>)
```

to a shape that can render both:

- typed personalization
- existing `profile_note`

For example:

```rust
render_session_profile_section(
    profile_note: Option<&str>,
    personalization: Option<&crate::config::PersonalizationConfig>,
)
```

**Step 2: Render personalization as advisory text**

Render the typed fields into a compact advisory block under the existing
`## Session Profile` wrapper. Keep the content operator-legible, for example:

```text
Preferred name: Chum
Response density: thorough
Initiative level: high_initiative
Standing boundaries:
- ask before destructive actions
Timezone: Asia/Shanghai
```

Do not render authority-looking headings inside that block.

**Step 3: Gate projection behind `ProfilePlusWindow`**

Keep the current memory-mode semantics:

- typed personalization should appear when the mode is `ProfilePlusWindow`
- it should not silently leak into `WindowOnly` or `WindowPlusSummary`

**Step 4: Preserve advisory demotion**

If a free-text field such as `standing_boundaries` contains identity-looking or
runtime-owned headings, keep the existing advisory-heading demotion behavior in
effect.

**Step 5: Run the targeted projection tests**

Run:

```bash
cargo test -p loongclaw-app profile_plus_window_includes_typed_personalization_section -- --exact
cargo test -p loongclaw-app window_only_ignores_typed_personalization_section -- --exact
cargo test -p loongclaw-app render_session_profile_section_merges_personalization_without_identity_promotion -- --exact
```

Expected:

- the new projection behavior passes without changing runtime-identity
  authority rules

### Task 4: Extract reusable prompt primitives and add `loongclaw personalize`

**Files:**
- Create: `crates/daemon/src/operator_prompt.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Create: `crates/daemon/src/personalize_cli.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/tests/integration/mod.rs`
- Create: `crates/daemon/tests/integration/personalize_cli.rs`

**Step 1: Add failing CLI parse/help coverage**

Add integration tests named along these lines:

```rust
fn cli_personalize_help_mentions_operator_preferences()
fn personalize_cli_accepts_config_flag()
```

The help text should describe:

- optional operator preference capture
- advisory persistence
- `loongclaw onboard` is still the setup path

**Step 2: Extract shared prompt helpers**

Move the reusable prompt pieces out of `onboard_cli.rs` into
`operator_prompt.rs`:

- `SelectOption`
- `SelectInteractionMode`
- prompt trait
- stdio/rich prompt helpers

Keep onboarding behavior unchanged while making the prompt surface reusable by
`personalize_cli.rs`.

**Step 3: Add a fakeable personalize flow**

Implement `personalize_cli.rs` around the extracted prompt trait so unit and
integration tests can drive the flow without real terminal input.

The flow should ask, in order:

1. preferred name
2. response density
3. initiative level
4. standing boundaries
5. optional timezone or locale

Then render a review screen with actions:

- save
- skip for now
- suppress future suggestions

**Step 4: Handle memory-profile compatibility explicitly**

If the operator saves personalization while `config.memory.profile` is not
`ProfilePlusWindow`, prompt for an explicit upgrade to
`ProfilePlusWindow`.

Do not silently rewrite memory mode without confirmation.

**Step 5: Persist config changes safely**

Saving should:

- update the typed personalization payload
- set `prompt_state = Configured`
- set `schema_version = 1`
- set `updated_at_epoch_seconds`
- write the config through the normal config writer

Suppressing should:

- leave typed preference fields empty
- set `prompt_state = Suppressed`
- persist the config without changing runtime identity fields

Skipping should:

- leave the config untouched

**Step 6: Run the targeted daemon tests**

Run:

```bash
cargo test -p loongclaw-daemon cli_personalize_help_mentions_operator_preferences -- --exact
cargo test -p loongclaw-daemon personalize_cli_accepts_config_flag -- --exact
cargo test -p loongclaw-daemon personalize_cli -- --nocapture
```

Expected:

- CLI parsing/help tests pass
- the fake-UI personalize flow tests pass for save, skip, suppress, and memory
  profile upgrade confirmation

### Task 5: Add next-action integration for onboarding, welcome, and doctor

**Files:**
- Modify: `crates/daemon/src/next_actions.rs`
- Modify: `crates/daemon/src/onboard_finalize.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/tests/integration/onboard_cli.rs`
- Modify: `crates/daemon/tests/integration/cli_tests.rs`

**Step 1: Add a new next-action kind**

Extend `SetupNextActionKind` with:

```rust
Personalize
```

Add a helper that determines whether personalization should be suggested:

- only when CLI is enabled
- only when `prompt_state != Suppressed`
- only when personalization is not already configured

Insert the action after `Ask` and `Chat`, not before them.

**Step 2: Reuse `next_actions` in the welcome banner**

Refactor `render_welcome_banner(...)` so welcome does not hand-maintain a
separate hardcoded command list.

Build the welcome quick-command list from `collect_setup_next_actions(...)`
plus the existing help line.

This keeps onboarding, welcome, and doctor aligned when `personalize` is added.

**Step 3: Map the new action in onboarding success**

Extend onboarding success summary mapping so:

- `first answer` remains the primary `start here` action
- `personalize` appears only as a secondary action

Update the existing onboarding-success rendering tests to lock the new order.

**Step 4: Surface personalization in healthy doctor output**

Update `build_doctor_next_steps_with_path_env(...)` and
`select_doctor_first_turn_actions(...)` so healthy doctor output can say
something like:

```text
Set your working preferences: loongclaw personalize --config '...'
```

Place this after:

- `Get a first answer`
- `Continue in chat`

and before lower-priority optional actions like generic browser-preview nudges.

**Step 5: Run the targeted surface tests**

Run:

```bash
cargo test -p loongclaw-daemon collect_setup_next_actions -- --nocapture
cargo test -p loongclaw-daemon build_doctor_next_steps_promotes_ask_and_chat_when_green -- --exact
cargo test -p loongclaw-daemon render_welcome_banner_includes_version_and_next_commands -- --exact
cargo test -p loongclaw-daemon onboard_cli -- --nocapture
```

Expected:

- `ask` remains first
- `chat` remains second
- personalization shows only as a secondary healthy-path suggestion

### Task 6: Update shipped docs and command inventory

**Files:**
- Modify: `docs/PRODUCT_SENSE.md`
- Modify: `docs/product-specs/index.md`
- Modify: `docs/product-specs/personalization.md`
- Modify: `README.md`

**Step 1: Promote the command into shipped product docs**

Update `docs/PRODUCT_SENSE.md` so the user-facing command table includes:

```text
personalize | Optional operator preference capture and review
```

Keep the documented primary first-run contract unchanged:

- `onboard -> ask -> chat -> doctor`

**Step 2: Update the product-specs index note**

Change `Personalization` from expectation-setting language to shipped language
once the command exists.

**Step 3: Add one bounded README mention**

Update the quickstart or post-onboard guidance so the feature is discoverable as
an optional follow-up, not a required setup step.

Do not rewrite the quickstart into a longer multi-step ritual.

### Task 7: Run full verification

**Files:**
- Verify only

**Step 1: Format**

```bash
cargo fmt --all
cargo fmt --all --check
```

**Step 2: Run focused tests**

```bash
cargo test -p loongclaw-app write_persists_typed_personalization_metadata -- --exact
cargo test -p loongclaw-app profile_plus_window_includes_typed_personalization_section -- --exact
cargo test -p loongclaw-app window_only_ignores_typed_personalization_section -- --exact
cargo test -p loongclaw-app render_session_profile_section_merges_personalization_without_identity_promotion -- --exact
cargo test -p loongclaw-daemon cli_personalize_help_mentions_operator_preferences -- --exact
cargo test -p loongclaw-daemon personalize_cli_accepts_config_flag -- --exact
cargo test -p loongclaw-daemon personalize_cli -- --nocapture
cargo test -p loongclaw-daemon build_doctor_next_steps_promotes_ask_and_chat_when_green -- --exact
cargo test -p loongclaw-daemon render_welcome_banner_includes_version_and_next_commands -- --exact
```

**Step 3: Run touched-surface lint**

```bash
cargo clippy -p loongclaw-app -p loongclaw-daemon --all-targets --all-features -- -D warnings
```

**Step 4: Run workspace tests**

```bash
cargo test --workspace --all-features
```

Expected:

- new app and daemon tests pass
- touched-surface lint passes
- full workspace tests pass

### Task 8: Prepare clean delivery

**Files:**
- Modify only the files touched by this plan

**Step 1: Inspect scope**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

**Step 2: Commit in bounded slices**

Suggested commit sequence:

```bash
git add crates/app/src/config/memory.rs
git add crates/app/src/config/mod.rs
git add crates/app/src/config/runtime.rs
git add crates/app/src/memory/runtime_config.rs
git add crates/app/src/runtime_identity.rs
git add crates/app/src/memory/mod.rs
git add crates/app/src/memory/context.rs
git commit -m "feat(app): add typed personalization session-profile state"
```

```bash
git add crates/daemon/src/operator_prompt.rs
git add crates/daemon/src/onboard_cli.rs
git add crates/daemon/src/personalize_cli.rs
git add crates/daemon/src/lib.rs
git add crates/daemon/src/main.rs
git add crates/daemon/src/next_actions.rs
git add crates/daemon/src/onboard_finalize.rs
git add crates/daemon/src/doctor_cli.rs
git add crates/daemon/tests/integration/mod.rs
git add crates/daemon/tests/integration/personalize_cli.rs
git add crates/daemon/tests/integration/onboard_cli.rs
git add crates/daemon/tests/integration/cli_tests.rs
git commit -m "feat(daemon): add personalize command and healthy-path guidance"
```

```bash
git add docs/PRODUCT_SENSE.md
git add docs/product-specs/index.md
git add docs/product-specs/personalization.md
git add README.md
git add docs/plans/2026-04-01-personalize-command-implementation-plan.md
git commit -m "docs(product): document shipped personalize flow"
```
