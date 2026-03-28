# Bash Exec Basic Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the first implementation slice for issue `#637`: an experimental `bash.exec` tool that executes a Bash command string through the existing LoongClaw tool pipeline, without changing `shell.exec`.

**Architecture:** Keep `bash.exec` as a separate discoverable runtime tool. Add a small `BashExecRuntimePolicy` to runtime config, plus a minimal Bash tool config that defaults to non-login shell execution and can opt into login-shell mode. Probe for a usable Bash executable when building runtime policy, hide the tool from discoverability surfaces when Bash is unavailable, and execute commands through `bash -c <command>` by default, switching to `bash -lc <command>` only when the config explicitly enables login-shell mode. Keep the same timeout/captured-output conventions used by `shell.exec`. Extract the generic child-process timeout/output code into a shared helper so `bash.exec` does not fork a second subprocess implementation.

**Tech Stack:** Rust, Tokio process execution, serde_json tool payloads, Cargo unit tests, existing tool catalog/runtime config infrastructure in `crates/app`

---

## File Structure

- Create: `crates/app/src/tools/process_exec.rs`
  - Shared subprocess helper for capped stdout/stderr capture, timeout handling, and runtime-safe async execution.
- Create: `crates/app/src/tools/bash.rs`
  - `bash.exec` payload parsing, Bash runtime probing, and Bash command execution.
- Modify: `crates/app/src/config/tools.rs`
  - Add the minimal Bash tool config surface for `tools.bash.login_shell`, defaulting to `false`.
- Modify: `crates/app/src/tools/shell.rs`
  - Reuse the shared subprocess helper without changing existing `shell.exec` policy semantics.
- Modify: `crates/app/src/tools/runtime_config.rs`
  - Add `BashExecRuntimePolicy`, project `tools.bash.login_shell` into runtime config, wire Bash probing into `ToolRuntimeConfig::from_loongclaw_config` / `from_env`, and emit a warning when Bash is unavailable.
- Modify: `crates/app/src/tools/catalog.rs`
  - Add the `bash.exec` descriptor, provider schema, search hint metadata, and discoverability tags.
- Modify: `crates/app/src/tools/mod.rs`
  - Register the new module, dispatch `bash.exec`, hide it from discoverability when unavailable, add alias/timeout coverage, and add focused tests.

## Implementation Notes

- Treat this plan as the first implementation slice under [`docs/plans/2026-03-27-bash-exec-basic-tool-design.md`](docs/plans/2026-03-27-bash-exec-basic-tool-design.md), not as the full implementation of issue `#637`.
- Default to non-login shell execution via `bash -c <command>`.
- Add a config-controlled login-shell option at `tools.bash.login_shell`; when it is explicitly enabled, switch execution to `bash -lc <command>`.
- Probe order for Bash runtime:
  - try `bash` from `PATH` first
  - on Windows, if that fails, try a small list of common absolute-path candidates such as Git Bash locations
- The unavailable-runtime warning should be emitted while building runtime policy, using the existing `eprintln!("warning: ...")` pattern already used in `runtime_config.rs`.
- Keep `shell.exec` behavior unchanged except for routing its subprocess work through the new shared helper.
- `bash.exec` remains discoverable-only. It does not become a provider-core tool in this slice.

## Scope-Out Follow-up

- Keep follow-up prompt design conservative. If discovery-followup behavior still needs refinement after this slice, prefer a structured coordinator-side mechanism for `tool.search` continuation instead of expanding tool-specific follow-up prompt branching.
- In particular, replacing the current `tool.search`-specific follow-up prompt selection with a more explicit discovery-to-invocation handoff should be tracked as later work, not folded further into this basic `bash.exec` slice.

### Task 1: Add the runtime availability contract for `bash.exec`

**Files:**
- Create: `crates/app/src/tools/bash.rs`
- Modify: `crates/app/src/config/tools.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`

- [ ] **Step 1: Write the failing runtime-config test**

