use std::collections::BTreeSet;

use loong_contracts::{Capability, ToolCoreRequest};
use serde_json::Value;

use super::{
    HTTP_REQUEST_TOOL_NAME, ToolExecutionKind, ToolView, config_import, feishu, runtime_tool_view,
    tool_catalog, tool_surface,
};

pub fn canonical_tool_name(raw: &str) -> &str {
    match raw {
        "browse.open" => return "browser.open",
        "browse.extract" => return "browser.extract",
        "browse.click" => return "browser.click",
        _ => {}
    }
    let catalog = tool_catalog();
    if let Some(descriptor) = catalog.resolve(raw) {
        return descriptor.name;
    }
    #[cfg(feature = "feishu-integration")]
    if let Some(canonical) = feishu::canonical_feishu_tool_name(raw) {
        return canonical;
    }
    raw
}

pub(crate) fn required_capabilities_for_request(request: &ToolCoreRequest) -> BTreeSet<Capability> {
    if let Some(peeked_request) = super::peek_tool_invoke_request(request) {
        return required_capabilities_for_tool_name_and_payload(
            peeked_request.tool_name,
            peeked_request.arguments,
        );
    }

    required_capabilities_for_tool_name_and_payload(
        canonical_tool_name(request.tool_name.as_str()),
        &request.payload,
    )
}

pub(crate) fn required_capabilities_for_tool_name_and_payload(
    tool_name: &str,
    payload: &Value,
) -> BTreeSet<Capability> {
    let _ = payload;
    let mut caps = BTreeSet::from([Capability::InvokeTool]);
    let visible_tool_name = user_visible_tool_name(tool_name);
    if tool_requires_network_egress(tool_name) {
        caps.insert(Capability::NetworkEgress);
    }
    match visible_tool_name.as_str() {
        "read" => {
            caps.insert(Capability::FilesystemRead);
        }
        "memory" => {
            caps.insert(Capability::FilesystemRead);
        }
        "write" | "edit" => {
            caps.insert(Capability::FilesystemWrite);
        }
        "sessions_list"
        | "tasks_list"
        | "sessions_history"
        | "session_status"
        | "session_heads"
        | "session_path"
        | "session_children"
        | "session_artifacts"
        | "session_events"
        | "session_wait"
        | "task_status"
        | "task_wait"
        | "task_history"
        | "task_events"
        | "session_search"
        | "session_tool_policy_status" => {
            caps.insert(Capability::MemoryRead);
        }
        "bash" => {
            caps.insert(Capability::FilesystemRead);
            caps.insert(Capability::FilesystemWrite);
            caps.insert(Capability::NetworkEgress);
        }
        _ if tool_name == config_import::CONFIG_IMPORT_TOOL_NAME => {
            caps.insert(Capability::FilesystemRead);
            let mode_requires_write =
                config_import::config_import_mode_requires_write_value(payload);
            if mode_requires_write {
                caps.insert(Capability::FilesystemWrite);
            }
        }
        _ => {}
    }
    caps
}

fn tool_requires_network_egress(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "web"
            | "browser"
            | HTTP_REQUEST_TOOL_NAME
            | "web.fetch"
            | "web.search"
            | "browser.open"
            | "browser.click"
            | "bash"
            | "skills.fetch"
            | "skills.source_search"
    )
}

pub fn is_known_tool_name(raw: &str) -> bool {
    if tool_catalog().resolve(raw).is_some() {
        return true;
    }
    if is_known_tool_name_in_view(raw, &runtime_tool_view()) {
        return true;
    }
    #[cfg(feature = "feishu-integration")]
    {
        feishu::is_known_feishu_tool_name(raw)
    }
    #[cfg(not(feature = "feishu-integration"))]
    {
        false
    }
}

pub fn is_known_tool_name_in_view(raw: &str, view: &ToolView) -> bool {
    let canonical_name = canonical_tool_name(raw);
    is_provider_exposed_tool_name(canonical_name) || view.contains(canonical_name)
}

pub fn is_provider_exposed_tool_name(raw: &str) -> bool {
    super::catalog::find_tool_catalog_entry(canonical_tool_name(raw))
        .is_some_and(|entry| entry.is_provider_exposed())
}

pub(crate) fn direct_tool_name_for_hidden_tool(raw: &str) -> Option<&'static str> {
    let canonical_name = canonical_tool_name(raw);
    if canonical_name == super::SHELL_EXEC_TOOL_NAME {
        return Some("bash");
    }
    tool_surface::direct_tool_name_for_hidden_tool(canonical_name)
}

pub fn user_visible_tool_name(raw: &str) -> String {
    let canonical_name = canonical_tool_name(raw);

    if canonical_name == "tool.search" {
        return "discovery".to_owned();
    }
    if canonical_name == "tool.invoke" {
        return "hidden tool".to_owned();
    }

    if is_tool_surface_id(canonical_name) {
        return canonical_name.to_owned();
    }

    if let Some(direct_tool_name) = direct_tool_name_for_hidden_tool(canonical_name) {
        return direct_tool_name.to_owned();
    }

    canonical_name.to_owned()
}

pub(crate) fn model_visible_tool_name(raw: &str) -> String {
    let canonical_name = canonical_tool_name(raw);
    user_visible_tool_name(canonical_name)
}

pub(crate) fn is_tool_surface_id(surface_id: &str) -> bool {
    tool_surface::is_tool_surface_id(surface_id)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedToolExecution {
    pub canonical_name: &'static str,
    pub execution_kind: ToolExecutionKind,
}

pub(crate) fn resolve_tool_execution(raw: &str) -> Option<ResolvedToolExecution> {
    let catalog = tool_catalog();
    if let Some(descriptor) = catalog.resolve(raw) {
        return Some(ResolvedToolExecution {
            canonical_name: descriptor.name,
            execution_kind: descriptor.execution_kind,
        });
    }
    #[cfg(feature = "feishu-integration")]
    if let Some(canonical_name) = feishu::canonical_feishu_tool_name(raw) {
        return Some(ResolvedToolExecution {
            canonical_name,
            execution_kind: ToolExecutionKind::Core,
        });
    }
    None
}
