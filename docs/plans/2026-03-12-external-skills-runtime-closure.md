# External Skills Runtime Closure Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn external skills from a download-only and migration-only feature into a managed runtime loop with install, list, inspect, invoke, and remove lifecycle tools.

**Architecture:** Keep the top-level provider/tool surface static, add managed lifecycle tools under `external_skills.*`, store installed skills under a deterministic managed root with a JSON index, and expose installed skills to the model through both structured lifecycle tools and a deterministic capability snapshot section. Skills remain instruction packages loaded into the existing conversation loop, not dynamically generated native function tools.

**Tech Stack:** Rust, serde/json/toml config, `loongclaw-app`, `loongclaw-daemon`, existing provider/tool runtime, new archive extraction dependencies if needed, Rust unit tests.

---

### Task 1: Extend External Skills Config And Runtime Types

**Files:**
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Test: `crates/app/src/config/tools_memory.rs`
- Test: `crates/app/src/tools/runtime_config.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn external_skills_defaults_include_managed_install_settings() {
    let config = ExternalSkillsConfig::default();
    assert!(!config.enabled);
    assert!(config.require_download_approval);
    assert!(config.install_root.is_none());
    assert!(config.auto_expose_installed);
}

#[test]
fn tool_runtime_config_from_env_defaults_external_skills_install_flags() {
    let config = ToolRuntimeConfig::default();
    assert!(config.external_skills.install_root.is_none());
    assert!(config.external_skills.auto_expose_installed);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app external_skills_defaults_include_managed_install_settings -- --exact`

Expected: FAIL because `install_root` and `auto_expose_installed` do not exist yet.

**Step 3: Write minimal implementation**

Add:

- `install_root: Option<String>` to config
- `auto_expose_installed: bool` to config
- normalized helpers and runtime mirrored fields in `ToolRuntimeConfig`
- default config rendering/loading coverage

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app config::tools_memory:: -- --nocapture`

Run: `cargo test -p loongclaw-app tools::runtime_config:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/config/tools_memory.rs crates/app/src/config/runtime.rs crates/app/src/tools/runtime_config.rs
git commit -m "feat: extend external skills runtime config"
```

### Task 2: Add Managed Skill Index And Install Helpers

**Files:**
- Modify: `crates/app/src/tools/external_skills.rs`
- Modify: `crates/app/Cargo.toml`
- Test: `crates/app/src/tools/external_skills.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn install_skill_from_directory_writes_index_and_managed_copy() {
    let root = unique_temp_dir("external-skills-install-dir");
    let source = root.join("demo-skill");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "# Demo Skill\n\nUse this skill.").unwrap();

    let config = test_tool_runtime_config_with_external_skills(&root, true);
    let outcome = execute_external_skills_install_tool_with_config(
        tool_request("external_skills.install", json!({ "path": source.display().to_string() })),
        &config,
    )
    .expect("install should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["skill_id"], "demo-skill");
    assert!(root.join("external-skills-installed").join("index.json").exists());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app install_skill_from_directory_writes_index_and_managed_copy -- --exact`

Expected: FAIL because there is no install path or managed index yet.

**Step 3: Write minimal implementation**

Add:

- managed install root resolution helpers
- installed skill index structs and JSON persistence
- safe local path resolution for install sources
- directory install path with `SKILL.md` validation
- archive extraction support for `.tgz` / `.tar.gz`
- managed copy into `external-skills-installed/<skill_id>/`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools::external_skills:: -- --nocapture`

Expected: PASS for the new install-path tests.

**Step 5: Commit**

```bash
git add crates/app/Cargo.toml crates/app/src/tools/external_skills.rs
git commit -m "feat: add managed external skill installation"
```

### Task 3: Add Lifecycle Tools For List, Inspect, Invoke, And Remove

**Files:**
- Modify: `crates/app/src/tools/external_skills.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/external_skills.rs`
- Test: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn list_installed_skills_returns_active_entries() {
    let fixture = install_demo_skill_fixture();
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.list".to_owned(),
            payload: json!({}),
        },
        &fixture.config,
    )
    .expect("list should succeed");

    assert_eq!(outcome.payload["skills"][0]["skill_id"], "demo-skill");
}

