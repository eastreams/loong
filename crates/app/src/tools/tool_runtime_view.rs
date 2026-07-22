use loong_runtime::runtime::Runtime;

use super::{ToolView, runtime_config, runtime_tool_view_for_runtime_config};
use crate::conversation::ConstrainedSubagentContractView;

pub fn runtime_tool_view_from_loong_config(config: &crate::config::LoongConfig) -> ToolView {
    let runtime_config = runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    runtime_tool_view_for_runtime_config(&runtime_config)
}

pub(crate) fn runtime_visible_tool_view(
    runtime: &Runtime<crate::context::RuntimeContextFactory>,
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
) -> ToolView {
    let mut runtime_view = runtime_tool_view_for_runtime_config(config);
    // Migrated tools no longer have duplicate rows in the legacy catalog.
    // Merge concrete registrations here so provider projection and Session
    // authority share the same runtime-owned source of typed tool identity.
    for path in runtime.registered_tool_paths() {
        // Enumeration is presentation-only. If a future ToolPlane violates its
        // path/entry invariant, omitting that entry fails closed instead of
        // resurrecting legacy metadata or panicking in a host process.
        let Ok((registration, _)) = runtime.tool_metadata(&path) else {
            continue;
        };
        runtime_view.insert_registration(path, registration);
    }

    match visible_tool_view {
        Some(injected) => {
            // Intersect the injected view with the runtime-visible surface so that
            // trusted _loong.tool_search.visible_tool_ids cannot re-expose
            // tools disabled by runtime config (browser.*, session_*, etc.).
            let mut visible = runtime_view.intersect(injected);
            for path in runtime.registered_tool_paths() {
                let Ok((registration, _)) = runtime.tool_metadata(&path) else {
                    continue;
                };
                let provider_name_is_allowed = match registration {
                    loong_runtime::tool_plane::ToolRegistration::Direct { provider_name } => {
                        injected.contains(provider_name)
                    }
                    loong_runtime::tool_plane::ToolRegistration::Discoverable {
                        discovery_name,
                    } => injected.contains(discovery_name),
                };
                if injected.contains_path(&path) || provider_name_is_allowed {
                    visible.insert_registration(path, registration);
                }
            }
            visible
        }
        None => runtime_view,
    }
}

/// Project one delegate contract onto the tools that this Runtime actually owns.
///
/// Fresh and restored children must use this same boundary. The static catalog
/// contributes only unmigrated tools; registered paths contribute migrated
/// tools only when the persisted/derived child allowlist names them.
pub(crate) fn runtime_delegate_child_tool_view(
    runtime: &Runtime<crate::context::RuntimeContextFactory>,
    config: &crate::config::ToolConfig,
    contract: Option<&ConstrainedSubagentContractView>,
) -> ToolView {
    let configured = super::delegate_child_tool_view_for_contract(config, contract);
    let child_allowlist = contract
        .map(|contract| &contract.child_tool_allowlist)
        .unwrap_or(&config.delegate.child_tool_allowlist);
    let mut view = configured;
    for path in runtime.registered_tool_paths() {
        let Ok((registration, _)) = runtime.tool_metadata(&path) else {
            continue;
        };
        let provider_name_is_allowed = match registration {
            loong_runtime::tool_plane::ToolRegistration::Direct { provider_name } => {
                child_allowlist
                    .iter()
                    .any(|allowed| allowed == provider_name)
            }
            loong_runtime::tool_plane::ToolRegistration::Discoverable { discovery_name } => {
                child_allowlist
                    .iter()
                    .any(|allowed| allowed == discovery_name)
            }
        };
        if provider_name_is_allowed {
            view.insert_registration(path, registration);
        }
    }
    view
}

#[cfg(test)]
#[path = "tool_runtime_view/tests.rs"]
mod tests;
