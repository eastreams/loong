use crate::CliResult;

use super::CLI_CHAT_COMPACT_COMMAND;
use super::CLI_CHAT_STATUS_COMMAND;
use super::CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND;
use super::CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND_ALIAS;

pub(super) use super::deck::print_help;
pub(super) use super::deck::render_cli_chat_help_lines_with_width;
pub(super) use super::onboard::render_cli_chat_missing_config_decline_lines_with_width;
pub(super) use super::onboard::render_cli_chat_missing_config_lines_with_width;
pub(super) use super::onboard::should_run_missing_config_onboard;
pub(super) use super::startup_state::CliChatStartupSummary;
pub(super) use super::startup_state::build_cli_chat_startup_summary;
pub(super) use super::startup_view::build_cli_chat_startup_screen_spec;
pub(super) use super::startup_view::print_cli_chat_startup;
#[cfg(test)]
pub(super) use super::startup_view::render_cli_chat_startup_lines_with_width;
pub(super) use super::status_view::render_cli_chat_status_lines_with_width;

#[cfg(test)]
pub(super) use super::history::ManualCompactionResult;
#[cfg(test)]
pub(super) use super::history::ManualCompactionStatus;
#[cfg(feature = "memory-sqlite")]
pub(super) use super::history::load_history_lines;
#[cfg(feature = "memory-sqlite")]
pub(super) use super::history::load_manual_compaction_result;
#[cfg(test)]
pub(super) use super::history::manual_compaction_status_from_report;
pub(super) use super::history::print_history;
pub(super) use super::history::print_manual_compaction;
pub(super) use super::history::render_cli_chat_history_lines_with_width;
pub(super) use super::history::render_manual_compaction_lines_with_width;

pub(super) use super::report::print_cli_chat_status;
pub(super) use super::report::print_fast_lane_summary;
pub(super) use super::report::print_safe_lane_summary;
pub(super) use super::report::print_turn_checkpoint_repair;
pub(super) use super::report::print_turn_checkpoint_startup_health;
pub(super) use super::report::print_turn_checkpoint_summary;
#[cfg(test)]
pub(super) use super::report::render_turn_checkpoint_startup_health_lines_with_width;
#[cfg(test)]
pub(super) use super::report::render_turn_checkpoint_status_health_lines_with_width;

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
        &[
            "/turn_checkpoint_summary",
            "/turn-checkpoint-summary",
            "/checkpoint_summary",
        ],
    )
}

pub(super) fn parse_exact_chat_command(
    input: &str,
    aliases: &[&str],
    usage: &str,
) -> CliResult<bool> {
    let trimmed = input.trim();
    let Some(command) = trimmed.split_whitespace().next() else {
        return Ok(false);
    };
    if !aliases.contains(&command) {
        return Ok(false);
    }
    if trimmed == command {
        return Ok(true);
    }
    Err(usage.to_owned())
}

pub(super) fn is_manual_compaction_command(input: &str) -> CliResult<bool> {
    let aliases = [CLI_CHAT_COMPACT_COMMAND];
    let usage = format!("usage: {CLI_CHAT_COMPACT_COMMAND}");
    parse_exact_chat_command(input, &aliases, usage.as_str())
}

pub(super) fn is_cli_chat_status_command(input: &str) -> CliResult<bool> {
    let aliases = [CLI_CHAT_STATUS_COMMAND];
    let usage = format!("usage: {CLI_CHAT_STATUS_COMMAND}");
    parse_exact_chat_command(input, &aliases, usage.as_str())
}

pub(super) fn is_turn_checkpoint_repair_command(input: &str) -> CliResult<bool> {
    let aliases = [
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND,
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND_ALIAS,
    ];
    let usage = format!(
        "usage: {} (alias: {})",
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND, CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND_ALIAS,
    );
    parse_exact_chat_command(input, &aliases, usage.as_str())
}
