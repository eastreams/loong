use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use loongclaw_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

use crate::KernelContext;
use crate::config::ToolConfig;
use crate::memory::runtime_config::MemoryRuntimeConfig;

mod catalog;
mod claw_import;
pub(crate) mod delegate;
mod external_skills;
mod file;
pub mod file_policy_ext;
mod kernel_adapter;
pub(crate) mod messaging;
pub mod runtime_config;
mod session;
mod shell;
pub mod shell_policy_ext;

pub use catalog::{
    ToolAvailability, ToolCatalog, ToolDescriptor, ToolExecutionKind, ToolView,
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    planned_delegate_child_tool_view, planned_root_tool_view, runtime_tool_view,
    runtime_tool_view_for_config, tool_catalog,
};
pub use kernel_adapter::MvpToolAdapter;

/// Execute a tool request, routing through the kernel for
/// policy enforcement and audit recording.
///
/// All requests are dispatched via `kernel.execute_tool_core` which
/// enforces `InvokeTool` capability, runs policy extensions, and records
/// audit events.  Callers must always supply a `KernelContext`.
pub async fn execute_tool(
    request: ToolCoreRequest,
    kernel_ctx: &KernelContext,
) -> Result<ToolCoreOutcome, String> {
    let caps = BTreeSet::from([Capability::InvokeTool]);
    kernel_ctx
        .kernel
        .execute_tool_core(
            kernel_ctx.pack_id(),
            &kernel_ctx.token,
            &caps,
            None,
            request,
        )
        .await
        .map_err(|e| format!("{e}"))
}

pub fn execute_tool_core(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    execute_tool_core_with_config(request, runtime_config::get_tool_runtime_config())
}

pub fn execute_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };

    match canonical_name {
        "sessions_list" | "sessions_history" | "session_status" | "session_events"
        | "session_archive" | "session_cancel" | "session_recover" => {
            session::execute_session_tool_with_policies(
                request,
                current_session_id,
                memory_config,
                tool_config,
            )
        }
        _ => Err(format!(
            "app_tool_not_found: unknown app tool `{}`",
            request.tool_name
        )),
    }
}

pub async fn wait_for_session_with_config(
    payload: Value,
    current_session_id: &str,
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (payload, current_session_id, memory_config, tool_config);
        return Err(
            "session tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.sessions.enabled {
            return Err("app_tool_disabled: session tools are disabled by config".to_owned());
        }
        session::wait_for_session_tool_with_policies(
            payload,
            current_session_id,
            memory_config,
            tool_config,
        )
        .await
    }
}

