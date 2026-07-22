use crate::config::LoongConfig;
#[cfg(any(test, feature = "memory-sqlite"))]
use crate::conversation::ContextCompactionReport;
use crate::conversation::ConversationTurnCoordinator;
#[cfg(any(test, feature = "memory-sqlite"))]
use crate::memory;
#[cfg(any(test, feature = "memory-sqlite"))]
use crate::runtime_self_continuity;
use crate::tui_surface::{TuiCalloutTone, TuiMessageSpec, TuiSectionSpec};
use crate::{CliResult, Context};

use super::CliTurnRuntime;
use super::detect_cli_chat_render_width;
use super::print_rendered_cli_chat_lines;
#[cfg(not(feature = "memory-sqlite"))]
use super::render_cli_chat_feature_unavailable_lines_with_width;
use super::render_cli_chat_message_spec_with_width;
use super::tui_plain_item;

const CONTINUE_OR_STATUS_HINT: &str =
    "Continue chatting, or run /status to inspect maintenance settings.";
const STATUS_OR_COMPACT_HINT: &str =
    "Use /status for runtime state or /compact before the next turn.";

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

#[allow(clippy::print_stdout)]
pub(super) async fn print_manual_compaction(runtime: &CliTurnRuntime) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let context = runtime.context().map_err(|error| error.to_string())?;
        let result =
            load_manual_compaction_result(&runtime.config, &context, &runtime.turn_coordinator)
                .await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_manual_compaction_lines_with_width(
            context.session().session_id(),
            &result,
            render_width,
        );
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

#[allow(clippy::print_stdout)]
pub(super) async fn print_history(limit: usize, context: &Context<'_>) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let history_lines = load_history_lines(limit, context).await?;
        let render_width = detect_cli_chat_render_width();
        let rendered_lines = render_cli_chat_history_lines_with_width(
            context.session().session_id(),
            limit,
            &history_lines,
            render_width,
        );
        print_rendered_cli_chat_lines(&rendered_lines);
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, context);
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
    config: &LoongConfig,
    context: &Context<'_>,
    turn_coordinator: &ConversationTurnCoordinator,
) -> CliResult<ManualCompactionResult> {
    let before_snapshot = load_manual_compaction_window_snapshot(context).await?;
    let before_turns = resolve_manual_compaction_turn_count(&before_snapshot);
    let report = turn_coordinator
        .compact_production_session(config, context)
        .await?;
    let after_snapshot = load_manual_compaction_window_snapshot(context).await?;
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
    context: &Context<'_>,
) -> CliResult<ManualCompactionWindowSnapshot> {
    const MAX_MANUAL_COMPACTION_WINDOW_TURNS: usize = 512;

    let snapshot = context
        .access()
        .memory()
        .window(MAX_MANUAL_COMPACTION_WINDOW_TURNS, true)
        .await
        .map_err(|error| format!("load compaction window failed: {error}"))?;
    let turns = snapshot
        .turns
        .into_iter()
        .map(|turn| memory::WindowTurn {
            role: turn.role,
            content: turn.content,
            ts: turn.ts,
        })
        .collect();
    let turn_count = Some(snapshot.turn_count);

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
    Err(format!(
        "manual compaction returned unexpected status: {status_label}"
    ))
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_manual_compaction_detail(
    status: ManualCompactionStatus,
    summary_headline: &Option<String>,
) -> String {
    let continuity_note = runtime_self_continuity::compaction_summary_scope_note();
    match status {
        ManualCompactionStatus::Applied => match summary_headline {
            Some(headline) => format!("{headline}. {continuity_note}"),
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

#[cfg(feature = "memory-sqlite")]
pub(super) async fn load_history_lines(
    limit: usize,
    context: &Context<'_>,
) -> CliResult<Vec<String>> {
    let snapshot = context
        .access()
        .memory()
        .window(limit, false)
        .await
        .map_err(|error| format!("load history failed: {error}"))?;
    let turns = snapshot
        .turns
        .into_iter()
        .map(|turn| memory::WindowTurn {
            role: turn.role,
            content: turn.content,
            ts: turn.ts,
        })
        .collect::<Vec<_>>();
    Ok(format_window_history_lines(&turns))
}
