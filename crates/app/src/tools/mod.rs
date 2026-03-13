use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use loongclaw_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

use crate::config::{LoongClawConfig, ToolConfig};
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::KernelContext;

mod catalog;
pub(crate) mod delegate;
mod file;
mod kernel_adapter;
mod memory;
pub(crate) mod messaging;
pub mod runtime_config;
mod session;
mod shell;

pub use catalog::{
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    planned_delegate_child_tool_view, planned_root_tool_view, runtime_tool_view,
    runtime_tool_view_for_config, tool_catalog, ToolAvailability, ToolCatalog, ToolDescriptor,
    ToolExecutionKind, ToolView,
};
pub use kernel_adapter::MvpToolAdapter;

/// Execute a tool request, optionally routing through the kernel for
/// policy enforcement and audit recording.
///
/// When `kernel_ctx` is `Some`, the request is dispatched via
/// `kernel.execute_tool_core` which enforces `InvokeTool` capability
/// and records audit events.  When `None`, the request falls through
/// to the direct `execute_tool_core` path.
pub async fn execute_tool(
    request: ToolCoreRequest,
    kernel_ctx: Option<&KernelContext>,
) -> Result<ToolCoreOutcome, String> {
    match kernel_ctx {
        Some(ctx) => {
            let caps = BTreeSet::from([Capability::InvokeTool]);
            ctx.kernel
                .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|e| format!("{e}"))
        }
        None => execute_tool_core(request),
    }
}

pub fn execute_tool_core(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    execute_tool_core_with_config(request, runtime_config::get_tool_runtime_config())
}

pub fn execute_app_tool(
    request: ToolCoreRequest,
    current_session_id: &str,
) -> Result<ToolCoreOutcome, String> {
    execute_app_tool_with_config(
        request,
        current_session_id,
        crate::memory::runtime_config::get_memory_runtime_config(),
        &crate::config::ToolConfig::default(),
    )
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
        | "session_cancel" | "session_archive" | "session_recover" | "session_unarchive" => {
            session::execute_session_tool_with_policies(
                request,
                current_session_id,
                memory_config,
                tool_config,
            )
        }
        "memory_search" => memory::execute_memory_search_tool_with_policies(
            request.payload,
            current_session_id,
            memory_config,
            tool_config,
        ),
        "sessions_send" => Err("app_tool_requires_runtime_support: sessions_send".to_owned()),
        "session_wait" => Err("app_tool_requires_async_runtime_support: session_wait".to_owned()),
        "delegate_async" => {
            Err("app_tool_requires_async_runtime_support: delegate_async".to_owned())
        }
        "delegate" => Err("app_tool_requires_turn_loop_dispatch: delegate".to_owned()),
        _ => Err(format!(
            "app_tool_not_found: unknown tool `{}`",
            request.tool_name
        )),
    }
}

#[derive(Clone, Default)]
pub struct AppToolRuntimeSupport<'a> {
    pub app_config: Option<&'a LoongClawConfig>,
    pub async_delegate_spawner: Option<Arc<dyn delegate::AsyncDelegateSpawner>>,
}

