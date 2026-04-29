use crate::CliResult;
use crate::acp::resolve_acp_backend_selection;
use crate::conversation::collect_context_engine_runtime_snapshot;
use crate::conversation::resolve_context_engine_selection;

use super::CliChatOptions;
use super::CliTurnRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatStartupSummary {
    pub(super) config_path: String,
    pub(super) memory_label: String,
    pub(super) session_id: String,
    pub(super) context_engine_id: String,
    pub(super) context_engine_source: String,
    pub(super) compaction_enabled: bool,
    pub(super) compaction_min_messages: Option<usize>,
    pub(super) compaction_trigger_estimated_tokens: Option<usize>,
    pub(super) compaction_preserve_recent_turns: usize,
    pub(super) compaction_preserve_recent_estimated_tokens: Option<usize>,
    pub(super) compaction_fail_open: bool,
    pub(super) acp_enabled: bool,
    pub(super) dispatch_enabled: bool,
    pub(super) conversation_routing: String,
    pub(super) allowed_channels: Vec<String>,
    pub(super) acp_backend_id: String,
    pub(super) acp_backend_source: String,
    pub(super) explicit_acp_request: bool,
    pub(super) event_stream_enabled: bool,
    pub(super) bootstrap_mcp_servers: Vec<String>,
    pub(super) working_directory: Option<String>,
}

pub(super) fn build_cli_chat_startup_summary(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<CliChatStartupSummary> {
    let context_engine_selection = resolve_context_engine_selection(&runtime.config);
    let context_engine_runtime = collect_context_engine_runtime_snapshot(&runtime.config)?;
    let compaction = context_engine_runtime.compaction;
    let acp_selection = resolve_acp_backend_selection(&runtime.config);
    Ok(CliChatStartupSummary {
        config_path: runtime.resolved_path.display().to_string(),
        memory_label: runtime.memory_label.clone(),
        session_id: runtime.session_id.clone(),
        context_engine_id: context_engine_selection.id.to_owned(),
        context_engine_source: context_engine_selection.source.as_str().to_owned(),
        compaction_enabled: compaction.enabled,
        compaction_min_messages: compaction.min_messages,
        compaction_trigger_estimated_tokens: compaction.trigger_estimated_tokens,
        compaction_preserve_recent_turns: runtime
            .config
            .conversation
            .compact_preserve_recent_turns(),
        compaction_preserve_recent_estimated_tokens: runtime
            .config
            .conversation
            .compact_preserve_recent_estimated_tokens(),
        compaction_fail_open: compaction.fail_open,
        acp_enabled: runtime.config.acp.enabled,
        dispatch_enabled: runtime.config.acp.dispatch_enabled(),
        conversation_routing: runtime
            .config
            .acp
            .dispatch
            .conversation_routing
            .as_str()
            .to_owned(),
        allowed_channels: runtime.config.acp.dispatch.allowed_channel_ids()?,
        acp_backend_id: acp_selection.id.to_owned(),
        acp_backend_source: acp_selection.source.as_str().to_owned(),
        explicit_acp_request: runtime.explicit_acp_request,
        event_stream_enabled: options.acp_event_stream,
        bootstrap_mcp_servers: runtime.effective_bootstrap_mcp_servers.clone(),
        working_directory: runtime
            .effective_working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}
