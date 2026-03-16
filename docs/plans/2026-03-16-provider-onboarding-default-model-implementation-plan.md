# Provider Onboarding Default Model Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace hidden provider-owned runtime preferred-model fallbacks with
explicit onboarding default models while preserving `Esc` onboarding exit UX and
user-configured fallback behavior.

**Architecture:** Add onboarding-only provider default model metadata, teach the
onboarding flow to use it when `model` is still `auto`, and remove built-in
runtime fallback seeding from `preferred_models`. Runtime, doctor, and onboarding
should only advertise fallback continuation when the operator explicitly
configured `preferred_models`.

**Tech Stack:** Rust, cargo test, cargo clippy

---

### Task 1: Add failing provider-config tests

**Files:**
- Modify: `crates/app/src/config/provider.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- `fresh_for_kind(ProviderKind::Minimax)` does not seed hidden
  `preferred_models`
- user-configured `preferred_models` still round-trip through
  `configured_auto_model_candidates()`
- MiniMax exposes an onboarding recommended model distinct from runtime fallback
  state

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-app fresh_minimax_provider_does_not_seed_hidden_preferred_models
cargo test -p loongclaw-app minimax_provider_exposes_onboarding_recommended_model
```

Expected: FAIL before implementation.

### Task 2: Add failing onboarding tests

**Files:**
- Modify: `crates/daemon/src/onboard_cli.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- interactive model selection screen for MiniMax shows the explicit recommended
  model as the prefilled default rather than promising hidden fallback behavior
- non-interactive onboarding with `provider = minimax` and no `--model` writes
  `MiniMax-M2.5`
- onboarding probe warnings only mention fallback continuation when
  `preferred_models` is explicitly configured

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-daemon render_model_selection_screen_lines_with_default
cargo test -p loongclaw-daemon provider_model_probe_failure_check
```

Expected: FAIL before implementation.

### Task 3: Add failing runtime messaging tests

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/app/src/provider/model_candidate_resolver_runtime.rs`

**Step 1: Write the failing tests**

Add tests that prove:

- runtime does not fall back to provider-owned hidden defaults
- runtime still falls back to explicitly configured `preferred_models`
- doctor probe warnings distinguish explicit configuration from missing config

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p loongclaw-app preferred_model_fallback
cargo test -p loongclaw-daemon doctor_cli::tests::
```

Expected: FAIL before implementation.

### Task 4: Implement onboarding-only default model metadata

**Files:**
- Modify: `crates/app/src/config/provider.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`

**Step 1: Add onboarding metadata**

Add a provider-kind method for onboarding recommended model IDs.

**Step 2: Use it in onboarding**

When onboarding resolves the model and the current selection is still `auto`,
switch the default/prefill to the onboarding recommended model.

**Step 3: Keep runtime semantics unchanged**

Do not use this metadata in runtime model resolution.

### Task 5: Remove hidden provider fallback seeding

**Files:**
- Modify: `crates/app/src/config/provider.rs`
- Modify: `crates/app/src/provider/model_candidate_resolver_runtime.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

**Step 1: Stop seeding built-in preferred models**

Remove provider-kind built-in default preferred model lists.

**Step 2: Keep explicit operator behavior**

`configured_auto_model_candidates()` should only return user-configured
`preferred_models`.

**Step 3: Make messages truthful**

Update onboarding/doctor wording from "preferred model fallback(s)" to
"configured preferred model fallback(s)" where applicable.

### Task 6: Update docs

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/product-specs/onboarding.md`

**Step 1: Remove hidden-fallback framing**

Document that onboarding may prefill a provider-recommended explicit model.

**Step 2: Preserve exit guidance**

Keep the `Esc` cancellation guidance and WSL note from the earlier fix.

### Task 7: Verification

Run:

```bash
cargo test -p loongclaw-app config::provider::tests::
cargo test -p loongclaw-app preferred_model_fallback -- --nocapture
cargo test -p loongclaw-daemon onboard_cli::tests:: -- --nocapture
cargo test -p loongclaw-daemon doctor_cli::tests:: -- --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Expected: PASS

### Task 8: GitHub follow-through

**Files:**
- No code changes required if verification passes

**Step 1: Update the existing PR description if needed**

Reflect the corrected architecture:

- keep explicit `Esc` onboarding exit guidance
- replace hidden provider fallback defaults with onboarding explicit defaults
- retain fallback continuation only for explicit operator-configured
  `preferred_models`

**Step 2: Keep issue/PR traceability intact**

Continue on issue `#214` and PR `#215` rather than opening replacement artifacts.
