# Provider Active Selection Safety Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prevent silent provider-selection drift across mixed legacy/profile configs, add severity-aware config diagnostics, and make `doctor` / `onboard` treat explicit-model probe failures as advisory warnings rather than hard failures.

**Architecture:** Preserve raw TOML intent during config parsing, then normalize provider profiles with deterministic active-provider recovery that prefers explicit user choice over container order. Extend validation diagnostics with severity, and finally align `doctor` / `onboard` to the normalized active provider plus the provider's model-selection strategy.

**Tech Stack:** Rust, serde, toml, existing `loongclaw-app` config/provider runtime, daemon CLI diagnostics, cargo test, cargo fmt.

---

### Task 1: Add failing config regression tests for selection drift

**Files:**
- Modify: `crates/app/src/config/runtime.rs`
- Test: `crates/app/src/config/runtime.rs`

**Step 1: Write the failing test**

Add tests covering:

```rust
#[test]
fn load_mixed_legacy_and_profile_config_preserves_explicit_legacy_provider_when_active_provider_missing() {
    let config = parse_toml_config_without_validation(raw).expect("config should parse");
    assert_eq!(config.active_provider_id(), Some("volcengine-coding"));
}

#[test]
fn load_mixed_config_recovers_missing_active_provider_from_explicit_legacy_provider() {
    let config = parse_toml_config_without_validation(raw).expect("config should parse");
    assert_eq!(config.provider.kind, ProviderKind::VolcengineCoding);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app mixed_legacy_and_profile_config -- --nocapture`
Expected: FAIL because normalization currently falls back to the first
`providers` map entry.

**Step 3: Write minimal implementation**

- add raw-intent tracking fields to `LoongClawConfig`
- parse raw TOML for explicit `provider` / `active_provider` presence
- update normalization to recover active selection from explicit legacy intent

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app mixed_legacy_and_profile_config -- --nocapture`
Expected: PASS.

### Task 2: Add failing validation-diagnostic severity tests

**Files:**
- Modify: `crates/app/src/config/shared.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Test: `crates/app/src/config/runtime.rs`

**Step 1: Write the failing test**

Add tests covering:

```rust
#[test]
fn validate_file_reports_warning_for_mixed_provider_selection_without_explicit_active_provider() {
    let (_, diagnostics) = validate_file(Some(path)).expect("validate_file should parse");
    assert_eq!(diagnostics[0].severity, "warn");
}

#[test]
fn validate_file_keeps_existing_env_pointer_diagnostics_as_errors() {
    let (_, diagnostics) = validate_file(Some(path)).expect("validate_file should parse");
    assert_eq!(diagnostics[0].severity, "error");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app validate_file_reports_warning_for_mixed_provider_selection_without_explicit_active_provider -- --nocapture`
Expected: FAIL because diagnostics do not yet expose severity or emit the new
provider-selection warning.

**Step 3: Write minimal implementation**

- introduce config validation severity enum and serialization
- default existing diagnostics to `Error`
- add warning diagnostics for risky mixed provider-selection states

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app validate_file_reports_warning_for_mixed_provider_selection_without_explicit_active_provider -- --nocapture`
Expected: PASS.

### Task 3: Add failing validate-config CLI output tests

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/tests/mod.rs`
- Test: `crates/daemon/src/tests/mod.rs`

**Step 1: Write the failing test**

Add tests covering:

```rust
#[test]
fn validate_config_json_includes_diagnostic_severity() {
    let payload = run_validate_config_json(...);
    assert_eq!(payload["diagnostics"][0]["severity"], "warn");
}

#[test]
fn validate_config_problem_json_preserves_warning_diagnostics() {
    let payload = run_validate_config_problem_json(...);
    assert_eq!(payload["errors"][0]["severity"], "warn");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon validate_config_json_includes_diagnostic_severity -- --nocapture`
Expected: FAIL because the serialized diagnostics do not yet include severity.

**Step 3: Write minimal implementation**

- thread diagnostic severity through text / JSON / problem-json output
- keep `fail_on_diagnostics` semantics unchanged for now

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon validate_config_json_includes_diagnostic_severity -- --nocapture`
Expected: PASS.

### Task 4: Add failing doctor/onboard tests for explicit-model probe downgrade

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/daemon/src/tests/mod.rs`
- Test: `crates/daemon/src/tests/onboard_cli.rs`
- Test: `crates/daemon/src/doctor_cli.rs`

