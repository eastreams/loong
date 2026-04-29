#[cfg(feature = "memory-sqlite")]
use crate::conversation::{load_fast_lane_tool_batch_event_summary, load_safe_lane_event_summary};

use super::startup_state::build_cli_chat_startup_summary;
use super::status_view::render_cli_chat_status_lines_with_width;
use super::*;

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_turn_checkpoint_startup_health(runtime: &CliTurnRuntime) {
    #[cfg(not(feature = "memory-sqlite"))]
    let _ = runtime;

    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    let limit = runtime.config.memory.sliding_window;

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_production_turn_checkpoint_diagnostics_with_limit(
            &runtime.config,
            &runtime.session_id,
            limit,
            runtime.conversation_binding(),
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

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_fast_lane_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &SessionStoreConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let summary =
            load_fast_lane_tool_batch_event_summary(session_id, limit, binding, memory_config)
                .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines =
            render_fast_lane_summary_lines_with_width(session_id, limit, &summary, render_width);

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "fast-lane",
            "fast-lane summary unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_safe_lane_summary(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &SessionStoreConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let summary =
            load_safe_lane_event_summary(session_id, limit, binding, memory_config).await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_safe_lane_summary_lines_with_width(
            session_id,
            limit,
            conversation_config,
            &summary,
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, conversation_config, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "safe-lane",
            "safe-lane summary unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_turn_checkpoint_summary(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongConfig,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] _memory_config: &SessionStoreConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let diagnostics = turn_coordinator
            .load_production_turn_checkpoint_diagnostics_with_limit(
                config, session_id, limit, binding,
            )
            .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_turn_checkpoint_summary_lines_with_width(
            session_id,
            limit,
            &diagnostics,
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (turn_coordinator, config, session_id, limit, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "checkpoint",
            "turn checkpoint summary unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_turn_checkpoint_repair(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let outcome = turn_coordinator
            .repair_production_turn_checkpoint_tail(config, session_id, binding)
            .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines =
            render_turn_checkpoint_repair_lines_with_width(session_id, &outcome, render_width);

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (turn_coordinator, config, session_id, binding);
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_feature_unavailable_lines_with_width(
            "repair",
            "turn checkpoint repair unavailable: memory-sqlite feature disabled",
            render_width,
        );

        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }
}

async fn print_turn_checkpoint_status_health(runtime: &CliTurnRuntime) {
    #[cfg(not(feature = "memory-sqlite"))]
    let _ = runtime;

    #[cfg(feature = "memory-sqlite")]
    let render_width = detect_cli_chat_render_width();

    #[cfg(feature = "memory-sqlite")]
    let limit = runtime.config.memory.sliding_window;

    #[cfg(feature = "memory-sqlite")]
    match runtime
        .turn_coordinator
        .load_production_turn_checkpoint_diagnostics_with_limit(
            &runtime.config,
            &runtime.session_id,
            limit,
            runtime.conversation_binding(),
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

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_startup_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Option<Vec<String>> {
    let summary = diagnostics.summary();
    if !summary.checkpoint_durable && !summary.requires_recovery {
        return None;
    }

    let message_spec = build_turn_checkpoint_health_message_spec(session_id, diagnostics);
    Some(render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_status_health_lines_with_width(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_health_message_spec(session_id, diagnostics);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_health_message_spec(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
) -> TuiMessageSpec {
    let summary = diagnostics.summary();

    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let reply_durable = bool_yes_no_value(summary.reply_durable);
    let checkpoint_durable = bool_yes_no_value(summary.checkpoint_durable);
    let recovery_needed = bool_yes_no_value(summary.requires_recovery);
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

    if let Some(compaction_diagnostics) = summary.latest_compaction_diagnostics.as_ref() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("compaction diagnostics".to_owned()),
            items: compaction_diagnostics
                .key_value_pairs()
                .into_iter()
                .map(|(key, value)| tui_plain_item(key, value))
                .collect(),
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

    TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec!["Use /turn_checkpoint_repair when recovery can run safely.".to_owned()],
    }
}
