# Bash Rules Dir Common-Semantics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the follow-up `bash.exec` rules-directory bug by changing `tools.bash.rules_dir` to match the repository's common path semantics instead of treating the loaded `config.toml` location as an implicit config root.

**Architecture:** Keep the shipped `bash.exec` AST governance, discoverability fail-close behavior, and existing kernel/bootstrap wiring intact. Narrow this slice to one semantic correction: when `tools.bash.rules_dir` is unset, default to `default_loongclaw_home()/rules`; when it is set, resolve it with the same `expand_path(...)` semantics used by other config path fields. Do not propagate `config_path` through unrelated runtime surfaces just to preserve the old Bash-specific config-relative behavior; that behavior is now intentionally removed.

**Tech Stack:** Rust, existing config/runtime/tool execution layers in `crates/app`, Cargo unit tests, existing `bash.exec` governance regression harnesses.

**Correctness Review Mode:** `auto-fix`

**Style Review Mode:** `single-pass`

---

## File Structure

- Modify: `crates/app/src/config/tools.rs`
  - Remove the Bash-specific config-relative `rules_dir` helper behavior and replace it with repo-common default/override semantics plus targeted tests.
- Modify: `crates/app/src/tools/runtime_config.rs`
  - Project the new `bash.rules_dir` semantics into runtime config and adjust focused tests to assert home-default behavior instead of config-relative behavior.
- Modify: `crates/app/src/tools/mod.rs`
  - Add an end-to-end regression test that reproduces the real manual smoke shape: home-scoped rules, repo cwd, no local `.loongclaw/rules`, and a `ToolRuntimeConfig::from_loongclaw_config(config, None)` execution path.
- Modify: `docs/plans/2026-04-01-bash-rules-dir-common-semantics-implementation-plan.md`
  - Check off progress as tasks complete during implementation.

## Implementation Notes

- This plan is a targeted follow-up slice to the committed `bash.exec` AST governance work. It is meant to keep the original work moving, not replace it with a repo-wide runtime-config cleanup.
- This plan intentionally supersedes the earlier Bash-specific assumption from the original AST plan that default rules resolution should derive from `config_path.parent()`.
- Repo-common semantics for this slice mean:
  - explicit `tools.bash.rules_dir` uses `expand_path(...)`
  - unset `tools.bash.rules_dir` defaults to `default_loongclaw_home().join("rules")`
  - no implicit rebasing of Bash rules paths against the loaded config file path
- Missing rules directory remains a non-error empty ruleset.
- Broken `*.rules` files remain fail-closed.
- Existing discoverability/runtime-visibility fail-close behavior for `governance.load_error` remains in scope only as preserved behavior; this slice does not redesign it.

## Scope In

- Bash rules-directory resolution semantics
- Runtime projection of those semantics into `ToolRuntimeConfig`
- End-to-end regression coverage for the real manual-smoke failure shape

## Scope Out

- Threading `resolved_config_path` through kernel/chat/provider/conversation call sites
- Changing approval, `shell.exec`, or `bash.exec` AST evaluation behavior
- Changing rule syntax, missing-dir semantics, or discoverability policy

### Task 1: Re-define `bash.rules_dir` to follow repo-common path semantics

**Files:**
- Modify: `crates/app/src/config/tools.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Test: `crates/app/src/config/tools.rs`
- Test: `crates/app/src/tools/runtime_config.rs`

- [x] **Step 1: Add red tests for the new default and override semantics**

Add focused tests in `crates/app/src/config/tools.rs` that pin the intended semantics:

```rust
#[test]
fn bash_tool_config_defaults_to_loongclaw_home_rules_dir() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("HOME", home.path());

    assert_eq!(
        BashToolConfig::default().resolved_rules_dir(),
        home.path().join(".loongclaw").join("rules")
    );
}

