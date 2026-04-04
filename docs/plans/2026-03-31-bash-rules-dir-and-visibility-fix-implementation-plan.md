# Bash Rules Dir And Visibility Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the next two scoped follow-ups for issue `#637`: make default `bash.exec` rules-directory resolution match `~/.loongclaw/rules` expectations when the config already lives under `.loongclaw/`, and fail-close `bash.exec` discoverability / runtime visibility when governance rule loading fails.

**Architecture:** Preserve the shipped AST governance execution semantics from the previous slice. Narrow the path fix to the default rules-directory helper so configs stored inside a `.loongclaw` directory resolve to sibling `rules/` instead of nesting another `.loongclaw/`. Keep direct `bash.exec` execution fail-closed with the existing explicit `policy_denied` load-error message, but add a separate discoverability helper so tool search, runtime catalog visibility, and delegate-child runtime visibility all hide `bash.exec` when governance loading failed.

**Tech Stack:** Rust, existing config/runtime/tool catalog layers in `crates/app`, Cargo unit tests, CLI smoke testing through `loongclaw chat`.

**Correctness Review Mode:** `auto-fix`

**Style Review Mode:** `single-pass`

---

## File Structure

- Modify: `crates/app/src/config/tools.rs`
  - Refine `BashToolConfig::resolved_rules_dir(...)` default semantics and add targeted config-path resolution tests.
- Modify: `crates/app/src/tools/runtime_config.rs`
  - Add a `bash.exec` discoverability helper distinct from runtime process readiness and cover it with focused unit tests.
- Modify: `crates/app/src/tools/mod.rs`
  - Make `tool.search` hide `bash.exec` when governance rules failed to load, with focused tests.
- Modify: `crates/app/src/tools/catalog.rs`
  - Make `ToolVisibilityGate::BashRuntime` fail closed on governance load errors for runtime views and delegate-child visibility, with focused tests.
- Modify: `docs/plans/2026-03-31-bash-rules-dir-and-visibility-fix-implementation-plan.md`
  - Check off progress as tasks complete.

## Implementation Notes

- This plan is the next follow-up slice after the committed AST governance implementation at `f3664b77`.
- Scope in:
  - configs whose parent directory is already named `.loongclaw` should default `bash.exec` rules to `<that-dir>/rules`
  - configs outside `.loongclaw` should continue defaulting to `<config-parent>/.loongclaw/rules`
  - explicit `tools.bash.rules_dir` overrides keep their existing absolute-vs-relative semantics
  - `bash.exec` execution should still return the existing governance load-error denial when a broken rule file exists
  - broken governance loading should also hide `bash.exec` from tool search, runtime catalog visibility, and delegate-child runtime visibility
- Scope out:
  - `approval_required`
  - `shell.exec` convergence
  - changing Starlark rule syntax
  - changing the current missing-rules-dir behavior
  - any repo-wide or other-user-visible config migration outside this branch

### Task 1: Fix default rules-directory semantics for configs already inside `.loongclaw`

