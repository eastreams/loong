use crate::tui_surface::TuiCalloutTone;
use crate::tui_surface::TuiMessageSpec;
use crate::tui_surface::TuiSectionSpec;

use super::CLI_CHAT_COMPACT_COMMAND;
use super::render_cli_chat_message_spec_with_width;
use super::startup_state::CliChatStartupSummary;
use super::tui_plain_item;

const STATUS_QUICK_COMMANDS_HINT: &str =
    "Inspect runtime state, then jump back into work with /history · /compact · /help.";

pub(super) fn render_cli_chat_status_lines_with_width(
    summary: &CliChatStartupSummary,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_status_message_spec(summary);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_cli_chat_status_message_spec(summary: &CliChatStartupSummary) -> TuiMessageSpec {
    let caption = format!("session={}", summary.session_id);
    let mut sections = build_cli_chat_runtime_sections(summary);
    let operator_callout = TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title: Some("next moves".to_owned()),
        lines: vec![format!(
            "Use {CLI_CHAT_COMPACT_COMMAND} to checkpoint the active session window on demand, then return to the transcript."
        )],
    };
    sections.push(operator_callout);

    TuiMessageSpec {
        role: "control deck".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![STATUS_QUICK_COMMANDS_HINT.to_owned()],
    }
}

pub(super) fn build_cli_chat_runtime_sections(
    summary: &CliChatStartupSummary,
) -> Vec<TuiSectionSpec> {
    let allowed_channels = if summary.allowed_channels.is_empty() {
        "-".to_owned()
    } else {
        summary.allowed_channels.join(",")
    };
    let compaction_min_messages = summary
        .compaction_min_messages
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let compaction_trigger_estimated_tokens = summary
        .compaction_trigger_estimated_tokens
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let compaction_preserve_recent_estimated_tokens = summary
        .compaction_preserve_recent_estimated_tokens
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let runtime_value = format!(
        "ACP enabled={} dispatch_enabled={} routing={} backend={} ({}) allowed_channels={allowed_channels}",
        summary.acp_enabled,
        summary.dispatch_enabled,
        summary.conversation_routing,
        summary.acp_backend_id,
        summary.acp_backend_source,
    );
    let context_engine_value = format!(
        "{} ({})",
        summary.context_engine_id, summary.context_engine_source
    );
    let session_section = TuiSectionSpec::KeyValues {
        title: Some("session anchor".to_owned()),
        items: vec![
            tui_plain_item("session", summary.session_id.clone()),
            tui_plain_item("config", summary.config_path.clone()),
            tui_plain_item("memory", summary.memory_label.clone()),
        ],
    };
    let runtime_section = TuiSectionSpec::KeyValues {
        title: Some("runtime posture".to_owned()),
        items: vec![
            tui_plain_item("context engine", context_engine_value),
            tui_plain_item("acp", runtime_value),
        ],
    };
    let continuity_section = TuiSectionSpec::KeyValues {
        title: Some("continuity guardrails".to_owned()),
        items: vec![
            tui_plain_item("compaction", summary.compaction_enabled.to_string()),
            tui_plain_item("min messages", compaction_min_messages),
            tui_plain_item("trigger tokens", compaction_trigger_estimated_tokens),
            tui_plain_item(
                "preserve recent",
                summary.compaction_preserve_recent_turns.to_string(),
            ),
            tui_plain_item(
                "preserve recent tokens",
                compaction_preserve_recent_estimated_tokens,
            ),
            tui_plain_item("fail open", summary.compaction_fail_open.to_string()),
        ],
    };
    let mut sections = vec![session_section, runtime_section, continuity_section];

    if summary.explicit_acp_request
        || summary.event_stream_enabled
        || !summary.bootstrap_mcp_servers.is_empty()
        || summary.working_directory.is_some()
    {
        let bootstrap_label = if summary.bootstrap_mcp_servers.is_empty() {
            "-".to_owned()
        } else {
            summary.bootstrap_mcp_servers.join(",")
        };
        let working_directory = summary.working_directory.as_deref().unwrap_or("-");
        let override_lines = vec![
            format!("explicit request: {}", summary.explicit_acp_request),
            format!("event stream: {}", summary.event_stream_enabled),
            format!("bootstrap MCP servers: {bootstrap_label}"),
            format!("working directory: {working_directory}"),
        ];
        let override_callout = TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("acp overrides".to_owned()),
            lines: override_lines,
        };
        sections.push(override_callout);
    }

    sections
}