Add this test to `crates/app/src/tools/runtime_config.rs`:

```rust
#[test]
fn tool_runtime_config_default_marks_bash_exec_unavailable() {
    let config = ToolRuntimeConfig::default();
    assert!(!config.bash_exec.is_runtime_ready());
    assert!(config.bash_exec.command.is_none());
    assert!(config.bash_exec.warning.is_none());
    assert!(!config.bash_exec.login_shell);
}

#[test]
fn tool_runtime_config_projects_bash_login_shell_flag() {
    let config: crate::config::ToolConfig =
        toml::from_str("[bash]\nlogin_shell = true\n").expect("toml");
    let loongclaw = crate::config::LoongClawConfig {
        tools: config,
        ..crate::config::LoongClawConfig::default()
    };

    let runtime = ToolRuntimeConfig::from_loongclaw_config(&loongclaw, None);
    assert!(runtime.bash_exec.login_shell);
}
```

- [ ] **Step 2: Write the failing runtime-probe tests**

Create `crates/app/src/tools/bash.rs` with red tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_bash_runtime_prefers_path_bash_before_windows_fallbacks() {
        let candidates = bash_runtime_candidates();
        assert_eq!(
            candidates.first().map(|candidate| candidate.as_os_str()),
            Some(std::ffi::OsStr::new("bash"))
        );
    }

    #[test]
    fn unavailable_runtime_policy_carries_warning() {
        let policy = unavailable_bash_runtime_policy();
        assert!(!policy.available);
        assert!(policy.command.is_none());
        assert_eq!(
            policy.warning.as_deref(),
            Some("bash unavailable; hiding bash.exec from runtime tool surface")
        );
    }

    #[test]
    fn bash_exec_arg_builder_defaults_to_non_login_shell() {
        assert_eq!(
            bash_exec_args("echo hi", false),
            vec!["-c".to_owned(), "echo hi".to_owned()]
        );
    }

    #[test]
    fn bash_exec_arg_builder_uses_login_shell_when_enabled() {
        assert_eq!(
            bash_exec_args("echo hi", true),
            vec!["-lc".to_owned(), "echo hi".to_owned()]
        );
    }
}
```

- [ ] **Step 3: Run the red tests**

Run:

- `cargo test -p loongclaw-app tool_runtime_config_default_marks_bash_exec_unavailable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_projects_bash_login_shell_flag --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app unavailable_runtime_policy_carries_warning --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_arg_builder_defaults_to_non_login_shell --lib -- --exact --nocapture`

Expected:

- FAIL because `ToolRuntimeConfig` does not yet have `bash_exec`
- FAIL because the config model does not yet have `tools.bash.login_shell`
- FAIL because `bash.rs` and the probe helpers do not exist yet

- [ ] **Step 4: Implement the runtime policy**

In `crates/app/src/tools/runtime_config.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BashExecRuntimePolicy {
    pub available: bool,
    pub command: Option<PathBuf>,
    pub warning: Option<String>,
    pub login_shell: bool,
}

impl BashExecRuntimePolicy {
    #[must_use]
    pub fn is_runtime_ready(&self) -> bool {
        self.available && self.command.is_some()
    }
}
```

Add `pub bash_exec: BashExecRuntimePolicy` to `ToolRuntimeConfig`.

In `crates/app/src/tools/bash.rs`, add:

```rust
pub(super) fn unavailable_bash_runtime_policy() -> super::runtime_config::BashExecRuntimePolicy {
    super::runtime_config::BashExecRuntimePolicy {
        available: false,
        command: None,
        warning: Some("bash unavailable; hiding bash.exec from runtime tool surface".to_owned()),
        login_shell: false,
    }
}

pub(super) fn bash_runtime_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = vec![std::path::PathBuf::from("bash")];
    #[cfg(windows)]
    {
        candidates.push(std::path::PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"));
        candidates.push(std::path::PathBuf::from(r"C:\Program Files\Git\usr\bin\bash.exe"));
    }
    candidates
}

