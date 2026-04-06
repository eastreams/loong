use super::*;

#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::Capability;
#[cfg(feature = "memory-sqlite")]
use serde_json::json;

use crate::conversation::{
    collect_context_engine_runtime_snapshot, resolve_context_engine_selection,
};
use crate::runtime_self_continuity;

pub(super) async fn print_turn_checkpoint_startup_health(runtime: &CliTurnRuntime) {
    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_turn_checkpoint_diagnostics(
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
    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_turn_checkpoint_diagnostics(
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

pub(super) fn render_cli_chat_startup_lines(summary: &CliChatStartupSummary) -> Vec<String> {
    let render_width = detect_cli_chat_render_width();
    render_cli_chat_startup_lines_with_width(summary, render_width)
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
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_startup_screen_spec(summary: &CliChatStartupSummary) -> TuiScreenSpec {
    let mut sections = vec![
        TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "first prompt".to_owned(),
                command: DEFAULT_FIRST_PROMPT.to_owned(),
            }],
        },
        TuiSectionSpec::Narrative {
            title: None,
            lines: vec!["- type your request, or use /help for commands".to_owned()],
        },
    ];
    let runtime_sections = build_cli_chat_runtime_sections(summary);
    sections.extend(runtime_sections);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("chat ready".to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
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
        footer_lines: Vec::new(),
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
    let runtime_line = format!(
        "ACP enabled={} dispatch_enabled={} routing={} backend={} ({}) allowed_channels={allowed_channels}",
        summary.acp_enabled,
        summary.dispatch_enabled,
        summary.conversation_routing,
        summary.acp_backend_id,
        summary.acp_backend_source,
    );
    let session_items = vec![
        TuiKeyValueSpec::Plain {
            key: "session".to_owned(),
            value: summary.session_id.clone(),
        },
        TuiKeyValueSpec::Plain {
            key: "config".to_owned(),
            value: summary.config_path.clone(),
        },
        TuiKeyValueSpec::Plain {
            key: "memory".to_owned(),
            value: summary.memory_label.clone(),
        },
    ];
    let context_engine_value = format!(
        "{} ({})",
        summary.context_engine_id, summary.context_engine_source
    );
    let runtime_items = vec![
        TuiKeyValueSpec::Plain {
            key: "context engine".to_owned(),
            value: context_engine_value,
        },
        TuiKeyValueSpec::Plain {
            key: "acp".to_owned(),
            value: runtime_line,
        },
    ];
    let session_section = TuiSectionSpec::KeyValues {
        title: Some("session details".to_owned()),
        items: session_items,
    };
    let runtime_section = TuiSectionSpec::KeyValues {
        title: Some("runtime details".to_owned()),
        items: runtime_items,
    };
    let continuity_section = TuiSectionSpec::KeyValues {
        title: Some("continuity maintenance".to_owned()),
        items: vec![
            TuiKeyValueSpec::Plain {
                key: "compaction".to_owned(),
                value: summary.compaction_enabled.to_string(),
            },
            TuiKeyValueSpec::Plain {
                key: "min messages".to_owned(),
                value: compaction_min_messages,
            },
            TuiKeyValueSpec::Plain {
                key: "trigger tokens".to_owned(),
                value: compaction_trigger_estimated_tokens,
            },
            TuiKeyValueSpec::Plain {
                key: "preserve recent".to_owned(),
                value: summary.compaction_preserve_recent_turns.to_string(),
            },
            TuiKeyValueSpec::Plain {
                key: "fail open".to_owned(),
                value: summary.compaction_fail_open.to_string(),
            },
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
        let cwd_label = summary.working_directory.as_deref().unwrap_or("-");
        let override_lines = vec![
            format!("explicit request: {}", summary.explicit_acp_request),
            format!("event stream: {}", summary.event_stream_enabled),
            format!("bootstrap MCP servers: {bootstrap_label}"),
            format!("working directory: {cwd_label}"),
        ];
        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("acp overrides".to_owned()),
            lines: override_lines,
        });
    }

    sections
}

pub(super) fn render_cli_chat_help_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = build_cli_chat_help_message_spec();
    render_tui_message_spec(&message_spec, width)
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
    let note_lines = vec![
        "Type any non-command text to send a normal assistant turn.".to_owned(),
        "Use /status to inspect runtime maintenance settings without sending a turn.".to_owned(),
        "Use /history to inspect the active memory window when a reply feels off.".to_owned(),
        "Use /compact to checkpoint the active session before the next turn.".to_owned(),
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("commands".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("slash commands".to_owned()),
                items: command_items,
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("usage notes".to_owned()),
                lines: note_lines,
            },
        ],
        footer_lines: Vec::new(),
    }
}