#[test]
fn invoke_installed_skill_returns_skill_markdown_and_metadata() {
    let fixture = install_demo_skill_fixture();
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.invoke".to_owned(),
            payload: json!({ "skill_id": "demo-skill" }),
        },
        &fixture.config,
    )
    .expect("invoke should succeed");

    assert!(outcome.payload["instructions"].as_str().unwrap().contains("Demo Skill"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app invoke_installed_skill_returns_skill_markdown_and_metadata -- --exact`

Expected: FAIL because these lifecycle tools do not exist yet.

**Step 3: Write minimal implementation**

Add static lifecycle tools:

- `external_skills.install`
- `external_skills.list`
- `external_skills.inspect`
- `external_skills.invoke`
- `external_skills.remove`

Wire them into:

- `canonical_tool_name()`
- `is_known_tool_name()`
- `execute_tool_core_with_config()`
- `tool_registry()`
- `provider_tool_definitions()`
- shape examples and tests

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools::tests::provider_tool_definitions_are_stable_and_complete -- --exact`

Run: `cargo test -p loongclaw-app tools::external_skills:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/external_skills.rs crates/app/src/tools/mod.rs
git commit -m "feat: add external skills lifecycle tools"
```

### Task 4: Expose Installed Skills In Capability Snapshot

**Files:**
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/provider/mod.rs`
- Test: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/provider/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn capability_snapshot_lists_installed_external_skills_when_enabled() {
    let snapshot = capability_snapshot_with_config(&fixture.config);
    assert!(snapshot.contains("[available_external_skills]"));
    assert!(snapshot.contains("- demo-skill:"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app capability_snapshot_lists_installed_external_skills_when_enabled -- --exact`

Expected: FAIL because the snapshot has no installed-skill section yet.

**Step 3: Write minimal implementation**

Add a runtime-aware capability snapshot helper that:

- keeps the existing `[available_tools]` block deterministic
- appends `[available_external_skills]` when exposure is enabled and the index
  is non-empty

Update provider system-message builders to use the runtime-aware snapshot path.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools::tests:: -- --nocapture`

Run: `cargo test -p loongclaw-app provider::tests:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/mod.rs crates/app/src/provider/mod.rs
git commit -m "feat: expose installed external skills in capability snapshot"
```

### Task 5: Tighten Error Contracts And Duplicate-Install Behavior

**Files:**
- Modify: `crates/app/src/tools/external_skills.rs`
- Test: `crates/app/src/tools/external_skills.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn install_rejects_source_without_skill_md() {
    let fixture = empty_external_skills_fixture();
    let err = execute_external_skills_install_tool_with_config(
        tool_request("external_skills.install", json!({ "path": fixture.source.display().to_string() })),
        &fixture.config,
    )
    .expect_err("install should fail");

    assert!(err.contains("SKILL.md"));
}

#[test]
fn install_rejects_duplicate_without_replace() {
    let fixture = install_demo_skill_fixture();
    let err = execute_external_skills_install_tool_with_config(
        tool_request("external_skills.install", json!({ "path": fixture.source.display().to_string() })),
        &fixture.config,
    )
    .expect_err("duplicate install should fail");

    assert!(err.contains("already installed"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app install_rejects_duplicate_without_replace -- --exact`

Expected: FAIL because duplicate-install behavior is not implemented yet.

**Step 3: Write minimal implementation**

Add:

- deterministic duplicate install rejection
- optional `replace=true` support
- explicit error strings for missing `SKILL.md`, missing `skill_id`, bad
  archive roots, unknown remove targets, and unknown invoke targets

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools::external_skills:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/tools/external_skills.rs
git commit -m "fix: harden external skill lifecycle errors"
```

### Task 6: Update Migration Messaging And Readme Documentation

**Files:**
- Modify: `crates/app/src/migration/mod.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Test: `crates/app/src/migration/mod.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn external_skill_warning_points_to_explicit_runtime_install_flow() {
    let warning = external_skill_warning(&artifact_fixture("skills_dir"));
    assert!(warning.contains("install"));
    assert!(warning.contains("invoke"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app external_skill_warning_points_to_explicit_runtime_install_flow -- --exact`

Expected: FAIL because the warning still says LoongClaw does not auto-wire the runtime.

**Step 3: Write minimal implementation**

Update migration warning/docs to say:

- migrated skills are still not auto-installed
- the new runtime flow is `fetch/install/list/invoke`

Update README examples and tool lists accordingly.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app migration:: -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/migration/mod.rs README.md README.zh-CN.md
git commit -m "docs: document external skills runtime lifecycle"
```

### Task 7: Full Verification

**Files:**
- No code changes required

**Step 1: Run focused external-skills tests**

Run: `cargo test -p loongclaw-app external_skills -- --nocapture`

Expected: PASS.

**Step 2: Run provider and migration regressions**

Run: `cargo test -p loongclaw-app provider::tests:: -- --nocapture`

Run: `cargo test -p loongclaw-app migration:: -- --nocapture`

Expected: PASS.

**Step 3: Run full workspace tests**

Run: `cargo test`

Expected: PASS across the workspace.

**Step 4: Inspect working tree**

Run: `git status --short`

Expected: only external-skills runtime closure files are modified.

**Step 5: Commit**

```bash
git add docs/plans/2026-03-12-external-skills-runtime-closure-design.md docs/plans/2026-03-12-external-skills-runtime-closure.md
git add crates/app/src/config/tools_memory.rs crates/app/src/config/runtime.rs crates/app/src/tools/runtime_config.rs crates/app/src/tools/external_skills.rs crates/app/src/tools/mod.rs crates/app/src/provider/mod.rs crates/app/src/migration/mod.rs README.md README.zh-CN.md crates/app/Cargo.toml
git commit -m "feat: close external skills runtime lifecycle"
```