**Files:**
- Modify: `crates/app/src/config/tools.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Test: `crates/app/src/config/tools.rs`
- Test: `crates/app/src/tools/runtime_config.rs`

- [x] **Step 1: Add red tests for the non-nested `.loongclaw/rules` expectation**

Add config-level resolution coverage in `crates/app/src/config/tools.rs`:

```rust
#[test]
fn bash_tool_config_resolves_rules_dir_without_double_loongclaw_segment() {
    let config = BashToolConfig::default();

    let resolved = config.resolved_rules_dir(Some(std::path::Path::new(
        "/home/test/.loongclaw/config.toml",
    )));

    assert_eq!(resolved, std::path::PathBuf::from("/home/test/.loongclaw/rules"));
}
```

Add runtime-config coverage in `crates/app/src/tools/runtime_config.rs`:

```rust
#[test]
fn tool_runtime_config_uses_loongclaw_home_rules_dir_when_config_lives_inside_loongclaw_dir() {
    let runtime = ToolRuntimeConfig::from_loongclaw_config(
        &LoongClawConfig::default(),
        Some(std::path::Path::new("/home/test/.loongclaw/config.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        PathBuf::from("/home/test/.loongclaw/rules")
    );
}
```

Keep the existing workspace-style expectation green:

```rust
#[test]
fn tool_runtime_config_uses_default_workspace_rules_dir_when_unset() {
    let runtime = ToolRuntimeConfig::from_loongclaw_config(
        &LoongClawConfig::default(),
        Some(std::path::Path::new("/tmp/work/loongclaw.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        PathBuf::from("/tmp/work/.loongclaw/rules")
    );
}
```

- [x] **Step 2: Run the red tests**

Run:

- `cargo test -p loongclaw-app bash_tool_config_resolves_rules_dir_without_double_loongclaw_segment --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_uses_loongclaw_home_rules_dir_when_config_lives_inside_loongclaw_dir --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_uses_default_workspace_rules_dir_when_unset --lib -- --exact --nocapture`

Expected:

- FAIL because the current default helper returns `/home/test/.loongclaw/.loongclaw/rules`
- PASS for the existing workspace-local expectation because configs outside `.loongclaw` should keep resolving to `<config-parent>/.loongclaw/rules`

- [x] **Step 3: Implement the minimal path-resolution fix**

In `crates/app/src/config/tools.rs`, keep explicit override handling unchanged and only refine the default branch. A minimal shape is:

```rust
fn default_bash_rules_dir(base_dir: &Path) -> PathBuf {
    if base_dir.file_name().is_some_and(|name| name == ".loongclaw") {
        base_dir.join("rules")
    } else {
        base_dir.join(".loongclaw").join("rules")
    }
}
```

Then use that helper from `BashToolConfig::resolved_rules_dir(...)` when `rules_dir` is unset.

- [x] **Step 4: Re-run the focused tests**

Run the three commands from Step 2 again.

Expected:

- PASS with `/home/test/.loongclaw/config.toml -> /home/test/.loongclaw/rules`
- PASS with `/tmp/work/loongclaw.toml -> /tmp/work/.loongclaw/rules`

### Task 2: Fail-close `bash.exec` discoverability when governance rule loading fails

**Files:**
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Test: `crates/app/src/tools/runtime_config.rs`
- Test: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/catalog.rs`

- [x] **Step 1: Add red tests for runtime discoverability vs execution readiness**

Add a focused unit test in `crates/app/src/tools/runtime_config.rs`:

```rust
#[test]
fn bash_exec_discoverability_requires_runtime_ready_and_governance_load_success() {
    let ready = BashExecRuntimePolicy {
        available: true,
        command: Some(PathBuf::from("bash")),
        ..BashExecRuntimePolicy::default()
    };
    assert!(ready.is_runtime_ready());
    assert!(ready.is_discoverable());

    let broken = BashExecRuntimePolicy {
        governance: BashGovernanceRuntimePolicy {
            load_error: Some("broken rules".to_owned()),
            ..BashGovernanceRuntimePolicy::default()
        },
        ..ready.clone()
    };
    assert!(broken.is_runtime_ready());
    assert!(!broken.is_discoverable());
}
```

Add tool-search coverage in `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_hides_bash_exec_when_governance_rules_failed_to_load() {
    let root = unique_tool_temp_dir("loongclaw-bash-tool-search-broken-rules");
    std::fs::create_dir_all(&root).expect("create root dir");

    let mut config = test_tool_runtime_config(root);
    config.bash_exec = ready_bash_exec_runtime_policy();
    config.bash_exec.governance.load_error = Some("broken rules".to_owned());

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "bash.exec", "limit": 10}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().all(|entry| entry["tool_id"] != "bash.exec"));
}
```

Add catalog visibility coverage in `crates/app/src/tools/catalog.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn bash_runtime_visibility_gate_hides_bash_exec_when_governance_rules_failed_to_load() {
    let runtime = ToolRuntimeConfig {
        bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(std::path::PathBuf::from("bash")),
            governance: crate::tools::runtime_config::BashGovernanceRuntimePolicy {
                load_error: Some("broken rules".to_owned()),
                ..crate::tools::runtime_config::BashGovernanceRuntimePolicy::default()
            },
            ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
        },
        ..ToolRuntimeConfig::default()
    };

    assert!(!tool_visibility_gate_enabled_for_runtime_policy(
        ToolVisibilityGate::BashRuntime,
        &runtime,
    ));
}
```

Add delegate-child coverage in `crates/app/src/tools/catalog.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn delegate_child_tool_view_hides_allowlisted_bash_exec_when_governance_rules_failed_to_load() {
    let mut config = ToolConfig::default();
    config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];
    let runtime = ToolRuntimeConfig {
        bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(std::path::PathBuf::from("bash")),
            governance: crate::tools::runtime_config::BashGovernanceRuntimePolicy {
                load_error: Some("broken rules".to_owned()),
                ..crate::tools::runtime_config::BashGovernanceRuntimePolicy::default()
            },
            ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
        },
        ..ToolRuntimeConfig::default()
    };

    let child_view = delegate_child_tool_view_for_runtime_config(&config, &runtime);

    assert!(!child_view.contains("bash.exec"));
}
```

- [x] **Step 2: Run the red tests**

Run:

- `cargo test -p loongclaw-app bash_exec_discoverability_requires_runtime_ready_and_governance_load_success --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_search_hides_bash_exec_when_governance_rules_failed_to_load --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_runtime_visibility_gate_hides_bash_exec_when_governance_rules_failed_to_load --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app delegate_child_tool_view_hides_allowlisted_bash_exec_when_governance_rules_failed_to_load --lib -- --exact --nocapture`

Expected:

- FAIL because `BashExecRuntimePolicy` does not yet distinguish discoverability from runtime readiness
- FAIL because `tool.search` and `ToolVisibilityGate::BashRuntime` still only check `is_runtime_ready()`

- [x] **Step 3: Implement a separate discoverability helper and route visibility through it**

In `crates/app/src/tools/runtime_config.rs`, add:

```rust
impl BashExecRuntimePolicy {
    #[must_use]
    pub fn is_discoverable(&self) -> bool {
        self.is_runtime_ready() && self.governance.load_error.is_none()
    }
}
```

Then:

- in `crates/app/src/tools/mod.rs`, use `config.bash_exec.is_discoverable()` for the `bash.exec` branch inside tool-search runtime usability
- in `crates/app/src/tools/catalog.rs`, use `config.bash_exec.is_discoverable()` for `ToolVisibilityGate::BashRuntime`
- do **not** replace the execution-path `is_runtime_ready()` guard in `crates/app/src/tools/bash.rs`; keep the explicit governance load-error denial there

- [x] **Step 4: Re-run the focused tests**

Run the four commands from Step 2 again.

Expected:

- PASS for discoverability helper semantics
- PASS for tool search / runtime visibility / delegate-child hiding
- existing `bash_exec_fails_closed_when_rule_loading_failed` test remains green

### Task 3: Regression verification, manual smoke, and stage commit

**Files:**
- Modify: `docs/plans/2026-03-31-bash-rules-dir-and-visibility-fix-implementation-plan.md`

- [ ] **Step 1: Run grouped focused coverage**

Run:

- `cargo test -p loongclaw-app bash_tool_config_resolves_rules_dir_without_double_loongclaw_segment --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_uses_loongclaw_home_rules_dir_when_config_lives_inside_loongclaw_dir --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_discoverability_requires_runtime_ready_and_governance_load_success --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_search_hides_bash_exec_when_governance_rules_failed_to_load --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_runtime_visibility_gate_hides_bash_exec_when_governance_rules_failed_to_load --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app delegate_child_tool_view_hides_allowlisted_bash_exec_when_governance_rules_failed_to_load --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_fails_closed_when_rule_loading_failed --lib -- --exact --nocapture`

- [ ] **Step 2: Run CI-parity verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Expected:

- PASS
- if the workspace tests still need the known escalated environment because of `browser_companion / wait-timeout`, document that in the task log before treating the run as complete

- [ ] **Step 3: Run the manual CLI smoke after code is fixed**

Before the smoke:

- if a temporary `[tools.bash].rules_dir = "/home/threeice/.loongclaw/rules"` override was added only to work around the old behavior, remove it so the default-path fix is actually exercised

Run:

- `target/debug/loongclaw chat --config /home/threeice/.loongclaw/config.toml --session bash-rules-smoke`

Inside the chat, manually verify:

- `printf ok` is allowed
- `git status --short` is allowed
- `git rev-parse HEAD` is denied by default mode
- `cargo publish --dry-run` is denied
- `c\argo publish --dry-run` is also denied

- [ ] **Step 4: Update plan progress and commit the completed implementation stage**

After verification and reviews converge:

- check off completed boxes in this plan
- `git add` the touched source files and this plan
- commit with a subject in the style of `fix(app): align bash rules path and visibility`
