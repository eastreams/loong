use std::collections::BTreeSet;
use std::io::{self, Write};

use loongclaw_contracts::Capability;

use crate::CliResult;
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context};

use super::config::{self, LoongClawConfig};
use super::conversation::{
    ConversationTurnCoordinator, ProviderErrorMode, SafeLaneEventSummary, SafeLaneFinalStatus,
    summarize_safe_lane_events,
};
#[cfg(feature = "memory-sqlite")]
use super::memory;
#[cfg(feature = "memory-sqlite")]
use super::memory::runtime_config::MemoryRuntimeConfig;

#[allow(clippy::print_stdout)] // CLI REPL output
pub async fn run_cli_chat(config_path: Option<&str>, session_hint: Option<&str>) -> CliResult<()> {
    let (resolved_path, config) = config::load(config_path)?;
    if !config.cli.enabled {
        return Err("CLI channel is disabled by config.cli.enabled=false".to_owned());
    }

    export_runtime_env(&config);
    let kernel_ctx = bootstrap_kernel_context("cli-chat", DEFAULT_TOKEN_TTL_S)?;

    #[cfg(feature = "memory-sqlite")]
    let memory_config = MemoryRuntimeConfig {
        sqlite_path: Some(config.memory.resolved_sqlite_path()),
        sliding_window: Some(config.memory.sliding_window),
    };

    #[cfg(feature = "memory-sqlite")]
    {
        let sqlite_path = config.memory.resolved_sqlite_path();
        let initialized = memory::ensure_memory_db_ready(Some(sqlite_path.clone()), &memory_config)
            .map_err(|error| format!("failed to initialize sqlite memory: {error}"))?;
        println!(
            "loongclaw chat started (config={}, memory={})",
            resolved_path.display(),
            initialized.display()
        );
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        println!(
            "loongclaw chat started (config={}, memory=disabled)",
            resolved_path.display()
        );
    }

    let session_id = session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();
    println!("session={session_id} (type /help for commands, /exit to quit)");
    let turn_coordinator = ConversationTurnCoordinator::new();

    loop {
        print!("you> ");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;
        let mut line = String::new();
        let read = io::stdin()
            .read_line(&mut line)
            .map_err(|error| format!("read stdin failed: {error}"))?;
        if read == 0 {
            println!();
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if is_exit_command(&config, input) {
            break;
        }
        if input == "/help" {
            print_help();
            continue;
        }
        if input == "/history" {
            #[cfg(feature = "memory-sqlite")]
            print_history(
                &session_id,
                config.memory.sliding_window,
                Some(&kernel_ctx),
                &memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_history(&session_id, config.memory.sliding_window, Some(&kernel_ctx)).await?;
            continue;
        }
        if let Some(limit) = parse_safe_lane_summary_limit(input, config.memory.sliding_window)? {
            #[cfg(feature = "memory-sqlite")]
            print_safe_lane_summary(&session_id, limit, &memory_config)?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_safe_lane_summary(&session_id, limit)?;
            continue;
        }

        let assistant_text = turn_coordinator
            .handle_turn(
                &config,
                &session_id,
                input,
                ProviderErrorMode::InlineMessage,
                Some(&kernel_ctx),
            )
            .await?;

        println!("loongclaw> {assistant_text}");
    }

    println!("bye.");
    Ok(())
}

fn is_exit_command(config: &LoongClawConfig, input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    config
        .cli
        .exit_commands
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .any(|value| !value.is_empty() && value == lower)
}

#[allow(clippy::print_stdout)] // CLI output
fn print_help() {
    println!("/help    show this help");
    println!("/history print current session sliding window");
    println!("/safe_lane_summary [limit]  summarize safe-lane runtime events");
    println!("/exit    quit chat");
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_history(
    session_id: &str,
    limit: usize,
    kernel_ctx: Option<&crate::KernelContext>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        if let Some(ctx) = kernel_ctx {
            let request = memory::build_window_request(session_id, limit);
            let caps = BTreeSet::from([Capability::MemoryRead]);
            let outcome = ctx
                .kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("load history via kernel failed: {error}"))?;
            let turns = memory::decode_window_turns(&outcome.payload);
            if turns.is_empty() {
                println!("(no history yet)");
                return Ok(());
            }
            for turn in turns {
                println!(
                    "[{}] {}: {}",
                    turn.ts.unwrap_or_default(),
                    turn.role,
                    turn.content
                );
            }
            return Ok(());
        }

        let turns = memory::window_direct(session_id, limit, memory_config)
            .map_err(|error| format!("load history failed: {error}"))?;
        if turns.is_empty() {
            println!("(no history yet)");
            return Ok(());
        }
        for turn in turns {
            println!("[{}] {}: {}", turn.ts, turn.role, turn.content);
        }
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, kernel_ctx);
        println!("history unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

fn parse_safe_lane_summary_limit(input: &str, default_window: usize) -> CliResult<Option<usize>> {
    let mut tokens = input.split_whitespace();
    let Some(command) = tokens.next() else {
        return Ok(None);
    };
    if command != "/safe_lane_summary" && command != "/safe-lane-summary" {
        return Ok(None);
    }

    let default_limit = default_window.saturating_mul(4).max(64);
    let limit = match tokens.next() {
        Some(raw) => raw.parse::<usize>().map_err(|error| {
            format!(
                "invalid /safe_lane_summary limit `{raw}`: {error}; usage: /safe_lane_summary [limit]"
            )
        })?,
        None => default_limit,
    };
    if limit == 0 {
        return Err(
            "invalid /safe_lane_summary limit `0`; usage: /safe_lane_summary [limit]".to_owned(),
        );
    }
    if tokens.next().is_some() {
        return Err("usage: /safe_lane_summary [limit]".to_owned());
    }
    Ok(Some(limit))
}

#[allow(clippy::print_stdout)] // CLI output
fn print_safe_lane_summary(
    session_id: &str,
    limit: usize,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        let turns = memory::window_direct(session_id, limit, memory_config)
            .map_err(|error| format!("load safe-lane summary failed: {error}"))?;
        let summary = summarize_safe_lane_events(
            turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str())),
        );
        println!("{}", format_safe_lane_summary(session_id, limit, &summary));
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit);
        println!("safe-lane summary unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

fn format_safe_lane_summary(
    session_id: &str,
    limit: usize,
    summary: &SafeLaneEventSummary,
) -> String {
    let final_status = match summary.final_status {
        Some(SafeLaneFinalStatus::Succeeded) => "succeeded",
        Some(SafeLaneFinalStatus::Failed) => "failed",
        None => "unknown",
    };
    let final_failure_code = summary.final_failure_code.as_deref().unwrap_or("-");
    let final_route = summary.final_route_decision.as_deref().unwrap_or("-");
    let final_route_reason = summary.final_route_reason.as_deref().unwrap_or("-");
    let metrics = summary.latest_metrics.as_ref();
    let rounds_started = metrics
        .map(|value| value.rounds_started as f64)
        .unwrap_or(summary.round_started_events as f64);
    let replan_rate = if rounds_started > 0.0 {
        summary.replan_triggered_events as f64 / rounds_started
    } else {
        0.0
    };
    let verify_failure_rate = if rounds_started > 0.0 {
        summary.verify_failed_events as f64 / rounds_started
    } else {
        0.0
    };
    let route_rollup = format_rollup_counts(&summary.route_decision_counts);
    let route_reason_rollup = format_rollup_counts(&summary.route_reason_counts);
    let failure_rollup = format_rollup_counts(&summary.failure_code_counts);
    let governor_trend_failure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_failure_ewma_milli);
    let governor_trend_backpressure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_backpressure_ewma_milli);

    let metrics_line = if let Some(metrics) = metrics {
        format!(
            "latest_metrics rounds_started={} rounds_succeeded={} rounds_failed={} verify_failures={} replans_triggered={} total_attempts_used={}",
            metrics.rounds_started,
            metrics.rounds_succeeded,
            metrics.rounds_failed,
            metrics.verify_failures,
            metrics.replans_triggered,
            metrics.total_attempts_used
        )
    } else {
        "latest_metrics unavailable".to_owned()
    };

    [
        format!("safe_lane_summary session={session_id} limit={limit}"),
        format!(
            "events lane_selected={} round_started={} round_completed_succeeded={} round_completed_failed={} verify_failed={} verify_policy_adjusted={} replan_triggered={} final_status={} governor_engaged={} governor_force_no_replan={}",
            summary.lane_selected_events,
            summary.round_started_events,
            summary.round_completed_succeeded_events,
            summary.round_completed_failed_events,
            summary.verify_failed_events,
            summary.verify_policy_adjusted_events,
            summary.replan_triggered_events,
            summary.final_status_events,
            summary.session_governor_engaged_events,
            summary.session_governor_force_no_replan_events
        ),
        format!(
            "terminal status={} failure_code={} route_decision={} route_reason={}",
            final_status, final_failure_code, final_route, final_route_reason
        ),
        format!(
            "governor trigger_failed_threshold={} trigger_backpressure_threshold={} trigger_trend_threshold={} trigger_recovery_threshold={}",
            summary.session_governor_failed_threshold_triggered_events,
            summary.session_governor_backpressure_threshold_triggered_events,
            summary.session_governor_trend_threshold_triggered_events,
            summary.session_governor_recovery_threshold_triggered_events
        ),
        format!(
            "governor_latest snapshots={} trend_samples={} trend_min_samples={} trend_failure_ewma={} trend_backpressure_ewma={} recovery_success_streak={} recovery_streak_threshold={}",
            summary.session_governor_metrics_snapshots_seen,
            summary
                .session_governor_latest_trend_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_trend_min_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            governor_trend_failure_ewma,
            governor_trend_backpressure_ewma,
            summary
                .session_governor_latest_recovery_success_streak
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_recovery_success_streak_threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
        ),
        format!(
            "rates replan_per_round={:.3} verify_fail_per_round={:.3}",
            replan_rate, verify_failure_rate
        ),
        metrics_line,
        format!("rollup route_decisions={route_rollup}"),
        format!("rollup route_reasons={route_reason_rollup}"),
        format!("rollup failure_codes={failure_rollup}"),
    ]
    .join("\n")
}

