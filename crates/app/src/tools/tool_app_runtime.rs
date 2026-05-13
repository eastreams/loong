use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use crate::config::ToolConfig;
use crate::session::store::SessionStoreConfig;

use super::{approval, canonical_tool_name, session};

pub fn execute_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_app_tool_dispatch(request, current_session_id, memory_config, tool_config)
}

pub(crate) fn execute_app_tool_with_visibility_checked_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_app_tool_dispatch(request, current_session_id, memory_config, tool_config)
}

fn execute_app_tool_dispatch(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
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
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
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

pub async fn wait_for_task_with_config(
    payload: Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
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
            return Err("app_tool_disabled: task tools are disabled by config".to_owned());
        }
        session::wait_for_task_tool_with_policies(
            payload,
            current_session_id,
            memory_config,
            tool_config,
        )
        .await
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn continue_session_with_runtime<
    R: crate::conversation::ConversationRuntime + ?Sized,
>(
    payload: Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    app_config: &crate::config::LoongConfig,
    runtime: &R,
    binding: crate::conversation::ConversationRuntimeBinding<'_>,
) -> Result<ToolCoreOutcome, String> {
    session::continue_session_with_runtime(
        payload,
        current_session_id,
        memory_config,
        tool_config,
        app_config,
        runtime,
        binding,
    )
    .await
}