pub(super) fn bash_exec_args(command: &str, login_shell: bool) -> Vec<String> {
    if login_shell {
        vec!["-lc".to_owned(), command.to_owned()]
    } else {
        vec!["-c".to_owned(), command.to_owned()]
    }
}
```

Add the minimal config surface in `crates/app/src/config/tools.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BashToolConfig {
    #[serde(default)]
    pub login_shell: bool,
}
```

Also add a `detect_bash_runtime_policy()` helper that:

- iterates the candidates in order
- accepts the first candidate whose `--version` probe returns success
- returns `available = true` and `command = Some(candidate)` on success
- otherwise returns `unavailable_bash_runtime_policy()`

On Windows, do not treat a fallback absolute path as available unless the same
`--version` probe succeeds. The fallback list must stay fail-closed.

Wire `BashToolConfig` into `ToolConfig`.

In `ToolRuntimeConfig::from_loongclaw_config` and `ToolRuntimeConfig::from_env`:

- populate `bash_exec` from `crate::tools::bash::detect_bash_runtime_policy()`
- set `bash_exec.login_shell` from `config.tools.bash.login_shell` in `from_loongclaw_config`
- default `bash_exec.login_shell` to `false` in `Default` and `from_env`
- if `warning` is present, emit it with `eprintln!("warning: {warning}")`

- [ ] **Step 5: Run the green tests**

Run:

- `cargo test -p loongclaw-app tool_runtime_config_default_marks_bash_exec_unavailable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app unavailable_runtime_policy_carries_warning --lib -- --exact --nocapture`

Expected: PASS

- [ ] **Step 6: Commit the runtime-policy slice**

Run:

- `git add crates/app/src/tools/bash.rs crates/app/src/tools/runtime_config.rs`
- `git commit -m "feat(app): add bash.exec runtime availability policy"`

### Task 2: Add the `bash.exec` visibility, catalog, and dispatch surface

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`

- [ ] **Step 1: Write the failing runtime-view visibility tests**

Add these tests to `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn runtime_tool_view_hides_bash_exec_when_runtime_is_unavailable() {
    let config = runtime_config::ToolRuntimeConfig::default();
    let view = runtime_tool_view_for_runtime_config(&config);
    assert!(!view.contains("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn runtime_tool_view_includes_bash_exec_when_runtime_is_available() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };
    let view = runtime_tool_view_for_runtime_config(&config);
    assert!(view.contains("bash.exec"));
}
```

- [ ] **Step 2: Write the failing discoverability tests**

Add these tests to `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_hides_bash_exec_when_runtime_is_unavailable() {
    let config = runtime_config::ToolRuntimeConfig::default();

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "bash shell command"}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().all(|entry| entry["tool_id"] != "bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_includes_bash_exec_when_runtime_is_available() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "bash shell command"}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().any(|entry| entry["tool_id"] == "bash.exec"));
}
```

- [ ] **Step 3: Write the failing catalog/schema test**

Add this test to `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_catalog_exposes_command_cwd_and_timeout_ms() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("bash.exec")
        .expect("bash.exec should be in the catalog");
    let definition = descriptor.provider_definition();
    let properties = definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("bash.exec parameters");

    assert!(properties.contains_key("command"));
    assert!(properties.contains_key("cwd"));
    assert!(properties.contains_key("timeout_ms"));
    assert!(!properties.contains_key("args"));

    let entry = catalog::find_tool_catalog_entry("bash.exec")
        .expect("bash.exec should be in catalog entries");
    assert_eq!(
        entry.argument_hint,
        "command:string,cwd?:string,timeout_ms?:integer"
    );
}
```

- [ ] **Step 4: Write the failing canonical-name and timeout tests**

Add these tests to `crates/app/src/tools/mod.rs`:

