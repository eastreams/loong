#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::Capability;
#[cfg(feature = "memory-sqlite")]
use serde_json::json;

use crate::CliResult;
use crate::acp::resolve_acp_backend_selection;
use crate::config;
use crate::config::LoongClawConfig;
#[cfg(any(test, feature = "memory-sqlite"))]
use crate::conversation::ContextCompactionReport;
use crate::conversation::ConversationRuntimeBinding;
use crate::conversation::ConversationTurnCoordinator;
use crate::conversation::collect_context_engine_runtime_snapshot;
use crate::conversation::resolve_context_engine_selection;
#[cfg(any(test, feature = "memory-sqlite"))]
use crate::memory;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(any(test, feature = "memory-sqlite"))]
use crate::runtime_self_continuity;
use crate::tui_surface::TuiActionSpec;
use crate::tui_surface::TuiCalloutTone;
use crate::tui_surface::TuiChoiceSpec;
use crate::tui_surface::TuiHeaderStyle;
use crate::tui_surface::TuiKeyValueSpec;
use crate::tui_surface::TuiMessageSpec;
use crate::tui_surface::TuiScreenSpec;
use crate::tui_surface::TuiSectionSpec;
use crate::tui_surface::render_tui_screen_spec;

use super::CLI_CHAT_COMPACT_COMMAND;
use super::CLI_CHAT_HELP_COMMAND;
use super::CLI_CHAT_HISTORY_COMMAND;
use super::CLI_CHAT_STATUS_COMMAND;
use super::CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND;
use super::CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND_ALIAS;
use super::CliChatOptions;
use super::CliTurnRuntime;
use super::DEFAULT_FIRST_PROMPT;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::TurnCheckpointDiagnostics;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::TurnCheckpointDurabilityRenderLabels;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::TurnCheckpointRecoveryRenderLabels;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::TurnCheckpointSummaryRenderLabels;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::bool_yes_no_value;
use super::detect_cli_chat_render_width;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::format_turn_checkpoint_failure_step;
use super::print_rendered_cli_chat_lines;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::recovery_callout_tone;
#[cfg(not(feature = "memory-sqlite"))]
use super::render_cli_chat_feature_unavailable_lines_with_width;
use super::render_cli_chat_message_spec_with_width;
#[cfg(any(test, feature = "memory-sqlite"))]
use super::render_turn_checkpoint_health_error_lines_with_width;
use super::tui_plain_item;

const PRIMARY_QUICK_COMMANDS_HINT: &str = "Quick commands: /help · /status · /history · /compact";
const STATUS_QUICK_COMMANDS_HINT: &str = "Quick commands: /history · /compact · /help";
const TRANSCRIPT_START_HINT: &str = "Type any request to start the transcript.";
const STATUS_OR_COMPACT_HINT: &str =
    "Use /status for runtime state or /compact before the next turn.";