fn format_rollup_counts(counts: &std::collections::BTreeMap<String, u32>) -> String {
    if counts.is_empty() {
        return "-".to_owned();
    }
    counts
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}

fn export_runtime_env(config: &LoongClawConfig) {
    // Populate the typed tool runtime config so executors never hit env vars
    // on the hot path.  Ignore the error if already initialised (e.g. tests).
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig {
        shell_allowlist: config
            .tools
            .shell_allowlist
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect(),
        file_root: Some(config.tools.resolved_file_root()),
    };
    let _ = crate::tools::runtime_config::init_tool_runtime_config(tool_rt);

    // Populate the typed memory runtime config (same pattern as tool config).
    let memory_rt = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(config.memory.resolved_sqlite_path()),
        sliding_window: Some(config.memory.sliding_window),
    };
    let _ = crate::memory::runtime_config::init_memory_runtime_config(memory_rt);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_safe_lane_summary_limit_accepts_default_and_explicit_limit() {
        assert_eq!(
            parse_safe_lane_summary_limit("/safe_lane_summary", 20).expect("parse"),
            Some(80)
        );
        assert_eq!(
            parse_safe_lane_summary_limit("/safe-lane-summary 120", 20).expect("parse"),
            Some(120)
        );
    }

    #[test]
    fn parse_safe_lane_summary_limit_rejects_invalid_input() {
        let error = parse_safe_lane_summary_limit("/safe_lane_summary 0", 20)
            .expect_err("zero limit should be rejected");
        assert!(error.contains("usage"));

        let error = parse_safe_lane_summary_limit("/safe_lane_summary abc", 20)
            .expect_err("non-number limit should be rejected");
        assert!(error.contains("invalid"));
    }

    #[test]
    fn format_safe_lane_summary_includes_rollups_and_rates() {
        let mut summary = SafeLaneEventSummary {
            lane_selected_events: 1,
            round_started_events: 2,
            round_completed_succeeded_events: 1,
            round_completed_failed_events: 1,
            verify_failed_events: 1,
            replan_triggered_events: 1,
            final_status_events: 1,
            session_governor_engaged_events: 1,
            session_governor_force_no_replan_events: 1,
            session_governor_failed_threshold_triggered_events: 1,
            session_governor_backpressure_threshold_triggered_events: 0,
            session_governor_trend_threshold_triggered_events: 1,
            session_governor_recovery_threshold_triggered_events: 0,
            session_governor_metrics_snapshots_seen: 2,
            session_governor_latest_trend_samples: Some(5),
            session_governor_latest_trend_min_samples: Some(4),
            session_governor_latest_trend_failure_ewma_milli: Some(250),
            session_governor_latest_trend_backpressure_ewma_milli: Some(63),
            session_governor_latest_recovery_success_streak: Some(4),
            session_governor_latest_recovery_success_streak_threshold: Some(3),
            final_status: Some(SafeLaneFinalStatus::Failed),
            final_failure_code: Some("safe_lane_plan_verify_failed".to_owned()),
            final_route_decision: Some("terminal".to_owned()),
            final_route_reason: Some("session_governor_no_replan".to_owned()),
            latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
                rounds_started: 2,
                rounds_succeeded: 1,
                rounds_failed: 1,
                verify_failures: 1,
                replans_triggered: 1,
                total_attempts_used: 3,
            }),
            ..SafeLaneEventSummary::default()
        };
        summary
            .route_decision_counts
            .insert("terminal".to_owned(), 1);
        summary
            .route_reason_counts
            .insert("session_governor_no_replan".to_owned(), 1);
        summary
            .failure_code_counts
            .insert("safe_lane_plan_verify_failed".to_owned(), 1);
        let formatted = format_safe_lane_summary("session-a", 128, &summary);

        assert!(formatted.contains("safe_lane_summary session=session-a limit=128"));
        assert!(formatted.contains("status=failed"));
        assert!(formatted.contains("route_decision=terminal"));
        assert!(formatted.contains("route_reason=session_governor_no_replan"));
        assert!(formatted.contains("replan_per_round=0.500"));
        assert!(formatted.contains("governor_engaged=1"));
        assert!(formatted.contains("governor_force_no_replan=1"));
        assert!(formatted.contains("trigger_failed_threshold=1"));
        assert!(formatted.contains("trigger_trend_threshold=1"));
        assert!(formatted.contains("governor_latest snapshots=2"));
        assert!(formatted.contains("trend_failure_ewma=0.250"));
        assert!(formatted.contains("rollup route_decisions=terminal:1"));
        assert!(formatted.contains("rollup route_reasons=session_governor_no_replan:1"));
        assert!(formatted.contains("rollup failure_codes=safe_lane_plan_verify_failed:1"));
    }
}