#[test]
fn bash_tool_config_resolves_relative_rules_dir_like_other_path_fields() {
    let config = BashToolConfig {
        rules_dir: Some("custom/rules".to_owned()),
        ..BashToolConfig::default()
    };

    assert_eq!(config.resolved_rules_dir(), PathBuf::from("custom/rules"));
}
```

Add or adjust focused runtime-config tests in `crates/app/src/tools/runtime_config.rs`:

```rust
#[test]
fn tool_runtime_config_uses_loongclaw_home_rules_dir_when_unset() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("HOME", home.path());

    let runtime = ToolRuntimeConfig::from_loongclaw_config(
        &LoongClawConfig::default(),
        Some(std::path::Path::new("/tmp/work/loongclaw.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        home.path().join(".loongclaw").join("rules")
    );
}

#[test]
fn tool_runtime_config_keeps_relative_bash_rules_dir_override_relative() {
    let config: crate::config::ToolConfig =
        toml::from_str("[bash]\nrules_dir = \"custom/rules\"\n").expect("bash tool config");
    let loongclaw = crate::config::LoongClawConfig {
        tools: config,
        ..crate::config::LoongClawConfig::default()
    };

    let runtime = ToolRuntimeConfig::from_loongclaw_config(
        &loongclaw,
        Some(std::path::Path::new("/tmp/work/loongclaw.toml")),
    );

    assert_eq!(runtime.bash_exec.governance.rules_dir, PathBuf::from("custom/rules"));
}
```

- [x] **Step 2: Run the red tests**

Run:

- `cargo test -p loongclaw-app bash_tool_config_defaults_to_loongclaw_home_rules_dir --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_tool_config_resolves_relative_rules_dir_like_other_path_fields --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tools::runtime_config::tests::tool_runtime_config_uses_loongclaw_home_rules_dir_when_unset --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tools::runtime_config::tests::tool_runtime_config_keeps_relative_bash_rules_dir_override_relative --lib -- --exact --nocapture`

Expected:

- FAIL because current Bash rules resolution still derives defaults and relative overrides from `config_path.parent()` / `cwd` special cases

- [x] **Step 3: Implement the semantic correction**

In `crates/app/src/config/tools.rs`:

- remove the `config_path` dependency from `BashToolConfig::resolved_rules_dir(...)`
- replace the default helper with `default_loongclaw_home().join("rules")`
- keep explicit overrides aligned with repo-common path behavior by returning `expand_path(rules_dir)`

The target shape is:

```rust
impl BashToolConfig {
    pub fn resolved_rules_dir(&self) -> PathBuf {
        self.rules_dir
            .as_deref()
            .map(expand_path)
            .unwrap_or_else(|| default_loongclaw_home().join("rules"))
    }
}
```

In `crates/app/src/tools/runtime_config.rs`:

- update the Bash governance builder call sites to use the new zero-argument helper
- keep all other `config_path` consumers unchanged

- [x] **Step 4: Re-run focused config/runtime tests**

Run:

- `cargo test -p loongclaw-app bash_tool_config_ --lib -- --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_uses_loongclaw_home_rules_dir_when_unset --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_keeps_relative_bash_rules_dir_override_relative --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_governance_runtime_treats_missing_rules_dir_as_empty_rule_set --lib -- --exact --nocapture`

Expected:

- PASS, with no remaining config-relative default/override expectations in Bash path resolution tests

### Task 2: Add the real-world regression test that previously failed in manual chat smoke

**Files:**
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/mod.rs`

- [ ] **Step 1: Add a regression test for home-default rules with repo cwd and `config + None` runtime construction**

Add a focused end-to-end test in `crates/app/src/tools/mod.rs` that mirrors the actual failure shape:

```rust
#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_uses_loongclaw_home_rules_dir_even_when_runtime_is_built_without_config_path() {
    use std::fs;

    let home = unique_tool_temp_dir("loongclaw-bash-home-rules");
    let workspace = unique_tool_temp_dir("loongclaw-bash-home-rules-workspace");
    let rules_dir = home.join(".loongclaw").join("rules");
    fs::create_dir_all(&rules_dir).expect("rules dir");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        rules_dir.join("allow.rules"),
        "prefix_rule(pattern=[\"printf\",\"ok\"], decision=\"allow\")\n",
    )
    .expect("rule file");
    fs::write(home.join(".loongclaw").join("config.toml"), "").expect("config");

    let mut env = ScopedEnv::new();
    env.set("HOME", &home);
    let _cwd = crate::test_support::ScopedCurrentDir::new(&workspace);

    let mut runtime = runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        &crate::config::LoongClawConfig::default(),
        None,
    );
    let (bash_exec, _log_path) = configured_test_bash_runtime_with_rules(&home);
    runtime.bash_exec = bash_exec;

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf ok"}),
        },
        &runtime,
    )
    .expect("home-default rules should permit execution");

    assert_eq!(outcome.status, "ok");
}
```

Keep the workspace root free of local `.loongclaw/rules` fixtures so the test fails for the old code for the same reason the manual smoke failed.

- [ ] **Step 2: Run the red regression test**

Run:

- `cargo test -p loongclaw-app bash_exec_uses_loongclaw_home_rules_dir_even_when_runtime_is_built_without_config_path --lib -- --exact --nocapture`

Expected:

- FAIL on the old code because the runtime resolves its default rules dir away from `HOME/.loongclaw/rules`

- [ ] **Step 3: Finalize the regression after Task 1 lands**

Once Task 1 is implemented, make any small fixture-path adjustments needed so the new regression uses the same helper assumptions as production, then re-run:

- `cargo test -p loongclaw-app bash_exec_uses_loongclaw_home_rules_dir_even_when_runtime_is_built_without_config_path --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_ --lib -- --nocapture`
- `cargo test -p loongclaw-app runtime_tool_view_ --lib -- --nocapture`

Expected:

- PASS, proving the manual-smoke shape no longer depends on config-relative path propagation

### Task 3: Verify the slice, run review, and capture the milestone

**Files:**
- Modify: `docs/plans/2026-04-01-bash-rules-dir-common-semantics-implementation-plan.md`

- [ ] **Step 1: Run required repo verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Expected:

- PASS, with the known browser-companion environment issue handled the same way as the current branch policy

- [ ] **Step 2: Run the required implementation review loop**

Follow the active superpower implementation workflow:

- correctness review in `auto-fix` mode
- style review in `single-pass` mode
- re-run focused tests and repo verification after any scope-in fixes

- [ ] **Step 3: Mark the plan complete and commit the implementation slice**

Update this plan file's checkboxes to reflect the final state, then create a local milestone commit after verification and review gates pass.