pub(super) fn print_help() {
    let render_width = detect_cli_chat_render_width();
    let rendered_lines = render_cli_chat_help_lines_with_width(render_width);
    print_rendered_cli_chat_lines(&rendered_lines);
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_manual_compaction_lines_with_width(
    session_id: &str,
    result: &ManualCompactionResult,
    width: usize,
) -> Vec<String> {
    let message_spec = build_manual_compaction_message_spec(session_id, result);
    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
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
        footer_lines: Vec::new(),
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_manual_compaction_status(status: ManualCompactionStatus) -> &'static str {
    match status {
        ManualCompactionStatus::Applied => "applied",
        ManualCompactionStatus::NoChange => "no_change",
        ManualCompactionStatus::FailedOpen => "failed_open",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_manual_compaction_tokens(value: Option<usize>) -> String {
    let Some(value) = value else {
        return "-".to_owned();
    };

    value.to_string()
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn manual_compaction_tone(status: ManualCompactionStatus) -> TuiCalloutTone {
    match status {
        ManualCompactionStatus::Applied => TuiCalloutTone::Success,
        ManualCompactionStatus::NoChange => TuiCalloutTone::Info,
        ManualCompactionStatus::FailedOpen => TuiCalloutTone::Warning,
    }
}

pub(super) async fn print_manual_compaction(runtime: &CliTurnRuntime) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let binding = ConversationRuntimeBinding::kernel(&runtime.kernel_ctx);
        let result = load_manual_compaction_result(
            &runtime.config,
            &runtime.session_id,
            &runtime.turn_coordinator,
            binding,
            &runtime.memory_config,
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

#[cfg(feature = "memory-sqlite")]
pub(super) async fn load_manual_compaction_result(
    config: &LoongClawConfig,
    session_id: &str,
    turn_coordinator: &ConversationTurnCoordinator,
    binding: ConversationRuntimeBinding<'_>,
    _memory_config: &MemoryRuntimeConfig,
) -> CliResult<ManualCompactionResult> {
    let before_snapshot = load_manual_compaction_window_snapshot(session_id, binding).await?;
    let before_turns = resolve_manual_compaction_turn_count(&before_snapshot);
    let report = turn_coordinator
        .compact_session(config, session_id, binding)
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
    if !content.starts_with("Compacted ") {
        return None;
    }

    let headline = content.lines().next()?.trim();
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
    let rendered_lines = render_tui_message_spec(&message_spec, width);

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
            return render_tui_message_spec(
                &TuiMessageSpec {
                    role: "checkpoint".to_owned(),
                    caption: Some(caption),
                    sections: vec![TuiSectionSpec::Callout {
                        tone: TuiCalloutTone::Info,
                        title: Some("durability status".to_owned()),
                        lines: vec![detail_line],
                    }],
                    footer_lines: Vec::new(),
                },
                width,
            );
        }
    };

    render_tui_message_spec(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_health_message_spec(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    always_emit: bool,
) -> Option<TuiMessageSpec> {
    let summary = diagnostics.summary();
    if !always_emit && !summary.checkpoint_durable {
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
        footer_lines: Vec::new(),
    })
}