/// Normalize a path by resolving `.` and `..` components without filesystem access.
///
/// - `Prefix` and `RootDir` are tracked separately so `..` can never "eat" them.
/// - `..` past the filesystem root (or volume root on Windows) is silently dropped.
/// - Relative paths preserve leading `..` components (e.g. `../../foo` stays as-is).
///
/// All three path-handling modules (`file`, `claw_import`, `file_policy_ext`) use
/// this single implementation to avoid divergence.
pub(super) fn normalize_without_fs(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut parts: Vec<OsString> = Vec::new();
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_owned()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = parts.last() {
                    if last != ".." {
                        let _ = parts.pop();
                    } else if !has_root {
                        parts.push(OsString::from(".."));
                    }
                } else if !has_root {
                    parts.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => parts.push(value.to_owned()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for part in parts {
        normalized.push(part);
    }
    if normalized.as_os_str().is_empty() {
        if has_root {
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}

pub fn canonical_tool_name(raw: &str) -> &str {
    let catalog = tool_catalog();
    match catalog.resolve(raw) {
        Some(descriptor) => descriptor.name,
        None => raw,
    }
}

pub fn is_known_tool_name(raw: &str) -> bool {
    is_known_tool_name_in_view(raw, &runtime_tool_view())
}

pub fn is_known_tool_name_in_view(raw: &str, view: &ToolView) -> bool {
    view.contains(canonical_tool_name(raw))
}

pub fn execute_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };
    match canonical_name {
        "claw.import" => claw_import::execute_claw_import_tool_with_config(request, config),
        "external_skills.inspect" => {
            external_skills::execute_external_skills_inspect_tool_with_config(request, config)
        }
        "external_skills.install" => {
            external_skills::execute_external_skills_install_tool_with_config(request, config)
        }
        "external_skills.invoke" => {
            external_skills::execute_external_skills_invoke_tool_with_config(request, config)
        }
        "external_skills.list" => {
            external_skills::execute_external_skills_list_tool_with_config(request, config)
        }
        "external_skills.policy" => {
            external_skills::execute_external_skills_policy_tool_with_config(request, config)
        }
        "external_skills.fetch" => {
            external_skills::execute_external_skills_fetch_tool_with_config(request, config)
        }
        "external_skills.remove" => {
            external_skills::execute_external_skills_remove_tool_with_config(request, config)
        }
        "shell.exec" => shell::execute_shell_tool_with_config(request, config),
        "file.read" => file::execute_file_read_tool_with_config(request, config),
        "file.write" => file::execute_file_write_tool_with_config(request, config),
        _ => Err(format!(
            "tool_not_found: unknown tool `{}`",
            request.tool_name
        )),
    }
}

/// Tool registry entry for capability snapshot disclosure.
#[derive(Debug, Clone)]
pub struct ToolRegistryEntry {
    pub name: &'static str,
    pub description: &'static str,
}

/// Returns a sorted list of all registered tools, gated by feature flags.
pub fn tool_registry() -> Vec<ToolRegistryEntry> {
    let catalog = tool_catalog();
    runtime_tool_view()
        .iter(&catalog)
        .map(|descriptor| ToolRegistryEntry {
            name: descriptor.name,
            description: descriptor.description,
        })
        .collect()
}

/// Produce a deterministic text block listing available tools,
/// suitable for appending to the system prompt.
pub fn capability_snapshot() -> String {
    capability_snapshot_with_config(runtime_config::get_tool_runtime_config())
}

pub fn capability_snapshot_with_config(config: &runtime_config::ToolRuntimeConfig) -> String {
    capability_snapshot_for_view_with_config(&runtime_tool_view(), config)
}

pub fn capability_snapshot_for_view(view: &ToolView) -> String {
    capability_snapshot_for_view_with_config(view, runtime_config::get_tool_runtime_config())
}

pub(crate) fn capability_snapshot_for_view_with_config(
    view: &ToolView,
    config: &runtime_config::ToolRuntimeConfig,
) -> String {
    let catalog = tool_catalog();
    let mut lines = vec!["[available_tools]".to_owned()];
    for descriptor in view.iter(&catalog) {
        lines.push(format!("- {}: {}", descriptor.name, descriptor.description));
    }
    let includes_external_skills = view
        .iter(&catalog)
        .any(|descriptor| descriptor.name.starts_with("external_skills."));
    if includes_external_skills
        && let Ok(skill_lines) = external_skills::installed_skill_snapshot_lines_with_config(config)
        && !skill_lines.is_empty()
    {
        lines.push(String::new());
        lines.push("[available_external_skills]".to_owned());
        lines.extend(skill_lines);
    }
    lines.join("\n")
}

/// Provider request tool schema for function-calling capable models.
///
/// The output shape matches OpenAI-compatible `tools=[{type:function,...}]`.
/// Order is deterministic for stable prompting/tests.
pub fn provider_tool_definitions() -> Vec<Value> {
    let catalog = tool_catalog();
    runtime_tool_view()
        .iter(&catalog)
        .map(|descriptor| {
            debug_assert_eq!(descriptor.availability, ToolAvailability::Runtime);
            descriptor.provider_definition()
        })
        .collect()
}

pub fn try_provider_tool_definitions_for_view(view: &ToolView) -> Result<Vec<Value>, String> {
    let catalog = tool_catalog();
    let mut tools = Vec::new();
    for descriptor in view.iter(&catalog) {
        if descriptor.availability != ToolAvailability::Runtime {
            return Err(format!(
                "tool_not_advertisable: `{}` is still planned and cannot be exposed yet",
                descriptor.name
            ));
        }
        tools.push(descriptor.provider_definition());
    }
    Ok(tools)
}

#[allow(dead_code)]
fn _shape_examples() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        (
            "claw.import",
            json!({
                "input_path": "/tmp/nanobot-workspace",
                "mode": "plan",
                "source": "auto"
            }),
        ),
        (
            "shell.exec",
            json!({
                "command": "echo",
                "args": ["hello"]
            }),
        ),
        (
            "external_skills.policy",
            json!({
                "action": "set",
                "policy_update_approved": true,
                "enabled": true,
                "require_download_approval": true,
                "allowed_domains": ["skills.sh"],
                "blocked_domains": ["*.evil.example"]
            }),
        ),
        (
            "external_skills.fetch",
            json!({
                "url": "https://skills.sh/packages/demo-skill.tar.gz",
                "approval_granted": true
            }),
        ),
        (
            "file.read",
            json!({
                "path": "README.md",
                "max_bytes": 4096
            }),
        ),
        (
            "file.write",
            json!({
                "path": "notes.txt",
                "content": "hello",
                "create_dirs": true
            }),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_snapshot_is_deterministic() {
        let snapshot = capability_snapshot();
        assert!(snapshot.starts_with("[available_tools]"));

        // Verify determinism: two calls produce identical output.
        let snapshot2 = capability_snapshot();
        assert_eq!(snapshot, snapshot2);
    }

    #[test]
    fn capability_snapshot_can_include_installed_external_skills() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-capability-snapshot-skills");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "skills/demo-skill/SKILL.md",
            "# Demo Skill\n\nUse this skill for explicit verification.\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            external_skills: runtime_config::ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: true,
            },
            ..runtime_config::ToolRuntimeConfig::default()
        };
        execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "skills/demo-skill"
                }),
            },
            &config,
        )
        .expect("install should succeed");

        let snapshot = capability_snapshot_with_config(&config);
        assert!(snapshot.contains("[available_external_skills]"));
        assert!(snapshot.contains(
            "- demo-skill: installed managed external skill; use external_skills.inspect or external_skills.invoke for details"
        ));

        fs::remove_dir_all(&root).ok();
    }

    #[cfg(all(
        feature = "tool-file",
        feature = "tool-shell",
        feature = "memory-sqlite"
    ))]
    #[test]
    fn capability_snapshot_lists_all_tools_when_all_features_enabled() {
        let snapshot = capability_snapshot();
        assert!(
            snapshot.contains(
                "- claw.import: Import legacy Claw configs into native LoongClaw settings"
            )
        );
        assert!(snapshot.contains("- delegate: Delegate a focused subtask into a child session"));
        assert!(snapshot.contains(
            "- delegate_async: Delegate a focused subtask into a background child session"
        ));
        assert!(snapshot.contains("- external_skills.fetch: Download external skills artifacts with domain policy and approval guards"));
        assert!(snapshot.contains("- external_skills.install: Install a managed external skill from a local directory or archive"));
        assert!(
            snapshot.contains(
                "- external_skills.inspect: Read metadata for an installed external skill"
            )
        );
        assert!(snapshot.contains(
            "- external_skills.invoke: Load an installed external skill into the conversation loop"
        ));
        assert!(snapshot.contains(
            "- external_skills.list: List managed external skills available for invocation"
        ));
        assert!(snapshot.contains("- external_skills.policy: Read/update external skills domain allow/block policy at runtime"));
        assert!(snapshot.contains(
            "- external_skills.remove: Remove an installed external skill from the managed runtime"
        ));
        assert!(snapshot.contains("- file.read: Read file contents"));
        assert!(snapshot.contains("- file.write: Write file contents"));
        assert!(snapshot.contains(
            "- session_archive: Archive a visible terminal session from default session listings"
        ));
        assert!(
            snapshot.contains("- session_cancel: Cancel a visible async delegate child session")
        );
        assert!(snapshot.contains("- session_events: Fetch session events for a visible session"));
        assert!(snapshot.contains(
            "- session_recover: Recover an overdue queued async delegate child session by marking it failed"
        ));
        assert!(
            snapshot.contains("- session_status: Inspect the current status of a visible session")
        );
        assert!(
            snapshot
                .contains("- session_wait: Wait for a visible session to reach a terminal state")
        );
        assert!(
            snapshot.contains("- sessions_history: Fetch transcript history for a visible session")
        );
        assert!(
            snapshot.contains("- sessions_list: List visible sessions and their high-level state")
        );
        assert!(snapshot.contains("- shell.exec: Execute shell commands"));

        // Verify sorted order: claw.import < delegate* < external_skills.* < file.* < session_* < sessions_* < shell.exec
        let lines: Vec<&str> = snapshot.lines().skip(1).collect();
        assert_eq!(lines.len(), 21);
        assert!(lines[0].starts_with("- claw.import"));
        assert!(lines[1].starts_with("- delegate"));
        assert!(lines[2].starts_with("- delegate_async"));
        assert!(lines[3].starts_with("- external_skills.fetch"));
        assert!(lines[4].starts_with("- external_skills.inspect"));
        assert!(lines[5].starts_with("- external_skills.install"));
        assert!(lines[6].starts_with("- external_skills.invoke"));
        assert!(lines[7].starts_with("- external_skills.list"));
        assert!(lines[8].starts_with("- external_skills.policy"));
        assert!(lines[9].starts_with("- external_skills.remove"));
        assert!(lines[10].starts_with("- file.read"));
        assert!(lines[11].starts_with("- file.write"));
        assert!(lines[12].starts_with("- session_archive"));
        assert!(lines[13].starts_with("- session_cancel"));
        assert!(lines[14].starts_with("- session_events"));
        assert!(lines[15].starts_with("- session_recover"));
        assert!(lines[16].starts_with("- session_status"));
        assert!(lines[17].starts_with("- session_wait"));
        assert!(lines[18].starts_with("- sessions_history"));
        assert!(lines[19].starts_with("- sessions_list"));
        assert!(lines[20].starts_with("- shell.exec"));
    }

    #[cfg(all(
        feature = "tool-file",
        feature = "tool-shell",
        feature = "memory-sqlite"
    ))]
    #[test]
    fn tool_registry_returns_all_known_tools() {
        let entries = tool_registry();
        assert_eq!(entries.len(), 21);
        let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
        assert!(names.contains(&"claw.import"));
        assert!(names.contains(&"delegate"));
        assert!(names.contains(&"delegate_async"));
        assert!(names.contains(&"external_skills.fetch"));
        assert!(names.contains(&"external_skills.install"));
        assert!(names.contains(&"external_skills.inspect"));
        assert!(names.contains(&"external_skills.invoke"));
        assert!(names.contains(&"external_skills.list"));
        assert!(names.contains(&"external_skills.policy"));
        assert!(names.contains(&"external_skills.remove"));
        assert!(names.contains(&"shell.exec"));
        assert!(names.contains(&"file.read"));
        assert!(names.contains(&"file.write"));
        assert!(names.contains(&"session_archive"));
        assert!(names.contains(&"session_cancel"));
        assert!(names.contains(&"session_events"));
        assert!(names.contains(&"session_recover"));
        assert!(names.contains(&"session_status"));
        assert!(names.contains(&"session_wait"));
        assert!(names.contains(&"sessions_history"));
        assert!(names.contains(&"sessions_list"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn capability_snapshot_for_view_only_lists_selected_tools() {
        let view = ToolView::from_tool_names(["claw.import", "shell.exec"]);
        let snapshot = capability_snapshot_for_view(&view);

        assert!(snapshot.contains("- claw.import:"));
        assert!(snapshot.contains("- shell.exec:"));
        assert!(!snapshot.contains("- file.read:"));
        assert!(!snapshot.contains("- external_skills.list:"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn try_provider_tool_definitions_for_view_returns_sorted_subset() {
        let view = ToolView::from_tool_names(["shell.exec", "claw.import"]);
        let defs = try_provider_tool_definitions_for_view(&view)
            .expect("restricted runtime view should be advertisable");
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();

        assert_eq!(names, vec!["claw_import", "shell_exec"]);
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn planned_root_tool_view_is_advertisable_when_all_tools_are_runtime_ready() {
        let defs = try_provider_tool_definitions_for_view(&planned_root_tool_view())
            .expect("all tools should now be advertisable");

        assert_eq!(defs.len(), 22);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn runtime_tool_view_includes_runtime_session_tools_but_hides_planned_ones() {
        let view = runtime_tool_view_for_config(&crate::config::ToolConfig::default());

        for tool_name in [
            "delegate",
            "delegate_async",
            "session_archive",
            "session_cancel",
            "session_events",
            "session_recover",
            "session_status",
            "session_wait",
            "sessions_history",
            "sessions_list",
        ] {
            assert!(
                view.contains(tool_name),
                "expected runtime view to include `{tool_name}`"
            );
        }

        let tool_name = "sessions_send";
        assert!(
            !view.contains(tool_name),
            "expected runtime view to keep `{tool_name}` hidden"
        );
    }

    #[test]
    fn runtime_tool_view_exposes_delegate_tools_with_depth_budget_only() {
        let config = crate::config::ToolConfig::default();

        let root_view = runtime_tool_view_for_config(&config);
        assert!(root_view.contains("delegate"));
        assert!(root_view.contains("delegate_async"));

        let child_view = delegate_child_tool_view_for_config(&config);
        assert!(!child_view.contains("delegate"));
        assert!(!child_view.contains("delegate_async"));

        let depth_budgeted_child = delegate_child_tool_view_for_config_with_delegate(&config, true);
        assert!(depth_budgeted_child.contains("delegate"));
        assert!(depth_budgeted_child.contains("delegate_async"));
    }

    #[test]
    fn runtime_tool_view_exposes_sessions_send_only_when_messages_enabled() {
        let default_root_view = runtime_tool_view_for_config(&crate::config::ToolConfig::default());
        assert!(!default_root_view.contains("sessions_send"));

        let mut config = crate::config::ToolConfig::default();
        config.messages.enabled = true;

        let root_view = runtime_tool_view_for_config(&config);
        assert!(root_view.contains("sessions_send"));

        let child_view = delegate_child_tool_view_for_config(&config);
        assert!(!child_view.contains("sessions_send"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn delegate_child_tool_view_hides_shell_by_default() {
        let view = delegate_child_tool_view_for_config(&crate::config::ToolConfig::default());

        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(!view.contains("shell.exec"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn delegate_child_tool_view_can_allow_shell_when_enabled() {
        let mut config = crate::config::ToolConfig::default();
        config.delegate.allow_shell_in_child = true;

        let view = delegate_child_tool_view_for_config(&config);

        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(view.contains("shell.exec"));
    }

    #[cfg(all(
        feature = "tool-file",
        feature = "tool-shell",
        feature = "memory-sqlite"
    ))]
    #[test]
    fn provider_tool_definitions_are_stable_and_complete() {
        let defs = provider_tool_definitions();
        assert_eq!(defs.len(), 21);

        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            names,
            vec![
                "claw_import",
                "delegate",
                "delegate_async",
                "external_skills_fetch",
                "external_skills_inspect",
                "external_skills_install",
                "external_skills_invoke",
                "external_skills_list",
                "external_skills_policy",
                "external_skills_remove",
                "file_read",
                "file_write",
                "session_archive",
                "session_cancel",
                "session_events",
                "session_recover",
                "session_status",
                "session_wait",
                "sessions_history",
                "sessions_list",
                "shell_exec"
            ]
        );

        for item in &defs {
            assert_eq!(item["type"], "function");
            assert_eq!(item["function"]["parameters"]["type"], "object");
        }

        let claw_import = defs
            .iter()
            .find(|item| {
                item.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    == Some("claw_import")
            })
            .expect("claw_import definition should exist");
        let mode_enum: Vec<&str> =
            claw_import["function"]["parameters"]["properties"]["mode"]["enum"]
                .as_array()
                .expect("mode enum should be an array")
                .iter()
                .filter_map(Value::as_str)
                .collect();
        assert_eq!(
            mode_enum,
            vec![
                "plan",
                "apply",
                "discover",
                "plan_many",
                "recommend_primary",
                "merge_profiles",
                "map_external_skills",
                "apply_selected",
                "rollback_last_apply"
            ]
        );
        assert!(
            claw_import["function"]["parameters"]["required"]
                .as_array()
                .expect("required should be an array")
                .is_empty()
        );
    }

    #[test]
    fn provider_tool_definitions_include_delegate_when_enabled() {
        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(
            &crate::config::ToolConfig::default(),
        ))
        .expect("runtime-visible tool schemas");
        let delegate = defs
            .iter()
            .find(|item| item["function"]["name"] == "delegate")
            .expect("delegate definition");
        let properties = delegate["function"]["parameters"]["properties"]
            .as_object()
            .expect("delegate properties");
        assert!(properties.contains_key("task"));
        assert!(properties.contains_key("label"));
        assert!(properties.contains_key("timeout_seconds"));
    }

    #[test]
    fn provider_tool_definitions_include_delegate_async_when_enabled() {
        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(
            &crate::config::ToolConfig::default(),
        ))
        .expect("runtime-visible tool schemas");
        let delegate_async = defs
            .iter()
            .find(|item| item["function"]["name"] == "delegate_async")
            .expect("delegate_async definition");
        let properties = delegate_async["function"]["parameters"]["properties"]
            .as_object()
            .expect("delegate_async properties");
        assert!(properties.contains_key("task"));
        assert!(properties.contains_key("label"));
        assert!(properties.contains_key("timeout_seconds"));
    }

    #[test]
    fn provider_tool_definitions_include_sessions_send_when_enabled() {
        let mut config = crate::config::ToolConfig::default();
        config.messages.enabled = true;

        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(&config))
            .expect("runtime-visible tool schemas");
        let sessions_send = defs
            .iter()
            .find(|item| item["function"]["name"] == "sessions_send")
            .expect("sessions_send definition");
        let properties = sessions_send["function"]["parameters"]["properties"]
            .as_object()
            .expect("sessions_send properties");
        assert!(properties.contains_key("session_id"));
        assert!(properties.contains_key("text"));
    }

    #[test]
    fn canonical_tool_name_maps_known_aliases() {
        assert_eq!(canonical_tool_name("claw_import"), "claw.import");
        assert_eq!(
            canonical_tool_name("external_skills_policy"),
            "external_skills.policy"
        );
        assert_eq!(
            canonical_tool_name("external_skills_fetch"),
            "external_skills.fetch"
        );
        assert_eq!(
            canonical_tool_name("external_skills_install"),
            "external_skills.install"
        );
        assert_eq!(
            canonical_tool_name("external_skills_list"),
            "external_skills.list"
        );
        assert_eq!(
            canonical_tool_name("external_skills_inspect"),
            "external_skills.inspect"
        );
        assert_eq!(
            canonical_tool_name("external_skills_invoke"),
            "external_skills.invoke"
        );
        assert_eq!(
            canonical_tool_name("external_skills_remove"),
            "external_skills.remove"
        );
        assert_eq!(canonical_tool_name("file_read"), "file.read");
        assert_eq!(canonical_tool_name("file_write"), "file.write");
        assert_eq!(canonical_tool_name("shell_exec"), "shell.exec");
        assert_eq!(canonical_tool_name("shell"), "shell.exec");
        assert_eq!(canonical_tool_name("file.read"), "file.read");
    }

    #[test]
    fn is_known_tool_name_accepts_canonical_and_alias_forms() {
        assert!(is_known_tool_name("claw.import"));
        assert!(is_known_tool_name("claw_import"));
        assert!(is_known_tool_name("external_skills.policy"));
        assert!(is_known_tool_name("external_skills_policy"));
        assert!(is_known_tool_name("external_skills.fetch"));
        assert!(is_known_tool_name("external_skills_fetch"));
        assert!(is_known_tool_name("external_skills.install"));
        assert!(is_known_tool_name("external_skills_install"));
        assert!(is_known_tool_name("external_skills.list"));
        assert!(is_known_tool_name("external_skills_list"));
        assert!(is_known_tool_name("external_skills.inspect"));
        assert!(is_known_tool_name("external_skills_inspect"));
        assert!(is_known_tool_name("external_skills.invoke"));
        assert!(is_known_tool_name("external_skills_invoke"));
        assert!(is_known_tool_name("external_skills.remove"));
        assert!(is_known_tool_name("external_skills_remove"));
        assert!(is_known_tool_name("file.read"));
        assert!(is_known_tool_name("file_read"));
        assert!(is_known_tool_name("file.write"));
        assert!(is_known_tool_name("file_write"));
        assert!(is_known_tool_name("shell.exec"));
        assert!(is_known_tool_name("shell_exec"));
        assert!(is_known_tool_name("shell"));
        assert!(!is_known_tool_name("nonexistent.tool"));
    }

    #[test]
    fn unknown_tool_returns_hard_error_code() {
        let err = execute_tool_core(ToolCoreRequest {
            tool_name: "unknown".to_owned(),
            payload: json!({"hello":"world"}),
        })
        .expect_err("unknown tool should return an error");
        assert!(
            err.contains("tool_not_found"),
            "error should contain tool_not_found, got: {err}"
        );
    }

    #[test]
    fn claw_import_plan_mode_returns_nativeized_preview() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-plan");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "plan",
                    "source": "nanobot",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import plan should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool_name"], "claw.import");
        assert_eq!(outcome.payload["mode"], "plan");
        assert_eq!(outcome.payload["source"], "nanobot");
        assert_eq!(
            outcome.payload["config_preview"]["prompt_pack_id"],
            "loongclaw-core-v1"
        );
        assert_eq!(
            outcome.payload["config_preview"]["memory_profile"],
            "profile_plus_window"
        );
        assert!(
            outcome.payload["config_preview"]["system_prompt_addendum"]
                .as_str()
                .expect("prompt addendum should exist")
                .contains("LoongClaw")
        );
        assert!(
            outcome.payload["config_preview"]["profile_note"]
                .as_str()
                .expect("profile note should exist")
                .contains("LoongClaw")
        );
        assert_eq!(outcome.payload["config_written"], false);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_mode_writes_target_config() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let output_path = root.join("generated").join("loongclaw.toml");
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw_import".to_owned(),
                payload: json!({
                    "mode": "apply",
                    "source": "nanobot",
                    "input_path": ".",
                    "output_path": "generated/loongclaw.toml",
                    "force": true
                }),
            },
            &config,
        )
        .expect("claw import apply should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply");
        assert_eq!(outcome.payload["config_written"], true);
        assert_eq!(
            outcome.payload["next_step"]
                .as_str()
                .expect("next_step should be present")
                .split_whitespace()
                .next(),
            Some("loongclaw")
        );
        assert_eq!(
            outcome.payload["output_path"]
                .as_str()
                .expect("output path should exist"),
            fs::canonicalize(&output_path)
                .expect("output path should canonicalize")
                .display()
                .to_string()
        );

        let raw = fs::read_to_string(&output_path).expect("output config should exist");
        assert!(raw.contains("prompt_pack_id = \"loongclaw-core-v1\""));
        assert!(raw.contains("profile = \"profile_plus_window\""));
        assert!(raw.contains("LoongClaw"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_discover_mode_returns_detected_sources() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-discover");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "discover",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import discover should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "discover");
        assert_eq!(outcome.payload["sources"][0]["source_id"], "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_plan_many_mode_returns_source_summaries_and_recommendation() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-plan-many");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "plan_many",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import plan_many should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "plan_many");
        assert_eq!(outcome.payload["plans"][0]["source_id"], "openclaw");
        assert_eq!(outcome.payload["recommendation"]["source_id"], "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_merge_profiles_mode_preserves_prompt_owner() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-merge-profiles");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- region: apac\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "merge_profiles",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import merge_profiles should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "merge_profiles");
        assert_eq!(
            outcome.payload["result"]["prompt_owner_source_id"],
            "openclaw"
        );
        assert!(
            outcome.payload["result"]["merged_profile_note"]
                .as_str()
                .expect("merged profile note should be present")
                .contains("region: apac")
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_map_external_skills_mode_returns_mapping_plan() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-map-external-skills");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
        fs::create_dir_all(root.join(".codex/skills")).expect("create codex skills dir");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "map_external_skills",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import map_external_skills should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "map_external_skills");
        assert_eq!(outcome.payload["result"]["artifact_count"], 2);
        assert_eq!(
            outcome.payload["result"]["declared_skills"][0],
            "custom/skill-a"
        );
        assert_eq!(
            outcome.payload["result"]["resolved_skills"][0],
            "custom/skill-a"
        );
        assert!(
            outcome.payload["result"]["profile_note_addendum"]
                .as_str()
                .expect("profile note addendum should exist")
                .contains("Imported External Skills Artifacts")
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_selected_mode_writes_manifest_and_backup() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply-selected");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let output_path = root.join("loongclaw.toml");
        let original_body = crate::config::render(&crate::config::LoongClawConfig::default())
            .expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "loongclaw.toml",
                    "source_id": "openclaw"
                }),
            },
            &config,
        )
        .expect("claw import apply_selected should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply_selected");
        assert!(
            Path::new(
                outcome.payload["result"]["backup_path"]
                    .as_str()
                    .expect("backup path should be present")
            )
            .exists()
        );
        assert!(
            Path::new(
                outcome.payload["result"]["manifest_path"]
                    .as_str()
                    .expect("manifest path should be present")
            )
            .exists()
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_selected_mode_can_apply_external_skills_plan() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply-selected-external");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");

        let output_path = root.join("loongclaw.toml");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "loongclaw.toml",
                    "source_id": "openclaw",
                    "apply_external_skills_plan": true
                }),
            },
            &config,
        )
        .expect("claw import apply_selected with external skills should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["result"]["external_skill_artifact_count"],
            1
        );
        assert_eq!(
            outcome.payload["result"]["external_skill_entries_applied"],
            3
        );
        assert!(
            outcome.payload["result"]["external_skills_manifest_path"]
                .as_str()
                .is_some(),
            "external skills manifest path should exist"
        );
        let raw = fs::read_to_string(&output_path).expect("read output config");
        assert!(raw.contains("Imported External Skills Artifacts"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_rollback_last_apply_restores_original_config() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-rollback-selected");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let output_path = root.join("loongclaw.toml");
        let original_body = crate::config::render(&crate::config::LoongClawConfig::default())
            .expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "loongclaw.toml",
                    "source_id": "openclaw"
                }),
            },
            &config,
        )
        .expect("claw import apply_selected should succeed");

        let rollback = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "rollback_last_apply",
                    "output_path": "loongclaw.toml"
                }),
            },
            &config,
        )
        .expect("claw import rollback_last_apply should succeed");

        assert_eq!(rollback.status, "ok");
        assert!(
            rollback.payload["rolled_back"]
                .as_bool()
                .expect("rolled_back flag should exist")
        );
        assert_eq!(
            fs::read_to_string(&output_path).expect("read restored config"),
            original_body
        );

        fs::remove_dir_all(&root).ok();
    }

    // --- Kernel-routed tool tests ---

    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use loongclaw_contracts::{ExecutionRoute, HarnessKind, ToolPlaneError};
    use loongclaw_kernel::{
        CoreToolAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine,
        VerticalPackManifest,
    };

    struct SharedTestToolAdapter {
        invocations: Arc<Mutex<Vec<ToolCoreRequest>>>,
    }

    #[async_trait]
    impl CoreToolAdapter for SharedTestToolAdapter {
        fn name(&self) -> &str {
            "test-tool-shared"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    fn build_tool_kernel_context(
        audit: Arc<InMemoryAuditSink>,
        capabilities: BTreeSet<Capability>,
    ) -> (KernelContext, Arc<Mutex<Vec<ToolCoreRequest>>>) {
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: capabilities,
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");

        let invocations = Arc::new(Mutex::new(Vec::new()));
        let adapter = SharedTestToolAdapter {
            invocations: invocations.clone(),
        };
        kernel.register_core_tool_adapter(adapter);
        kernel
            .set_default_core_tool_adapter("test-tool-shared")
            .expect("set default tool adapter");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let ctx = KernelContext {
            kernel: Arc::new(kernel),
            token,
        };

        (ctx, invocations)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_call_through_kernel_records_audit() {
        let audit = Arc::new(InMemoryAuditSink::default());
        let (ctx, invocations) =
            build_tool_kernel_context(audit.clone(), BTreeSet::from([Capability::InvokeTool]));

        let request = ToolCoreRequest {
            tool_name: "echo".to_owned(),
            payload: json!({"msg": "hello"}),
        };
        let outcome = execute_tool(request, &ctx)
            .await
            .expect("tool call via kernel should succeed");
        assert_eq!(outcome.status, "ok");

        // Verify the tool adapter received the request.
        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].tool_name, "echo");

        // Verify audit events contain a tool plane invocation.
        let events = audit.snapshot();
        let has_tool_plane = events.iter().any(|event| {
            matches!(
                &event.kind,
                loongclaw_kernel::AuditEventKind::PlaneInvoked {
                    plane: loongclaw_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });
        assert!(has_tool_plane, "audit should contain tool plane invocation");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mvp_tool_adapter_routes_through_kernel() {
        use kernel_adapter::MvpToolAdapter;

        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel =
            LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");
        kernel.register_core_tool_adapter(MvpToolAdapter::new());
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let caps = BTreeSet::from([Capability::InvokeTool]);
        // Use an unknown tool name — it should propagate as an error through the adapter
        let request = ToolCoreRequest {
            tool_name: "noop".to_owned(),
            payload: json!({"key": "value"}),
        };
        let err = kernel
            .execute_tool_core("test-pack", &token, &caps, None, request)
            .await
            .expect_err("unknown tool via MvpToolAdapter should fail");
        assert!(
            format!("{err}").contains("tool_not_found"),
            "error should contain tool_not_found, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_call_through_kernel_denied_without_capability() {
        let audit = Arc::new(InMemoryAuditSink::default());
        // Grant MemoryRead only — InvokeTool is missing.
        let (ctx, _invocations) =
            build_tool_kernel_context(audit, BTreeSet::from([Capability::MemoryRead]));

        let request = ToolCoreRequest {
            tool_name: "echo".to_owned(),
            payload: json!({"msg": "hello"}),
        };
        let err = execute_tool(request, &ctx)
            .await
            .expect_err("should be denied without InvokeTool capability");

        // The error message should indicate a policy/capability denial.
        assert!(
            err.contains("denied") || err.contains("capability") || err.contains("Capability"),
            "error should mention denial or capability, got: {err}"
        );
    }
}