**Step 1: Write the failing test**

Add tests covering:

```rust
#[tokio::test]
async fn explicit_model_probe_failure_warns_during_onboard_preflight() {
    let checks = run_preflight_checks(&config_with_explicit_model(), false).await;
    assert_eq!(model_probe.level, OnboardCheckLevel::Warn);
}

#[tokio::test]
async fn explicit_model_probe_failure_warns_during_doctor() {
    let checks = collect_doctor_checks_for_config(config_with_explicit_model()).await;
    assert_eq!(model_probe.level, DoctorCheckLevel::Warn);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon explicit_model_probe_failure_warns -- --nocapture`
Expected: FAIL because probe failures are currently classified as `Fail`.

**Step 3: Write minimal implementation**

- classify model-probe failures using `provider.explicit_model()`
- keep auto-discovery failures as `Fail`
- include detail that explicit-model chat may still work without catalog
  discovery

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon explicit_model_probe_failure_warns -- --nocapture`
Expected: PASS.

### Task 5: Add active-provider context to doctor/onboard detail

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/provider_presentation.rs`
- Modify: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`

**Step 1: Write the failing test**

Add tests covering:

```rust
#[test]
fn provider_credential_check_mentions_active_provider_profile() {
    let check = provider_credential_check(&config_with_active_profile("openrouter"));
    assert!(check.detail.contains("openrouter"));
}

#[test]
fn provider_model_probe_warning_mentions_explicit_model_mode() {
    let check = classify_probe_failure(...);
    assert!(check.detail.contains("explicitly configured"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon provider_credential_check_mentions_active_provider_profile -- --nocapture`
Expected: FAIL because current details describe only the normalized provider
config, not the selected saved profile.

**Step 3: Write minimal implementation**

- add helper(s) that render active provider context from normalized config
- reuse them in doctor/onboard detail text

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon provider_credential_check_mentions_active_provider_profile -- --nocapture`
Expected: PASS.

### Task 6: Run focused suites and broader regressions

**Files:**
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/config/shared.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Test: `crates/daemon/src/tests/mod.rs`
- Test: `crates/daemon/src/tests/onboard_cli.rs`

**Step 1: Run focused tests**

Run:

```bash
cargo test -p loongclaw-app mixed_legacy_and_profile_config -- --nocapture
cargo test -p loongclaw-app validate_file_reports_warning_for_mixed_provider_selection_without_explicit_active_provider -- --nocapture
cargo test -p loongclaw-daemon explicit_model_probe_failure_warns -- --nocapture
```

Expected: PASS.

**Step 2: Run broader package regressions**

Run:

```bash
cargo test -p loongclaw-app -- --test-threads=1
cargo test -p loongclaw-daemon -- --test-threads=1
```

Expected: PASS.

**Step 3: Run formatting verification**

Run:

```bash
cargo fmt --all --check
```

Expected: PASS.

### Task 7: Prepare delivery artifacts

**Files:**
- Modify: `docs/plans/2026-03-16-provider-active-selection-safety-design.md`
- Modify: `docs/plans/2026-03-16-provider-active-selection-safety-implementation-plan.md`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/config/shared.rs`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`

**Step 1: Inspect scope before commit**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Expected: only provider-selection safety files are present.

**Step 2: Commit**

Run:

```bash
git add docs/plans/2026-03-16-provider-active-selection-safety-design.md \
  docs/plans/2026-03-16-provider-active-selection-safety-implementation-plan.md \
  crates/app/src/config/runtime.rs \
  crates/app/src/config/shared.rs \
  crates/daemon/src/main.rs \
  crates/daemon/src/doctor_cli.rs \
  crates/daemon/src/onboard_cli.rs \
  crates/daemon/src/tests/mod.rs \
  crates/daemon/src/tests/onboard_cli.rs
git commit -m "fix: harden provider active selection semantics"
```

**Step 3: Push and open PR**

Run:

```bash
git push fork-chumyin fix/provider-active-selection
gh pr create --repo loongclaw-ai/loongclaw --base alpha-test --head chumyin:fix/provider-active-selection
```

PR body must include: `Closes #175`