```rust
#[test]
fn canonical_tool_name_maps_bash_exec_provider_name() {
    assert_eq!(canonical_tool_name("bash_exec"), "bash.exec");
}

#[test]
fn framework_timeout_treats_bash_exec_as_dedicated_timeout_tool() {
    assert!(tool_uses_dedicated_timeout("bash.exec"));
}
```

- [ ] **Step 5: Run the red tests**

Run:

- `cargo test -p loongclaw-app runtime_tool_view_hides_bash_exec_when_runtime_is_unavailable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app runtime_tool_view_includes_bash_exec_when_runtime_is_available --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_search_hides_bash_exec_when_runtime_is_unavailable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_search_includes_bash_exec_when_runtime_is_available --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_catalog_exposes_command_cwd_and_timeout_ms --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app canonical_tool_name_maps_bash_exec_provider_name --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app framework_timeout_treats_bash_exec_as_dedicated_timeout_tool --lib -- --exact --nocapture`

Expected:

- FAIL because the catalog has no `bash.exec` descriptor yet
- FAIL because the runtime tool view does not know how to hide/show `bash.exec`
- FAIL because the timeout helper does not yet include `bash.exec`

- [ ] **Step 6: Implement the visibility, catalog, and dispatch wiring**

In `crates/app/src/tools/catalog.rs`, add:

```rust
pub enum ToolVisibilityGate {
    Always,
    Sessions,
    SessionMutation,
    Messages,
    Delegate,
    Browser,
    BrowserCompanion,
    BashRuntime,
    ExternalSkills,
    MemoryFileRoot,
    WebFetch,
    WebSearch,
}
```

Update the gate functions:

```rust
ToolVisibilityGate::BashRuntime => config.bash_exec.is_runtime_ready(),
```

for `tool_visibility_gate_enabled_for_runtime_view(...)`, and the same:

```rust
ToolVisibilityGate::BashRuntime => config.bash_exec.is_runtime_ready(),
```

for `tool_visibility_gate_enabled_for_runtime_policy(...)`.

Add a descriptor:

```rust
ToolDescriptor {
    name: "bash.exec",
    provider_name: "bash_exec",
    aliases: &[],
    description: "Execute one Bash command string through a fresh Bash process",
    execution_kind: ToolExecutionKind::Core,
    availability: ToolAvailability::Runtime,
    exposure: ToolExposureClass::Discoverable,
    visibility_gate: ToolVisibilityGate::BashRuntime,
    capability_action_class: CapabilityActionClass::ExecuteExisting,
    policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
    provider_definition_builder: bash_exec_definition,
}
```

Add `bash_exec_definition(...)` with:

```rust
"required": ["command"],
"additionalProperties": false,
"properties": {
    "command": { "type": "string" },
    "cwd": { "type": "string" },
    "timeout_ms": {
        "type": "integer",
        "minimum": 1000,
        "maximum": 600000
    }
}
```

Update:

- `tool_argument_hint("bash.exec")`
- `tool_parameter_types("bash.exec")`
- `tool_required_fields("bash.exec")`
- `tool_tags("bash.exec")`

In `crates/app/src/tools/mod.rs`:

- add `mod bash;`
- add `"bash.exec" => bash::execute_bash_tool_with_config(request, config),`
- update `tool_search_entry_is_runtime_usable(...)` with:

```rust
"bash.exec" => config.bash_exec.is_runtime_ready(),
```

- include `bash.exec` in `tool_uses_dedicated_timeout(...)`

- [ ] **Step 7: Run the green tests**

Run:

- `cargo test -p loongclaw-app runtime_tool_view_hides_bash_exec_when_runtime_is_unavailable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app runtime_tool_view_includes_bash_exec_when_runtime_is_available --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_search_hides_bash_exec_when_runtime_is_unavailable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_search_includes_bash_exec_when_runtime_is_available --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_catalog_exposes_command_cwd_and_timeout_ms --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app canonical_tool_name_maps_bash_exec_provider_name --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app framework_timeout_treats_bash_exec_as_dedicated_timeout_tool --lib -- --exact --nocapture`