pub async fn execute_app_tool_with_runtime_support(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    runtime_support: AppToolRuntimeSupport<'_>,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };
    match canonical_name {
        "sessions_list" | "sessions_history" | "session_status" | "session_events"
        | "session_cancel" | "session_archive" | "session_recover" | "session_unarchive" => {
            session::execute_session_tool_with_policies(
                request,
                current_session_id,
                memory_config,
                tool_config,
            )
        }
        "memory_search" => memory::execute_memory_search_tool_with_policies(
            request.payload,
            current_session_id,
            memory_config,
            tool_config,
        ),
        "session_wait" => {
            wait_for_session_with_config(
                request.payload,
                current_session_id,
                memory_config,
                tool_config,
            )
            .await
        }
        "sessions_send" => {
            let app_config = runtime_support
                .app_config
                .ok_or_else(|| "sessions_send_not_configured".to_owned())?;
            messaging::execute_sessions_send_with_config(
                request.payload,
                current_session_id,
                memory_config,
                tool_config,
                app_config,
            )
            .await
        }
        "delegate_async" => {
            let spawner = runtime_support
                .async_delegate_spawner
                .ok_or_else(|| "delegate_async_not_configured".to_owned())?;
            delegate::execute_delegate_async_with_config(
                request.payload,
                current_session_id,
                memory_config,
                tool_config,
                spawner,
            )
            .await
        }
        "delegate" => Err("app_tool_requires_turn_loop_dispatch: delegate".to_owned()),
        _ => Err(format!(
            "app_tool_not_found: unknown tool `{}`",
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
    capability_snapshot_for_view(&runtime_tool_view())
}

pub fn capability_snapshot_for_view(view: &ToolView) -> String {
    let catalog = tool_catalog();
    let mut lines = vec!["[available_tools]".to_owned()];
    for descriptor in view.iter(&catalog) {
        lines.push(format!("- {}: {}", descriptor.name, descriptor.description));
    }
    lines.join("\n")
}

/// Provider request tool schema for function-calling capable models.
///
/// The output shape matches OpenAI-compatible `tools=[{type:function,...}]`.
/// Order is deterministic for stable prompting/tests.
pub fn provider_tool_definitions() -> Vec<Value> {
    match try_provider_tool_definitions_for_view(&runtime_tool_view()) {
        Ok(definitions) => definitions,
        Err(error) => {
            debug_assert!(
                false,
                "runtime tool view should always be advertisable: {error}"
            );
            Vec::new()
        }
    }
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
            "shell.exec",
            json!({
                "command": "echo",
                "args": ["hello"]
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

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn capability_snapshot_lists_all_tools_when_all_features_enabled() {
        let snapshot = capability_snapshot();
        assert!(snapshot.contains("- delegate: Delegate a focused subtask into a child session"));
        assert!(snapshot.contains(
            "- delegate_async: Delegate a focused subtask into a background child session"
        ));
        assert!(snapshot.contains("- file.read: Read file contents"));
        assert!(snapshot.contains("- file.write: Write file contents"));
        assert!(snapshot.contains(
            "- memory_search: Search visible transcript memory across persisted session turns"
        ));
        assert!(snapshot.contains("- shell.exec: Execute shell commands"));
        assert!(
            snapshot.contains("- sessions_list: List visible sessions and their high-level state")
        );
        assert!(
            snapshot.contains("- sessions_history: Fetch transcript history for a visible session")
        );
        assert!(
            snapshot.contains("- session_status: Inspect the current status of a visible session")
        );
        assert!(snapshot.contains(
            "- session_archive: Archive a visible terminal session from default session listings"
        ));
        assert!(
            snapshot.contains("- session_cancel: Cancel a visible async delegate child session")
        );
        assert!(snapshot.contains(
            "- session_recover: Recover an overdue queued async delegate child session by marking it failed"
        ));
        assert!(snapshot
            .contains("- session_unarchive: Restore a visible archived terminal session to default session listings"));
        assert!(snapshot
            .contains("- session_wait: Wait for a visible session to reach a terminal state"));
        assert!(snapshot.contains("- session_events: Fetch session events for a visible session"));

        // Verify sorted canonical name order.
        let lines: Vec<&str> = snapshot.lines().skip(1).collect();
        assert_eq!(lines.len(), 15);
        assert!(lines[0].starts_with("- delegate"));
        assert!(lines[1].starts_with("- delegate_async"));
        assert!(lines[2].starts_with("- file.read"));
        assert!(lines[3].starts_with("- file.write"));
        assert!(lines[4].starts_with("- memory_search"));
        assert!(lines[5].starts_with("- session_archive"));
        assert!(lines[6].starts_with("- session_cancel"));
        assert!(lines[7].starts_with("- session_events"));
        assert!(lines[8].starts_with("- session_recover"));
        assert!(lines[9].starts_with("- session_status"));
        assert!(lines[10].starts_with("- session_unarchive"));
        assert!(lines[11].starts_with("- session_wait"));
        assert!(lines[12].starts_with("- sessions_history"));
        assert!(lines[13].starts_with("- sessions_list"));
        assert!(lines[14].starts_with("- shell.exec"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn tool_registry_returns_all_known_tools() {
        let entries = tool_registry();
        assert_eq!(entries.len(), 15);
        let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
        assert!(names.contains(&"delegate"));
        assert!(names.contains(&"delegate_async"));
        assert!(names.contains(&"shell.exec"));
        assert!(names.contains(&"file.read"));
        assert!(names.contains(&"file.write"));
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"session_archive"));
        assert!(names.contains(&"session_cancel"));
        assert!(names.contains(&"session_events"));
        assert!(names.contains(&"session_recover"));
        assert!(names.contains(&"session_unarchive"));
        assert!(names.contains(&"sessions_list"));
        assert!(names.contains(&"sessions_history"));
        assert!(names.contains(&"session_status"));
        assert!(names.contains(&"session_wait"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn provider_tool_definitions_are_stable_and_complete() {
        let defs = provider_tool_definitions();
        assert_eq!(defs.len(), 15);

        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            names,
            vec![
                "delegate",
                "delegate_async",
                "file_read",
                "file_write",
                "memory_search",
                "session_archive",
                "session_cancel",
                "session_events",
                "session_recover",
                "session_status",
                "session_unarchive",
                "session_wait",
                "sessions_history",
                "sessions_list",
                "shell_exec",
            ]
        );

        for item in &defs {
            assert_eq!(item["type"], "function");
            assert_eq!(item["function"]["parameters"]["type"], "object");
        }

        let session_wait = defs
            .iter()
            .find(|item| item["function"]["name"] == "session_wait")
            .expect("session_wait definition");
        let properties = session_wait["function"]["parameters"]["properties"]
            .as_object()
            .expect("session_wait properties");
        assert!(properties.contains_key("session_id"));
        assert!(properties.contains_key("session_ids"));
        assert!(properties.contains_key("timeout_ms"));
        assert!(properties.contains_key("after_id"));
        assert_eq!(
            session_wait["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("session_wait oneOf")
                .len(),
            2
        );

        let session_recover = defs
            .iter()
            .find(|item| item["function"]["name"] == "session_recover")
            .expect("session_recover definition");
        let recover_properties = session_recover["function"]["parameters"]["properties"]
            .as_object()
            .expect("session_recover properties");
        assert!(recover_properties.contains_key("session_id"));
        assert!(recover_properties.contains_key("session_ids"));
        assert!(recover_properties.contains_key("dry_run"));
        assert_eq!(
            session_recover["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("session_recover oneOf")
                .len(),
            2
        );

        let session_cancel = defs
            .iter()
            .find(|item| item["function"]["name"] == "session_cancel")
            .expect("session_cancel definition");
        let cancel_properties = session_cancel["function"]["parameters"]["properties"]
            .as_object()
            .expect("session_cancel properties");
        assert!(cancel_properties.contains_key("session_id"));
        assert!(cancel_properties.contains_key("session_ids"));
        assert!(cancel_properties.contains_key("dry_run"));
        assert_eq!(
            session_cancel["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("session_cancel oneOf")
                .len(),
            2
        );

        let session_archive = defs
            .iter()
            .find(|item| item["function"]["name"] == "session_archive")
            .expect("session_archive definition");
        let archive_properties = session_archive["function"]["parameters"]["properties"]
            .as_object()
            .expect("session_archive properties");
        assert!(archive_properties.contains_key("session_id"));
        assert!(archive_properties.contains_key("session_ids"));
        assert!(archive_properties.contains_key("dry_run"));
        assert_eq!(
            session_archive["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("session_archive oneOf")
                .len(),
            2
        );

        let session_unarchive = defs
            .iter()
            .find(|item| item["function"]["name"] == "session_unarchive")
            .expect("session_unarchive definition");
        let unarchive_properties = session_unarchive["function"]["parameters"]["properties"]
            .as_object()
            .expect("session_unarchive properties");
        assert!(unarchive_properties.contains_key("session_id"));
        assert!(unarchive_properties.contains_key("session_ids"));
        assert!(unarchive_properties.contains_key("dry_run"));
        assert_eq!(
            session_unarchive["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("session_unarchive oneOf")
                .len(),
            2
        );

        let session_status = defs
            .iter()
            .find(|item| item["function"]["name"] == "session_status")
            .expect("session_status definition");
        let status_properties = session_status["function"]["parameters"]["properties"]
            .as_object()
            .expect("session_status properties");
        assert!(status_properties.contains_key("session_id"));
        assert!(status_properties.contains_key("session_ids"));
        assert_eq!(
            session_status["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("session_status oneOf")
                .len(),
            2
        );

        let memory_search = defs
            .iter()
            .find(|item| item["function"]["name"] == "memory_search")
            .expect("memory_search definition");
        let memory_search_properties = memory_search["function"]["parameters"]["properties"]
            .as_object()
            .expect("memory_search properties");
        assert!(memory_search_properties.contains_key("query"));
        assert!(memory_search_properties.contains_key("session_id"));
        assert!(memory_search_properties.contains_key("session_ids"));
        assert!(memory_search_properties.contains_key("limit"));
        assert!(memory_search_properties.contains_key("excerpt_chars"));
        assert_eq!(
            memory_search["function"]["parameters"]["oneOf"]
                .as_array()
                .expect("memory_search oneOf")
                .len(),
            3
        );

        let sessions_list = defs
            .iter()
            .find(|item| item["function"]["name"] == "sessions_list")
            .expect("sessions_list definition");
        let list_properties = sessions_list["function"]["parameters"]["properties"]
            .as_object()
            .expect("sessions_list properties");
        assert!(list_properties.contains_key("limit"));
        assert!(list_properties.contains_key("state"));
        assert!(list_properties.contains_key("kind"));
        assert!(list_properties.contains_key("parent_session_id"));
        assert!(list_properties.contains_key("overdue_only"));
        assert!(list_properties.contains_key("include_archived"));
        assert!(list_properties.contains_key("include_delegate_lifecycle"));
    }

    #[test]
    fn canonical_tool_name_maps_known_aliases() {
        assert_eq!(canonical_tool_name("file_read"), "file.read");
        assert_eq!(canonical_tool_name("file_write"), "file.write");
        assert_eq!(canonical_tool_name("shell_exec"), "shell.exec");
        assert_eq!(canonical_tool_name("shell"), "shell.exec");
        assert_eq!(canonical_tool_name("memory_search"), "memory_search");
        assert_eq!(canonical_tool_name("file.read"), "file.read");
    }

    #[test]
    fn is_known_tool_name_accepts_canonical_and_alias_forms() {
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
    fn tool_catalog_marks_core_and_app_tools() {
        let catalog = tool_catalog();
        assert_eq!(
            catalog
                .descriptor("file.read")
                .expect("file.read descriptor")
                .execution_kind,
            ToolExecutionKind::Core
        );
        assert_eq!(
            catalog
                .descriptor("delegate")
                .expect("delegate descriptor")
                .execution_kind,
            ToolExecutionKind::App
        );
        assert_eq!(
            catalog
                .descriptor("delegate_async")
                .expect("delegate_async descriptor")
                .execution_kind,
            ToolExecutionKind::App
        );
        assert_eq!(
            catalog
                .descriptor("delegate")
                .expect("delegate descriptor")
                .availability,
            ToolAvailability::Runtime
        );
        assert_eq!(
            catalog
                .descriptor("delegate_async")
                .expect("delegate_async descriptor")
                .availability,
            ToolAvailability::Runtime
        );
        assert_eq!(
            catalog
                .descriptor("sessions_list")
                .expect("sessions_list descriptor")
                .availability,
            ToolAvailability::Runtime
        );
    }

    #[test]
    fn planned_root_tool_view_contains_first_phase_tools() {
        let view = planned_root_tool_view();
        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        #[cfg(feature = "tool-shell")]
        assert!(view.contains("shell.exec"));
        assert!(view.contains("sessions_list"));
        assert!(view.contains("sessions_history"));
        assert!(view.contains("session_status"));
        assert!(view.contains("session_events"));
        assert!(view.contains("memory_search"));
        assert!(view.contains("session_archive"));
        assert!(view.contains("session_cancel"));
        assert!(view.contains("session_recover"));
        assert!(view.contains("session_wait"));
        assert!(view.contains("delegate"));
        assert!(view.contains("delegate_async"));
    }

    #[test]
    fn runtime_tool_view_includes_delegate_and_session_tools() {
        let view = runtime_tool_view();
        assert!(view.contains("delegate"));
        assert!(view.contains("sessions_list"));
        assert!(view.contains("sessions_history"));
        assert!(view.contains("session_status"));
        assert!(view.contains("session_events"));
        assert!(view.contains("session_archive"));
        assert!(view.contains("session_cancel"));
        assert!(view.contains("session_recover"));
        assert!(view.contains("session_wait"));
        assert!(view.contains("delegate_async"));
    }

    #[test]
    fn session_cancel_is_visible_in_root_and_hidden_in_child_views() {
        let root_view = runtime_tool_view();
        assert!(root_view.contains("session_cancel"));

        let child_view = planned_delegate_child_tool_view();
        assert!(!child_view.contains("session_cancel"));

        let child_with_depth = delegate_child_tool_view_for_config_with_delegate(
            &crate::config::ToolConfig::default(),
            true,
        );
        assert!(!child_with_depth.contains("session_cancel"));
    }

    #[test]
    fn session_archive_is_visible_in_root_and_hidden_in_child_views() {
        let root_view = runtime_tool_view();
        assert!(root_view.contains("session_archive"));

        let child_view = planned_delegate_child_tool_view();
        assert!(!child_view.contains("session_archive"));

        let child_with_depth = delegate_child_tool_view_for_config_with_delegate(
            &crate::config::ToolConfig::default(),
            true,
        );
        assert!(!child_with_depth.contains("session_archive"));
    }

    #[test]
    fn memory_search_is_visible_in_root_and_hidden_in_child_views() {
        let root_view = runtime_tool_view();
        assert!(root_view.contains("memory_search"));

        let child_view = planned_delegate_child_tool_view();
        assert!(!child_view.contains("memory_search"));

        let child_with_depth = delegate_child_tool_view_for_config_with_delegate(
            &crate::config::ToolConfig::default(),
            true,
        );
        assert!(!child_with_depth.contains("memory_search"));
    }

    #[test]
    fn session_unarchive_is_visible_in_root_and_hidden_in_child_views() {
        let root_view = runtime_tool_view();
        assert!(root_view.contains("session_unarchive"));

        let child_view = planned_delegate_child_tool_view();
        assert!(!child_view.contains("session_unarchive"));

        let child_with_depth = delegate_child_tool_view_for_config_with_delegate(
            &crate::config::ToolConfig::default(),
            true,
        );
        assert!(!child_with_depth.contains("session_unarchive"));
    }

    #[test]
    fn runtime_tool_view_for_config_omits_disabled_session_and_delegate_tools() {
        let mut config = crate::config::ToolConfig::default();
        config.sessions.enabled = false;
        config.delegate.enabled = false;

        let view = runtime_tool_view_for_config(&config);
        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(!view.contains("delegate"));
        assert!(!view.contains("delegate_async"));
        assert!(!view.contains("sessions_list"));
        assert!(!view.contains("sessions_history"));
        assert!(!view.contains("session_status"));
        assert!(!view.contains("session_events"));
        assert!(!view.contains("memory_search"));
        assert!(!view.contains("session_archive"));
        assert!(!view.contains("session_recover"));
        assert!(!view.contains("session_unarchive"));
        assert!(!view.contains("session_wait"));
    }

    #[test]
    fn delegate_child_tool_view_for_config_allows_shell_when_enabled() {
        let mut config = crate::config::ToolConfig::default();
        config.delegate.allow_shell_in_child = true;

        let view = delegate_child_tool_view_for_config(&config);
        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(view.contains("shell.exec"));
        assert!(!view.contains("delegate"));
        assert!(!view.contains("sessions_list"));
        assert!(!view.contains("session_recover"));
        assert!(!view.contains("session_wait"));
    }

    #[test]
    fn delegate_child_tool_view_with_remaining_depth_allows_delegate() {
        let config = crate::config::ToolConfig::default();

        let view = delegate_child_tool_view_for_config_with_delegate(&config, true);
        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(view.contains("delegate"));
        assert!(view.contains("delegate_async"));
        assert!(view.contains("sessions_history"));
        assert!(view.contains("session_status"));
        assert!(!view.contains("sessions_list"));
    }

    #[test]
    fn delegate_child_tool_view_default_allowlist_matches_runtime_child_tools() {
        let config = crate::config::ToolConfig::default();
        assert_eq!(
            config.delegate.child_tool_allowlist,
            vec!["file.read", "file.write"]
        );
    }

    #[test]
    fn child_tool_view_excludes_delegate_and_session_list() {
        let view = planned_delegate_child_tool_view();
        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(view.contains("sessions_history"));
        assert!(view.contains("session_status"));
        assert!(!view.contains("shell.exec"));
        assert!(!view.contains("delegate"));
        assert!(!view.contains("delegate_async"));
        assert!(!view.contains("sessions_list"));
        assert!(!view.contains("session_events"));
        assert!(!view.contains("session_archive"));
        assert!(!view.contains("session_cancel"));
        assert!(!view.contains("session_recover"));
        assert!(!view.contains("session_wait"));
    }

    #[test]
    fn child_session_self_inspection_tool_view_includes_status_and_history_only() {
        let view = planned_delegate_child_tool_view();
        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(view.contains("sessions_history"));
        assert!(view.contains("session_status"));
        assert!(!view.contains("sessions_list"));
        assert!(!view.contains("session_events"));
        assert!(!view.contains("session_archive"));
        assert!(!view.contains("session_cancel"));
        assert!(!view.contains("session_recover"));
        assert!(!view.contains("session_wait"));
        assert!(!view.contains("delegate_async"));
    }

    #[test]
    fn delegate_async_is_visible_in_root_and_depth_budgeted_child_views() {
        let root_view = runtime_tool_view();
        assert!(root_view.contains("delegate_async"));

        let child_allowed = delegate_child_tool_view_for_config_with_delegate(
            &crate::config::ToolConfig::default(),
            true,
        );
        assert!(child_allowed.contains("delegate_async"));

        let child_exhausted = planned_delegate_child_tool_view();
        assert!(!child_exhausted.contains("delegate_async"));
    }

    #[test]
    fn provider_tool_definitions_follow_tool_view() {
        let view = ToolView::from_tool_names(["file.read"]);
        let defs =
            try_provider_tool_definitions_for_view(&view).expect("runtime-visible tool schemas");
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(names, vec!["file_read"]);
    }

    #[test]
    fn provider_tool_definitions_include_delegate_when_visible() {
        let view = ToolView::from_tool_names(["delegate", "delegate_async", "file.read"]);
        let defs =
            try_provider_tool_definitions_for_view(&view).expect("runtime-visible tool schemas");
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(names, vec!["delegate", "delegate_async", "file_read"]);
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn runtime_tool_view_exposes_sessions_send_only_when_messages_enabled() {
        let raw = r#"
[tools.messages]
enabled = true
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");
        let root_view = runtime_tool_view_for_config(&parsed.tools);
        assert!(root_view.contains("sessions_send"));

        let child_view = delegate_child_tool_view_for_config(&parsed.tools);
        assert!(!child_view.contains("sessions_send"));
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn provider_tool_definitions_include_sessions_send_when_enabled() {
        let raw = r#"
[tools.messages]
enabled = true
"#;
        let parsed =
            toml::from_str::<crate::config::LoongClawConfig>(raw).expect("parse tool config");
        let defs =
            try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(&parsed.tools))
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

    #[cfg(feature = "memory-sqlite")]
    fn isolated_memory_config(
        test_name: &str,
    ) -> crate::memory::runtime_config::MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-tools-mod-{test_name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("create tools test directory");
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(base.join("memory.sqlite3")),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn execute_app_tool_runtime_support_routes_session_wait() {
        let memory_config = isolated_memory_config("runtime-session-wait");
        let repo =
            crate::session::repository::SessionRepository::new(&memory_config).expect("repo");
        repo.create_session(crate::session::repository::NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: crate::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: crate::session::repository::SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(crate::session::repository::NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: crate::session::repository::SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: crate::session::repository::SessionState::Ready,
        })
        .expect("create child");

        let outcome = execute_app_tool_with_runtime_support(
            ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 1
                }),
            },
            "root-session",
            &memory_config,
            &crate::config::ToolConfig::default(),
            AppToolRuntimeSupport::default(),
        )
        .await
        .expect("session_wait outcome");

        assert_eq!(outcome.status, "timeout");
        assert_eq!(outcome.payload["session"]["session_id"], "child-session");
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn execute_app_tool_runtime_support_reports_sessions_send_not_configured() {
        let memory_config = isolated_memory_config("runtime-sessions-send");

        let error = execute_app_tool_with_runtime_support(
            ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:123",
                    "text": "hello"
                }),
            },
            "root-session",
            &memory_config,
            &crate::config::ToolConfig::default(),
            AppToolRuntimeSupport::default(),
        )
        .await
        .expect_err("missing app config should be rejected");

        assert!(
            error.contains("sessions_send_not_configured"),
            "expected sessions_send_not_configured, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn execute_app_tool_runtime_support_rejects_delegate_without_turn_loop() {
        let memory_config = isolated_memory_config("runtime-delegate");

        let error = execute_app_tool_with_runtime_support(
            ToolCoreRequest {
                tool_name: "delegate".to_owned(),
                payload: json!({
                    "task": "research the runtime"
                }),
            },
            "root-session",
            &memory_config,
            &crate::config::ToolConfig::default(),
            AppToolRuntimeSupport::default(),
        )
        .await
        .expect_err("delegate should require turn-loop dispatch");

        assert!(
            error.contains("app_tool_requires_turn_loop_dispatch: delegate"),
            "expected delegate turn-loop error, got: {error}"
        );
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
        let outcome = execute_tool(request, Some(&ctx))
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
        let err = execute_tool(request, Some(&ctx))
            .await
            .expect_err("should be denied without InvokeTool capability");

        // The error message should indicate a policy/capability denial.
        assert!(
            err.contains("denied") || err.contains("capability") || err.contains("Capability"),
            "error should mention denial or capability, got: {err}"
        );
    }
}