const CONTINUE_OR_STATUS_HINT: &str =
    "Continue chatting, or run /status to inspect maintenance settings.";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManualCompactionResult {
    pub(super) status: ManualCompactionStatus,
    pub(super) before_turns: usize,
    pub(super) after_turns: usize,
    pub(super) estimated_tokens_before: Option<usize>,
    pub(super) estimated_tokens_after: Option<usize>,
    pub(super) summary_headline: Option<String>,
    pub(super) detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManualCompactionStatus {
    Applied,
    NoChange,
    FailedOpen,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManualCompactionWindowSnapshot {
    turns: Vec<memory::WindowTurn>,
    turn_count: Option<usize>,
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) fn print_cli_chat_startup(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<()> {
    let summary = build_cli_chat_startup_summary(runtime, options)?;
    for line in render_cli_chat_startup_lines(&summary) {
        println!("{line}");
    }
    Ok(())
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_turn_checkpoint_startup_health(runtime: &CliTurnRuntime) {
    #[cfg(not(feature = "memory-sqlite"))]
    let _ = runtime;

    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_production_turn_checkpoint_diagnostics(
            &runtime.config,
            &runtime.session_id,
            crate::conversation::ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await
    {
        Ok(diagnostics) => {
            if let Some(rendered_lines) = render_turn_checkpoint_startup_health_lines_with_width(
                &runtime.session_id,
                &diagnostics,
                render_width,
            ) {
                print_rendered_cli_chat_lines(&rendered_lines);
            }
        }
        Err(error) => {
            let rendered_lines = render_turn_checkpoint_health_error_lines_with_width(
                &runtime.session_id,
                &error,
                render_width,
            );

            print_rendered_cli_chat_lines(&rendered_lines);
        }
    }
}

pub(super) async fn print_cli_chat_status(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<()> {
    let render_width = detect_cli_chat_render_width();
    let summary = build_cli_chat_startup_summary(runtime, options)?;
    let rendered_lines = render_cli_chat_status_lines_with_width(&summary, render_width);
    print_rendered_cli_chat_lines(&rendered_lines);
    print_turn_checkpoint_status_health(runtime).await;
    Ok(())
}

async fn print_turn_checkpoint_status_health(runtime: &CliTurnRuntime) {
    #[cfg(not(feature = "memory-sqlite"))]
    let _ = runtime;

    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_production_turn_checkpoint_diagnostics(
            &runtime.config,
            &runtime.session_id,
            crate::conversation::ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
        )
        .await
    {
        Ok(diagnostics) => {
            let rendered_lines = render_turn_checkpoint_status_health_lines_with_width(
                &runtime.session_id,
                &diagnostics,
                render_width,
            );

            print_rendered_cli_chat_lines(&rendered_lines);
        }
        Err(error) => {
            let rendered_lines = render_turn_checkpoint_health_error_lines_with_width(
                &runtime.session_id,
                &error,
                render_width,
            );

            print_rendered_cli_chat_lines(&rendered_lines);
        }
    }
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

fn render_cli_chat_startup_lines(summary: &CliChatStartupSummary) -> Vec<String> {
    let render_width = detect_cli_chat_render_width();
    render_cli_chat_startup_lines_with_width(summary, render_width)
}

pub(super) fn should_run_missing_config_onboard(read: usize, input: &str) -> bool {
    if read == 0 {
        return false;
    }

    let normalized_input = input.trim().to_ascii_lowercase();

    if normalized_input.is_empty() {
        return true;
    }

    matches!(normalized_input.as_str(), "y" | "yes")
}

pub(super) fn render_cli_chat_missing_config_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_missing_config_screen_spec(onboard_hint);
    render_tui_screen_spec(&screen_spec, width, false)
}

fn build_cli_chat_missing_config_screen_spec(onboard_hint: &str) -> TuiScreenSpec {
    let intro_lines = vec![
        format!("Welcome to {}!", config::PRODUCT_DISPLAY_NAME),
        "No configuration found for interactive chat.".to_owned(),
    ];
    let sections = vec![TuiSectionSpec::ActionGroup {
        title: Some("setup command".to_owned()),
        inline_title_when_wide: true,
        items: vec![TuiActionSpec {
            label: "start setup".to_owned(),
            command: onboard_hint.to_owned(),
        }],
    }];
    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: "run setup wizard".to_owned(),
            detail_lines: vec!["Create a config now and return to interactive chat.".to_owned()],
            recommended: true,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: "skip for now".to_owned(),
            detail_lines: vec!["Exit chat now and keep the setup command for later.".to_owned()],
            recommended: false,
        },
    ];
    let footer_lines = vec!["Press Enter to accept y.".to_owned()];

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("setup required".to_owned()),
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    }
}

pub(super) fn render_cli_chat_missing_config_decline_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_missing_config_decline_message_spec(onboard_hint);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_cli_chat_missing_config_decline_message_spec(onboard_hint: &str) -> TuiMessageSpec {
    let setup_hint = format!("You can run '{onboard_hint}' later to get started.");
    let sections = vec![
        TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("setup skipped".to_owned()),
            lines: vec![setup_hint],
        },
        TuiSectionSpec::ActionGroup {
            title: Some("start later".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "setup command".to_owned(),
                command: onboard_hint.to_owned(),
            }],
        },
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("setup required".to_owned()),
        sections,
        footer_lines: vec!["Run setup now to unlock the full chat surface.".to_owned()],
    }
}

pub(super) fn render_cli_chat_startup_lines_with_width(
    summary: &CliChatStartupSummary,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_startup_screen_spec(summary);
    render_tui_screen_spec(&screen_spec, width, false)
}

pub(super) fn render_cli_chat_status_lines_with_width(
    summary: &CliChatStartupSummary,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_status_message_spec(summary);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_cli_chat_startup_screen_spec(summary: &CliChatStartupSummary) -> TuiScreenSpec {
    let first_prompt_action = TuiActionSpec {
        label: "first prompt".to_owned(),
        command: DEFAULT_FIRST_PROMPT.to_owned(),
    };
    let start_here_section = TuiSectionSpec::ActionGroup {
        title: Some("start here".to_owned()),
        inline_title_when_wide: true,
        items: vec![first_prompt_action],
    };
    let narrative_section = TuiSectionSpec::Narrative {
        title: None,
        lines: vec!["- type your request, or use /help for commands".to_owned()],
    };
    let runtime_sections = build_cli_chat_runtime_sections(summary);
    let mut sections = vec![start_here_section, narrative_section];
    sections.extend(runtime_sections);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("chat ready".to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: vec![
            PRIMARY_QUICK_COMMANDS_HINT.to_owned(),
            TRANSCRIPT_START_HINT.to_owned(),
        ],
    }
}

fn build_cli_chat_status_message_spec(summary: &CliChatStartupSummary) -> TuiMessageSpec {
    let caption = format!("session={}", summary.session_id);
    let mut sections = build_cli_chat_runtime_sections(summary);
    let operator_callout = TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title: Some("operator controls".to_owned()),
        lines: vec![format!(
            "Use {CLI_CHAT_COMPACT_COMMAND} to checkpoint the active session window on demand."
        )],
    };
    sections.push(operator_callout);

    TuiMessageSpec {
        role: "status".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![STATUS_QUICK_COMMANDS_HINT.to_owned()],
    }
}

fn build_cli_chat_runtime_sections(summary: &CliChatStartupSummary) -> Vec<TuiSectionSpec> {
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
        title: Some("session details".to_owned()),
        items: vec![
            tui_plain_item("session", summary.session_id.clone()),
            tui_plain_item("config", summary.config_path.clone()),
            tui_plain_item("memory", summary.memory_label.clone()),
        ],
    };
    let runtime_section = TuiSectionSpec::KeyValues {
        title: Some("runtime details".to_owned()),
        items: vec![
            tui_plain_item("context engine", context_engine_value),
            tui_plain_item("acp", runtime_value),
        ],
    };
    let continuity_section = TuiSectionSpec::KeyValues {
        title: Some("continuity maintenance".to_owned()),
        items: vec![
            tui_plain_item("compaction", summary.compaction_enabled.to_string()),
            tui_plain_item("min messages", compaction_min_messages),
            tui_plain_item("trigger tokens", compaction_trigger_estimated_tokens),
            tui_plain_item(
                "preserve recent",
                summary.compaction_preserve_recent_turns.to_string(),
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

pub(super) fn render_cli_chat_help_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = build_cli_chat_help_message_spec();
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_cli_chat_help_message_spec() -> TuiMessageSpec {
    let command_items = vec![
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_HELP_COMMAND.to_owned(),
            value: "show chat commands".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_COMPACT_COMMAND.to_owned(),
            value: "write a continuity-safe checkpoint into the active window".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_STATUS_COMMAND.to_owned(),
            value: "show session, runtime, compaction, and durability status".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_HISTORY_COMMAND.to_owned(),
            value: "print the current session sliding window".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/fast_lane_summary [limit]".to_owned(),
            value: "summarize fast-lane batch execution events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/safe_lane_summary [limit]".to_owned(),
            value: "summarize safe-lane runtime events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/turn_checkpoint_summary [limit]".to_owned(),
            value: "summarize durable turn finalization state".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND.to_owned(),
            value: "repair durable turn finalization tail when safe".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/exit".to_owned(),
            value: "quit chat".to_owned(),
        },
    ];
    let usage_section = TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title: Some("usage notes".to_owned()),
        lines: vec![
            "Type any non-command text to send a normal assistant turn.".to_owned(),
            "Use /status to inspect runtime maintenance settings without sending a turn."
                .to_owned(),
            "Use /history to inspect the active memory window when a reply feels off.".to_owned(),
            "Use /compact to checkpoint the active session before the next turn.".to_owned(),
        ],
    };
    let command_section = TuiSectionSpec::KeyValues {
        title: Some("slash commands".to_owned()),
        items: command_items,
    };

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("commands".to_owned()),
        sections: vec![command_section, usage_section],
        footer_lines: vec![
            "Send normal text to continue the transcript.".to_owned(),
            "Use /exit to leave chat.".to_owned(),
        ],
    }
}

pub(super) fn render_cli_chat_history_lines_with_width(
    session_id: &str,
    limit: usize,
    history_lines: &[String],
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_history_message_spec(session_id, limit, history_lines);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_cli_chat_history_message_spec(
    session_id: &str,
    limit: usize,
    history_lines: &[String],
) -> TuiMessageSpec {
    let caption = format!("session={session_id} limit={limit}");
    let history_section = TuiSectionSpec::Narrative {
        title: Some("sliding window".to_owned()),
        lines: history_lines.to_vec(),
    };

    TuiMessageSpec {
        role: "history".to_owned(),
        caption: Some(caption),
        sections: vec![history_section],
        footer_lines: vec![STATUS_OR_COMPACT_HINT.to_owned()],
    }
}

pub(super) fn render_manual_compaction_lines_with_width(
    session_id: &str,
    result: &ManualCompactionResult,
    width: usize,
) -> Vec<String> {
    let message_spec = build_manual_compaction_message_spec(session_id, result);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_manual_compaction_message_spec(
    session_id: &str,
    result: &ManualCompactionResult,
) -> TuiMessageSpec {
    let caption = format!("session={session_id}");
    let status = format_manual_compaction_status(result.status).to_owned();
    let estimated_tokens_before = format_manual_compaction_tokens(result.estimated_tokens_before);
    let estimated_tokens_after = format_manual_compaction_tokens(result.estimated_tokens_after);
    let tone = manual_compaction_tone(result.status);
    let result_section = TuiSectionSpec::KeyValues {
        title: Some("compaction result".to_owned()),
        items: vec![
            tui_plain_item("status", status),
            tui_plain_item("before turns", result.before_turns.to_string()),
            tui_plain_item("after turns", result.after_turns.to_string()),
            tui_plain_item("tokens before", estimated_tokens_before),
            tui_plain_item("tokens after", estimated_tokens_after),
            tui_plain_item(
                "summary",
                result
                    .summary_headline
                    .clone()
                    .unwrap_or_else(|| "-".to_owned()),
            ),
        ],
    };
    let detail_section = TuiSectionSpec::Callout {
        tone,
        title: Some("details".to_owned()),
        lines: vec![result.detail.clone()],
    };

    TuiMessageSpec {
        role: "compact".to_owned(),
        caption: Some(caption),
        sections: vec![result_section, detail_section],
        footer_lines: vec![CONTINUE_OR_STATUS_HINT.to_owned()],
    }
}

fn format_manual_compaction_status(status: ManualCompactionStatus) -> &'static str {
    match status {
        ManualCompactionStatus::Applied => "applied",
        ManualCompactionStatus::NoChange => "no_change",
        ManualCompactionStatus::FailedOpen => "failed_open",
    }
}

fn format_manual_compaction_tokens(value: Option<usize>) -> String {
    let Some(value) = value else {
        return "-".to_owned();
    };
    value.to_string()
}

fn manual_compaction_tone(status: ManualCompactionStatus) -> TuiCalloutTone {
    match status {
        ManualCompactionStatus::Applied => TuiCalloutTone::Success,
        ManualCompactionStatus::NoChange => TuiCalloutTone::Info,
        ManualCompactionStatus::FailedOpen => TuiCalloutTone::Warning,
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) fn print_help() {
    let render_width = detect_cli_chat_render_width();
    let rendered_lines = render_cli_chat_help_lines_with_width(render_width);
    print_rendered_cli_chat_lines(&rendered_lines);
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_manual_compaction(runtime: &CliTurnRuntime) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let binding = ConversationRuntimeBinding::kernel(&runtime.kernel_ctx);
        let result = load_manual_compaction_result(
            &runtime.config,
            &runtime.session_id,
            &runtime.turn_coordinator,
            binding,
        )
        .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines =
            render_manual_compaction_lines_with_width(&runtime.session_id, &result, render_width);
        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = runtime;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "compact",
            "manual compaction unavailable: memory-sqlite feature disabled",
            render_width,
        );
        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_history(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let history_lines = load_history_lines(session_id, limit, binding, memory_config).await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_history_lines_with_width(
            session_id,
            limit,
            &history_lines,
            render_width,
        );
        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "history",
            "history unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn load_manual_compaction_result(
    config: &LoongClawConfig,
    session_id: &str,
    turn_coordinator: &ConversationTurnCoordinator,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<ManualCompactionResult> {
    let before_snapshot = load_manual_compaction_window_snapshot(session_id, binding).await?;
    let before_turns = resolve_manual_compaction_turn_count(&before_snapshot);
    let report = turn_coordinator
        .compact_production_session(config, session_id, binding)
        .await?;
    let after_snapshot = load_manual_compaction_window_snapshot(session_id, binding).await?;
    let after_turns = resolve_manual_compaction_turn_count(&after_snapshot);
    let summary_headline = extract_manual_compaction_summary_headline(&after_snapshot);
    let status = manual_compaction_status_from_report(&report)?;
    let detail = build_manual_compaction_detail(status, &summary_headline);

    Ok(ManualCompactionResult {
        status,
        before_turns,
        after_turns,
        estimated_tokens_before: report.estimated_tokens_before,
        estimated_tokens_after: report.estimated_tokens_after,
        summary_headline,
        detail,
    })
}

#[cfg(feature = "memory-sqlite")]
async fn load_manual_compaction_window_snapshot(
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<ManualCompactionWindowSnapshot> {
    const MAX_MANUAL_COMPACTION_WINDOW_TURNS: usize = 512;

    let kernel_ctx = binding
        .kernel_context()
        .ok_or_else(|| "manual compaction requires a kernel-bound session".to_owned())?;
    let caps = BTreeSet::from([Capability::MemoryRead]);
    let request = loongclaw_contracts::MemoryCoreRequest {
        operation: memory::MEMORY_OP_WINDOW.to_owned(),
        payload: json!({
            "session_id": session_id,
            "limit": MAX_MANUAL_COMPACTION_WINDOW_TURNS,
            "allow_extended_limit": true,
        }),
    };
    let outcome = kernel_ctx
        .kernel
        .execute_memory_core(
            kernel_ctx.pack_id(),
            &kernel_ctx.token,
            &caps,
            None,
            request,
        )
        .await
        .map_err(|error| format!("load compaction window via kernel failed: {error}"))?;

    if outcome.status != "ok" {
        let status = outcome.status;
        let message = format!("load compaction window via kernel returned non-ok status: {status}");
        return Err(message);
    }

    let turns = memory::decode_window_turns(&outcome.payload);
    let turn_count = memory::decode_window_turn_count(&outcome.payload);

    Ok(ManualCompactionWindowSnapshot { turns, turn_count })
}

#[cfg(feature = "memory-sqlite")]
fn resolve_manual_compaction_turn_count(snapshot: &ManualCompactionWindowSnapshot) -> usize {
    snapshot.turn_count.unwrap_or(snapshot.turns.len())
}

#[cfg(feature = "memory-sqlite")]
fn extract_manual_compaction_summary_headline(
    snapshot: &ManualCompactionWindowSnapshot,
) -> Option<String> {
    let first_turn = snapshot.turns.first()?;
    let content = first_turn.content.trim();
    if !crate::conversation::is_compacted_summary_content(content) {
        return None;
    }

    let headline = content
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(crate::conversation::COMPACTED_SUMMARY_PREFIX))
        .or_else(|| content.lines().next().map(str::trim))?;
    Some(headline.to_owned())
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn manual_compaction_status_from_report(
    report: &ContextCompactionReport,
) -> CliResult<ManualCompactionStatus> {
    if report.was_applied() {
        return Ok(ManualCompactionStatus::Applied);
    }

    if report.was_skipped() {
        return Ok(ManualCompactionStatus::NoChange);
    }

    if report.was_failed_open() {
        return Ok(ManualCompactionStatus::FailedOpen);
    }

    let status_label = report.status_label();
    let message = format!("manual compaction returned unexpected status: {status_label}");
    Err(message)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_manual_compaction_detail(
    status: ManualCompactionStatus,
    summary_headline: &Option<String>,
) -> String {
    let continuity_note = runtime_self_continuity::compaction_summary_scope_note();
    match status {
        ManualCompactionStatus::Applied => match summary_headline {
            Some(headline) => {
                format!("{headline}. {continuity_note}")
            }
            None => {
                format!(
                    "Compaction completed and the active session window was rewritten. {continuity_note}"
                )
            }
        },
        ManualCompactionStatus::NoChange => {
            "No compaction change applied. The active session was already summarized or already compact enough."
                .to_owned()
        }
        ManualCompactionStatus::FailedOpen => {
            "Compaction failed open and left the current history unchanged. Inspect /status and /history before continuing."
                .to_owned()
        }
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_window_history_lines(turns: &[memory::WindowTurn]) -> Vec<String> {
    if turns.is_empty() {
        return vec!["(no history yet)".to_owned()];
    }

    turns
        .iter()
        .map(|turn| {
            format!(
                "[{}] {}: {}",
                turn.ts.unwrap_or_default(),
                turn.role,
                turn.content
            )
        })
        .collect()
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_prompt_context_history_lines(entries: &[memory::MemoryContextEntry]) -> Vec<String> {
    if entries.is_empty() {
        return vec!["(no history yet)".to_owned()];
    }

    let mut lines = Vec::new();
    for entry in entries {
        match entry.kind {
            memory::MemoryContextKind::Profile => {
                lines.push("[profile]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::Summary => {
                lines.push("[summary]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::Derived => {
                lines.push("[derived]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::RetrievedMemory => {
                lines.push("[retrieved_memory]".to_owned());
                lines.push(entry.content.clone());
            }
            memory::MemoryContextKind::Turn => {
                lines.push(format!("{}: {}", entry.role, entry.content));
            }
        }
    }
    lines
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn load_history_lines(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<Vec<String>> {
    if let Some(ctx) = binding.kernel_context() {
        let request = memory::build_window_request(session_id, limit);
        let caps = BTreeSet::from([Capability::MemoryRead]);
        let outcome = ctx
            .kernel
            .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
            .await
            .map_err(|error| format!("load history via kernel failed: {error}"))?;
        if outcome.status != "ok" {
            return Err(format!(
                "load history via kernel returned non-ok status: {}",
                outcome.status
            ));
        }
        let turns = memory::decode_window_turns(&outcome.payload);
        return Ok(format_window_history_lines(&turns));
    }

    let entries = memory::load_prompt_context(session_id, memory_config)
        .map_err(|error| format!("load history failed: {error}"))?;
    Ok(format_prompt_context_history_lines(&entries))
}

pub(super) fn parse_safe_lane_summary_limit(
    input: &str,
    default_window: usize,
) -> CliResult<Option<usize>> {
    parse_summary_limit(
        input,
        default_window,
        &["/safe_lane_summary", "/safe-lane-summary"],
    )
}

pub(super) fn parse_fast_lane_summary_limit(
    input: &str,
    default_window: usize,
) -> CliResult<Option<usize>> {
    parse_summary_limit(
        input,
        default_window,
        &["/fast_lane_summary", "/fast-lane-summary"],
    )
}

pub(super) fn parse_summary_limit(
    input: &str,
    default_window: usize,
    aliases: &[&str],
) -> CliResult<Option<usize>> {
    let Some(primary_alias) = aliases.first().copied() else {
        return Ok(None);
    };

    let mut tokens = input.split_whitespace();
    let Some(command) = tokens.next() else {
        return Ok(None);
    };
    if !aliases.contains(&command) {
        return Ok(None);
    }

    let usage = format!("usage: {primary_alias} [limit]");
    let default_limit = default_window.saturating_mul(4).max(64);
    let limit = match tokens.next() {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|error| format!("invalid {primary_alias} limit `{raw}`: {error}; {usage}"))?,
        None => default_limit,
    };
    if limit == 0 {
        return Err(format!("invalid {primary_alias} limit `0`; {usage}"));
    }
    if tokens.next().is_some() {
        return Err(usage);
    }
    Ok(Some(limit))
}

pub(super) fn parse_turn_checkpoint_summary_limit(
    input: &str,
    default_window: usize,
) -> CliResult<Option<usize>> {
    parse_summary_limit(
        input,
        default_window,
        &["/turn_checkpoint_summary", "/turn-checkpoint-summary"],
    )
}

pub(super) fn parse_exact_chat_command(
    input: &str,
    aliases: &[&str],
    usage: &str,
) -> CliResult<bool> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }

    let mut tokens = trimmed.split_whitespace();
    let Some(command) = tokens.next() else {
        return Ok(false);
    };
    if !aliases.contains(&command) {
        return Ok(false);
    }

    if tokens.next().is_some() {
        return Err(usage.to_owned());
    }

    Ok(true)
}

pub(super) fn is_manual_compaction_command(input: &str) -> CliResult<bool> {
    let aliases = [CLI_CHAT_COMPACT_COMMAND];
    let usage = "usage: /compact";
    parse_exact_chat_command(input, &aliases, usage)
}

pub(super) fn is_cli_chat_status_command(input: &str) -> CliResult<bool> {
    let aliases = [CLI_CHAT_STATUS_COMMAND];
    let usage = "usage: /status";
    parse_exact_chat_command(input, &aliases, usage)
}

pub(super) fn is_turn_checkpoint_repair_command(input: &str) -> CliResult<bool> {
    let aliases = [
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND,
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND_ALIAS,
    ];
    let usage = "usage: /turn_checkpoint_repair";
    parse_exact_chat_command(input, &aliases, usage)
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_startup_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Option<Vec<String>> {
    let message_spec = build_turn_checkpoint_health_message_spec(
        session_id,
        diagnostics,
        /*always_emit*/ false,
    )?;
    let rendered_lines = render_cli_chat_message_spec_with_width(&message_spec, width);

    Some(rendered_lines)
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_status_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_health_message_spec(
        session_id,
        diagnostics,
        /*always_emit*/ true,
    );
    let message_spec = match message_spec {
        Some(message_spec) => message_spec,
        None => {
            let caption = format!("session={session_id}");
            let detail_line = "checkpoint diagnostics unavailable".to_owned();
            return render_cli_chat_message_spec_with_width(
                &TuiMessageSpec {
                    role: "checkpoint".to_owned(),
                    caption: Some(caption),
                    sections: vec![TuiSectionSpec::Callout {
                        tone: TuiCalloutTone::Info,
                        title: Some("durability status".to_owned()),
                        lines: vec![detail_line],
                    }],
                    footer_lines: vec![
                        "Use /status to refresh runtime health after the next turn.".to_owned(),
                    ],
                },
                width,
            );
        }
    };

    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_health_message_spec(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    always_emit: bool,
) -> Option<TuiMessageSpec> {
    let summary = diagnostics.summary();
    if !always_emit && !summary.checkpoint_durable && !summary.requires_recovery {
        return None;
    }

    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let recovery_needed = bool_yes_no_value(summary.requires_recovery);
    let reply_durable = bool_yes_no_value(summary.reply_durable);
    let checkpoint_durable = bool_yes_no_value(summary.checkpoint_durable);
    let recovery_tone = recovery_callout_tone(summary.requires_recovery);
    let caption = format!("session={session_id}");
    let recovery_reason = recovery_labels.reason.to_owned();

    let mut sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("durability status".to_owned()),
            items: vec![
                tui_plain_item("state", render_labels.session_state.to_owned()),
                tui_plain_item("durability", durability_labels.durability.to_owned()),
                tui_plain_item("reply durable", reply_durable),
                tui_plain_item("checkpoint durable", checkpoint_durable),
            ],
        },
        TuiSectionSpec::Callout {
            tone: recovery_tone,
            title: Some("recovery".to_owned()),
            lines: vec![
                format!("recovery needed: {recovery_needed}"),
                format!("action: {}", recovery_labels.action),
                format!("source: {}", recovery_labels.source),
                format!("reason: {recovery_reason}"),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("latest turn".to_owned()),
            items: vec![
                tui_plain_item("stage", render_labels.stage.to_owned()),
                tui_plain_item("after turn", render_labels.after_turn.to_owned()),
                tui_plain_item("compaction", render_labels.compaction.to_owned()),
                tui_plain_item("lane", render_labels.lane.to_owned()),
                tui_plain_item("result kind", render_labels.result_kind.to_owned()),
                tui_plain_item(
                    "persistence mode",
                    render_labels.persistence_mode.to_owned(),
                ),
                tui_plain_item("identity", render_labels.identity.to_owned()),
                tui_plain_item("failure step", failure_step.to_owned()),
            ],
        },
    ];

    if render_labels.safe_lane_route_decision != "-"
        || render_labels.safe_lane_route_reason != "-"
        || render_labels.safe_lane_route_source != "-"
    {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("safe-lane route".to_owned()),
            items: vec![
                tui_plain_item(
                    "decision",
                    render_labels.safe_lane_route_decision.to_owned(),
                ),
                tui_plain_item("reason", render_labels.safe_lane_route_reason.to_owned()),
                tui_plain_item("source", render_labels.safe_lane_route_source.to_owned()),
            ],
        });
    }

    if let Some(probe) = diagnostics.runtime_probe() {
        let probe_lines = vec![
            format!("action: {}", probe.action().as_str()),
            format!("source: {}", probe.source().as_str()),
            format!("reason: {}", probe.reason().as_str()),
        ];

        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("runtime probe".to_owned()),
            lines: probe_lines,
        });
    }

    Some(TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec!["Use /turn_checkpoint_repair when recovery can run safely.".to_owned()],
    })
}