Expected: PASS

- [ ] **Step 8: Commit the catalog slice**

Run:

- `git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs`
- `git commit -m "feat(app): register bash.exec in the tool catalog"`

### Task 3: Implement the `bash.exec` executor with shared subprocess helpers

**Files:**
- Create: `crates/app/src/tools/process_exec.rs`
- Modify: `crates/app/src/tools/bash.rs`
- Modify: `crates/app/src/tools/shell.rs`
- Modify: `crates/app/src/tools/mod.rs`

- [ ] **Step 1: Write the failing validation and unavailable-runtime tests**

Add these tests to `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_rejects_blank_command() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "  "}),
        },
        &config,
    )
    .expect_err("blank commands should be rejected");

    assert!(error.contains("bash.exec requires payload.command"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_returns_runtime_unavailable_error_when_no_bash_is_configured() {
    let config = runtime_config::ToolRuntimeConfig::default();
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "pwd"}),
        },
        &config,
    )
    .expect_err("unavailable runtime should fail closed");

    assert!(error.contains("bash unavailable"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_reports_failed_status_for_non_zero_exit() {
    let mut config = runtime_config::ToolRuntimeConfig::default();
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf 'oops'; exit 7"}),
        },
        &config,
    )
    .expect("non-zero exit should still return a tool outcome");

    assert_eq!(outcome.status, "failed");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("oops"));
    assert_eq!(outcome.payload["exit_code"].as_i64(), Some(7));
}
```

- [ ] **Step 2: Write the failing Unix execution tests**

Add these Unix-only tests to `crates/app/src/tools/mod.rs`:

```rust
#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_runs_command_string_via_bash_runtime() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf 'hello'"}),
        },
        &config,
    )
    .expect("bash command should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("hello"));
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_honors_cwd() {
    let root = unique_temp_dir("loongclaw-bash-exec-cwd");
    let subdir = root.join("subdir");
    std::fs::create_dir_all(&subdir).expect("create subdir");

    let mut config = test_tool_runtime_config(root.clone());
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": subdir.display().to_string(),
            }),
        },
        &config,
    )
    .expect("bash command should succeed");

    let expected = subdir.display().to_string();
    assert_eq!(outcome.payload["stdout"].as_str(), Some(expected.as_str()));
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_times_out_when_timeout_ms_is_small() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(std::path::PathBuf::from("bash")),
        warning: None,
    };

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "sleep 10",
                "timeout_ms": 1,
            }),
        },
        &config,
    )
    .expect_err("slow bash command should time out");

    assert!(error.contains("timed out after"));
}

#[test]
fn bash_exec_runtime_policy_defaults_to_non_login_shell() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    assert!(!config.bash_exec.login_shell);
}
```

- [ ] **Step 3: Run the red tests**

Run:

- `cargo test -p loongclaw-app bash_exec_rejects_blank_command --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_returns_runtime_unavailable_error_when_no_bash_is_configured --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_reports_failed_status_for_non_zero_exit --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_runs_command_string_via_bash_runtime --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_honors_cwd --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_times_out_when_timeout_ms_is_small --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_runtime_policy_defaults_to_non_login_shell --lib -- --exact --nocapture`

Expected:

- FAIL because `bash.exec` has no executor yet
- FAIL because the shared subprocess helper does not exist yet

- [ ] **Step 4: Add the shared subprocess helper**

Create `crates/app/src/tools/process_exec.rs`:

```rust
use std::{future::Future, path::Path, process::Stdio, thread, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

pub(super) const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub(super) const MAX_TIMEOUT_MS: u64 = 600_000;
const OUTPUT_CAP_BYTES: usize = 1_048_576;

pub(super) fn run_tool_async<F>(future: F) -> Result<F::Output, String>
where
    F: Future + Send,
    F::Output: Send,
{
    /* move the current run_shell_async implementation here */
}

pub(super) async fn run_process_with_timeout(
    program: &std::path::Path,
    args: &[String],
    cwd: &Path,
    timeout_ms: u64,
) -> Result<std::process::Output, String> {
    /* move the current capped stdout/stderr + timeout logic here */
}
```

