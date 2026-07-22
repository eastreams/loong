use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use crate::Context;
use crate::config::ToolConfig;
use crate::session::store::SessionStoreConfig;

use super::{approval, canonical_tool_name, session};

/// Execute a legacy app-owned tool for context-free CLI/spec callers.
///
/// This path has no live Session authority, so its visibility ceiling is the
/// static legacy catalog projected from config. Runtime execution must call
/// [`execute_legacy_app_tool_in_view`] with the current Session's ToolView.
pub fn execute_legacy_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let authority_tool_view = super::runtime_tool_view_for_config(tool_config);
    execute_legacy_app_tool_in_view(
        request,
        current_session_id,
        memory_config,
        tool_config,
        &authority_tool_view,
    )
}

/// Dispatch after the runtime owner has projected the current Session ToolView.
///
/// Keeping the view explicit prevents this legacy leaf from reconstructing
/// authority from static catalog metadata.
pub(crate) fn execute_legacy_app_tool_in_view(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    authority_tool_view: &super::ToolView,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    if let Some(descriptor) = super::tool_catalog().descriptor(canonical_name)
        && descriptor.owner == super::ToolOwner::LegacyApp
        && !authority_tool_view.contains(descriptor.name)
    {
        return Err(format!("tool_not_visible: {}", descriptor.name));
    }
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };

    match canonical_name {
        "approval_requests_list" | "approval_request_status" | "approval_request_resolve" => {
            approval::execute_approval_tool_with_policies(
                request,
                current_session_id,
                memory_config,
                tool_config,
            )
        }
        "sessions_list"
        | "tasks_list"
        | "sessions_history"
        | "task_history"
        | "task_events"
        | "task_cancel"
        | "task_recover"
        | "session_heads"
        | "session_path"
        | "session_children"
        | "session_artifacts"
        | "session_tool_policy_status"
        | "session_tool_policy_set"
        | "session_tool_policy_clear"
        | "session_status"
        | "task_status"
        | "session_events"
        | "session_search"
        | "session_archive"
        | "session_cancel"
        | "session_create_checkpoint"
        | "session_create_branch_summary"
        | "session_continue"
        | "session_fork_head"
        | "session_pin_head"
        | "session_set_active_head"
        | "session_unpin_head"
        | "session_recover" => session::execute_session_tool_with_policies(
            request,
            current_session_id,
            memory_config,
            tool_config,
        ),
        _ => Err(format!(
            "app_tool_not_found: unknown app tool `{}`",
            request.tool_name
        )),
    }
}

pub async fn wait_for_session_with_config(
    payload: Value,
    context: &Context<'_>,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (payload, context, memory_config, tool_config);
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
        session::wait_for_session_tool_with_policies(payload, context, memory_config, tool_config)
            .await
    }
}

pub async fn wait_for_task_with_config(
    payload: Value,
    context: &Context<'_>,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (payload, context, memory_config, tool_config);
        return Err(
            "session tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.sessions.enabled {
            return Err("app_tool_disabled: task tools are disabled by config".to_owned());
        }
        session::wait_for_task_tool_with_policies(payload, context, memory_config, tool_config)
            .await
    }
}