Update `crates/app/src/tools/shell.rs` to call the shared helper and keep:

- shell payload validation
- shell allow/deny/default behavior
- existing `shell.exec` payload/result shape

- [ ] **Step 5: Implement `bash.exec`**

In `crates/app/src/tools/bash.rs`, add:

```rust
pub(super) fn execute_bash_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "bash.exec payload must be an object".to_owned())?;
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "bash.exec requires payload.command".to_owned())?;
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let timeout_ms = parse_bash_timeout_ms(payload)?;

    let runtime = &config.bash_exec;
    let bash_path = runtime
        .command
        .as_ref()
        .filter(|_| runtime.is_runtime_ready())
        .ok_or_else(|| "bash unavailable: bash.exec is disabled on this runtime".to_owned())?;

    let args = bash_exec_args(command, runtime.login_shell);
    let output = super::process_exec::run_tool_async(
        super::process_exec::run_process_with_timeout(bash_path.as_path(), &args, cwd.as_path(), timeout_ms),
    )??;

    Ok(ToolCoreOutcome {
        status: if output.status.success() { "ok".to_owned() } else { "failed".to_owned() },
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "command": command,
            "cwd": cwd.display().to_string(),
            "exit_code": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            "stderr": String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }),
    })
}
```

Keep `parse_bash_timeout_ms(...)` aligned with the shared timeout constants:

```rust
let timeout_ms = payload
    .get("timeout_ms")
    .and_then(Value::as_u64)
    .unwrap_or(super::process_exec::DEFAULT_TIMEOUT_MS);
Ok(timeout_ms.clamp(1_000, super::process_exec::MAX_TIMEOUT_MS))
```

This default must match the shared subprocess helper contract: omitting `timeout_ms`
uses `super::process_exec::DEFAULT_TIMEOUT_MS`, not a bash-specific fallback and not
an error.

- [ ] **Step 6: Run the green tests**

Run:

- `cargo test -p loongclaw-app bash_exec_rejects_blank_command --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_returns_runtime_unavailable_error_when_no_bash_is_configured --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_reports_failed_status_for_non_zero_exit --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_runs_command_string_via_bash_runtime --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_honors_cwd --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_times_out_when_timeout_ms_is_small --lib -- --exact --nocapture`

Expected: PASS

- [ ] **Step 7: Commit the executor slice**

Run:

- `git add crates/app/src/tools/process_exec.rs crates/app/src/tools/bash.rs crates/app/src/tools/shell.rs crates/app/src/tools/mod.rs`
- `git commit -m "feat(app): implement bash.exec executor"`

### Task 4: Verify the slice and guard adjacent regressions

**Files:**
- Test: `crates/app/src/tools/bash.rs`
- Test: `crates/app/src/tools/shell.rs`
- Test: `crates/app/src/tools/mod.rs`

- [ ] **Step 1: Run focused `bash.exec` tests**

Run:

- `cargo test -p loongclaw-app bash_exec --lib -- --nocapture`

Expected: PASS

- [ ] **Step 2: Run adjacent `shell.exec` regressions**

Run:

- `cargo test -p loongclaw-app shell_exec_ --lib -- --nocapture`

Expected: PASS

- [ ] **Step 3: Run repository-grade verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`

Expected: PASS

- [ ] **Step 4: Commit the verified slice**

Run:

- `git add crates/app/src/tools/process_exec.rs crates/app/src/tools/bash.rs crates/app/src/tools/shell.rs crates/app/src/tools/runtime_config.rs crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs`
- `git commit -m "feat(app): finish bash.exec basic tool support"`
