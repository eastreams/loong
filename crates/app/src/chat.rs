#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;
use std::io::{self, Write};

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::Capability;

use crate::CliResult;
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context};

use super::config::{self, ConversationConfig, LoongClawConfig};
#[cfg(feature = "memory-sqlite")]
use super::conversation::summarize_safe_lane_events;
use super::conversation::{ConversationTurnCoordinator, ProviderErrorMode};
#[cfg(any(test, feature = "memory-sqlite"))]
use super::conversation::{SafeLaneEventSummary, SafeLaneFinalStatus};
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
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

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
            print_safe_lane_summary(&session_id, limit, &config.conversation, &memory_config)?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_safe_lane_summary(&session_id, limit, &config.conversation)?;
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

        let entries = memory::load_prompt_context(session_id, memory_config)
            .map_err(|error| format!("load history failed: {error}"))?;
        if entries.is_empty() {
            println!("(no history yet)");
            return Ok(());
        }
        for entry in entries {
            match entry.kind {
                memory::MemoryContextKind::Profile => {
                    println!("[profile]");
                    println!("{}", entry.content);
                }
                memory::MemoryContextKind::Summary => {
                    println!("[summary]");
                    println!("{}", entry.content);
                }
                memory::MemoryContextKind::Turn => {
                    println!("{}: {}", entry.role, entry.content);
                }
            }
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
    conversation_config: &ConversationConfig,
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
        println!(
            "{}",
            format_safe_lane_summary(session_id, limit, conversation_config, &summary)
        );
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, conversation_config);
        println!("safe-lane summary unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_safe_lane_summary(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
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
    let latest_tool_truncation_ratio = format_milli_ratio(
        summary
            .latest_tool_output
            .as_ref()
            .map(|snapshot| snapshot.truncation_ratio_milli),
    );
    let aggregate_tool_truncation_ratio_milli = summary
        .tool_output_aggregate_truncation_ratio_milli
        .or_else(|| {
            if summary.tool_output_result_lines_total == 0 {
                None
            } else {
                Some(
                    summary
                        .tool_output_truncated_result_lines_total
                        .saturating_mul(1000)
                        .saturating_div(summary.tool_output_result_lines_total)
                        .min(u32::MAX as u64) as u32,
                )
            }
        });
    let aggregate_tool_truncation_ratio =
        aggregate_tool_truncation_ratio_milli.map(|milli| (milli as f64) / 1000.0);
    let aggregate_tool_truncation_ratio_text = aggregate_tool_truncation_ratio
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "-".to_owned());
    let health_signal = derive_safe_lane_health_signal(
        conversation_config,
        summary,
        replan_rate,
        verify_failure_rate,
        aggregate_tool_truncation_ratio,
    );
    let health_flags = if health_signal.flags.is_empty() {
        "-".to_owned()
    } else {
        health_signal.flags.join(",")
    };
    let health_payload = serde_json::json!({
        "severity": health_signal.severity,
        "flags": health_signal.flags.clone(),
    })
    .to_string();
    let latest_health_event_severity = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| snapshot.severity.as_str())
        .unwrap_or("-");
    let latest_health_event_flags = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| {
            if snapshot.flags.is_empty() {
                "-".to_owned()
            } else {
                snapshot.flags.join(",")
            }
        })
        .unwrap_or_else(|| "-".to_owned());

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
        format!(
            "tool_output snapshots={} truncated_events={} result_lines_total={} truncated_result_lines_total={} latest_truncation_ratio={} aggregate_truncation_ratio={} aggregate_truncation_ratio_milli={} truncation_verify_failed_events={} truncation_replan_events={} truncation_final_failure_events={}",
            summary.tool_output_snapshots_seen,
            summary.tool_output_truncated_events,
            summary.tool_output_result_lines_total,
            summary.tool_output_truncated_result_lines_total,
            latest_tool_truncation_ratio,
            aggregate_tool_truncation_ratio_text
            ,
            aggregate_tool_truncation_ratio_milli
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary.tool_output_truncation_verify_failed_events,
            summary.tool_output_truncation_replan_events,
            summary.tool_output_truncation_final_failure_events
        ),
        format!(
            "health severity={} flags={health_flags}",
            health_signal.severity
        ),
        format!("health_payload {health_payload}"),
        format!(
            "health_events snapshots={} warn={} critical={} latest_severity={} latest_flags={}",
            summary.health_signal_snapshots_seen,
            summary.health_signal_warn_events,
            summary.health_signal_critical_events,
            latest_health_event_severity,
            latest_health_event_flags
        ),
        metrics_line,
        format!("rollup route_decisions={route_rollup}"),
        format!("rollup route_reasons={route_reason_rollup}"),
        format!("rollup failure_codes={failure_rollup}"),
    ]
    .join("\n")
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn derive_safe_lane_health_signal(
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
    replan_rate: f64,
    verify_failure_rate: f64,
    aggregate_truncation_ratio: Option<f64>,
) -> SafeLaneHealthSignal {
    let mut flags = Vec::new();
    let mut has_critical = false;
    let truncation_warn_threshold =
        conversation_config.safe_lane_health_truncation_warn_threshold();
    let truncation_critical_threshold =
        conversation_config.safe_lane_health_truncation_critical_threshold();
    let verify_failure_warn_threshold =
        conversation_config.safe_lane_health_verify_failure_warn_threshold();
    let replan_warn_threshold = conversation_config.safe_lane_health_replan_warn_threshold();

    if let Some(ratio) = aggregate_truncation_ratio {
        if ratio >= truncation_critical_threshold {
            flags.push(format!("truncation_severe({ratio:.3})"));
            has_critical = true;
        } else if ratio >= truncation_warn_threshold {
            flags.push(format!("truncation_pressure({ratio:.3})"));
        }
    }
    if verify_failure_rate >= verify_failure_warn_threshold {
        flags.push(format!("verify_failure_pressure({verify_failure_rate:.3})"));
    }
    if replan_rate >= replan_warn_threshold {
        flags.push(format!("replan_pressure({replan_rate:.3})"));
    }
    let terminal_instability = matches!(summary.final_status, Some(SafeLaneFinalStatus::Failed))
        && summary
            .final_failure_code
            .as_deref()
            .map(|code| {
                code.contains("verify_failed")
                    || code.contains("backpressure")
                    || code.contains("session_governor")
            })
            .unwrap_or(false);
    if terminal_instability {
        flags.push("terminal_instability".to_owned());
        has_critical = true;
    }

    SafeLaneHealthSignal {
        severity: if has_critical {
            "critical"
        } else if flags.is_empty() {
            "ok"
        } else {
            "warn"
        },
        flags,
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeLaneHealthSignal {
    severity: &'static str,
    flags: Vec<String>,
}

#[cfg(any(test, feature = "memory-sqlite"))]
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

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_milli_ratio(value: Option<u32>) -> String {
    value
        .map(|raw| format!("{:.3}", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}

fn export_runtime_env(config: &LoongClawConfig) {
    crate::memory::runtime_config::apply_memory_runtime_env(&config.memory);
    crate::process_env::set_var(
        "LOONGCLAW_SHELL_ALLOWLIST",
        config.tools.shell_allowlist.join(","),
    );
    crate::process_env::set_var(
        "LOONGCLAW_FILE_ROOT",
        config.tools.resolved_file_root().display().to_string(),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_ENABLED",
        config.external_skills.enabled.to_string(),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
        config.external_skills.require_download_approval.to_string(),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
        config
            .external_skills
            .normalized_allowed_domains()
            .join(","),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
        config
            .external_skills
            .normalized_blocked_domains()
            .join(","),
    );
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
        external_skills: crate::tools::runtime_config::ExternalSkillsRuntimePolicy {
            enabled: config.external_skills.enabled,
            require_download_approval: config.external_skills.require_download_approval,
            allowed_domains: config
                .external_skills
                .normalized_allowed_domains()
                .into_iter()
                .collect(),
            blocked_domains: config
                .external_skills
                .normalized_blocked_domains()
                .into_iter()
                .collect(),
        },
    };
    let _ = crate::tools::runtime_config::init_tool_runtime_config(tool_rt);

    // Populate the typed memory runtime config (same pattern as tool config).
    let memory_rt =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
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
        let config = ConversationConfig::default();
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
            latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
                output_lines: 2,
                result_lines: 2,
                truncated_result_lines: 1,
                any_truncated: true,
                truncation_ratio_milli: 500,
            }),
            tool_output_snapshots_seen: 2,
            tool_output_truncated_events: 1,
            tool_output_result_lines_total: 3,
            tool_output_truncated_result_lines_total: 1,
            tool_output_aggregate_truncation_ratio_milli: Some(333),
            tool_output_truncation_verify_failed_events: 1,
            tool_output_truncation_replan_events: 1,
            tool_output_truncation_final_failure_events: 1,
            latest_health_signal: Some(crate::conversation::SafeLaneHealthSignalSnapshot {
                severity: "critical".to_owned(),
                flags: vec!["terminal_instability".to_owned()],
            }),
            health_signal_snapshots_seen: 2,
            health_signal_warn_events: 1,
            health_signal_critical_events: 1,
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
        let formatted = format_safe_lane_summary("session-a", 128, &config, &summary);

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
        assert!(formatted.contains(
            "tool_output snapshots=2 truncated_events=1 result_lines_total=3 truncated_result_lines_total=1"
        ));
        assert!(formatted.contains("latest_truncation_ratio=0.500"));
        assert!(formatted.contains("aggregate_truncation_ratio=0.333"));
        assert!(formatted.contains("aggregate_truncation_ratio_milli=333"));
        assert!(formatted.contains("truncation_verify_failed_events=1"));
        assert!(formatted.contains("truncation_replan_events=1"));
        assert!(formatted.contains("truncation_final_failure_events=1"));
        assert!(formatted.contains("health severity=critical"));
        assert!(formatted.contains("health_payload {\"flags\":"));
        assert!(formatted.contains("\"severity\":\"critical\""));
        assert!(formatted.contains(
            "health_events snapshots=2 warn=1 critical=1 latest_severity=critical latest_flags=terminal_instability"
        ));
        assert!(formatted.contains("truncation_pressure(0.333)"));
        assert!(formatted.contains("verify_failure_pressure(0.500)"));
        assert!(formatted.contains("replan_pressure(0.500)"));
        assert!(formatted.contains("terminal_instability"));
        assert!(formatted.contains("rollup route_decisions=terminal:1"));
        assert!(formatted.contains("rollup route_reasons=session_governor_no_replan:1"));
        assert!(formatted.contains("rollup failure_codes=safe_lane_plan_verify_failed:1"));
    }

    #[test]
    fn format_safe_lane_summary_health_is_ok_when_no_risk_signals() {
        let config = ConversationConfig::default();
        let summary = SafeLaneEventSummary {
            lane_selected_events: 1,
            round_started_events: 3,
            final_status_events: 1,
            final_status: Some(SafeLaneFinalStatus::Succeeded),
            latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
                rounds_started: 3,
                rounds_succeeded: 3,
                rounds_failed: 0,
                verify_failures: 0,
                replans_triggered: 0,
                total_attempts_used: 3,
            }),
            tool_output_snapshots_seen: 1,
            tool_output_result_lines_total: 2,
            tool_output_truncated_result_lines_total: 0,
            latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
                output_lines: 2,
                result_lines: 2,
                truncated_result_lines: 0,
                any_truncated: false,
                truncation_ratio_milli: 0,
            }),
            ..SafeLaneEventSummary::default()
        };
        let formatted = format_safe_lane_summary("session-ok", 64, &config, &summary);
        assert!(formatted.contains("health severity=ok flags=-"));
        assert!(formatted.contains("health_payload {\"flags\":[],\"severity\":\"ok\"}"));
        assert!(formatted.contains(
            "health_events snapshots=0 warn=0 critical=0 latest_severity=- latest_flags=-"
        ));
    }

    #[test]
    fn format_safe_lane_summary_respects_configurable_health_thresholds() {
        let config = ConversationConfig {
            safe_lane_health_truncation_warn_threshold: 0.20,
            safe_lane_health_truncation_critical_threshold: 0.50,
            safe_lane_health_verify_failure_warn_threshold: 0.70,
            safe_lane_health_replan_warn_threshold: 0.70,
            ..ConversationConfig::default()
        };
        let summary = SafeLaneEventSummary {
            round_started_events: 4,
            verify_failed_events: 1,
            replan_triggered_events: 1,
            tool_output_snapshots_seen: 1,
            tool_output_result_lines_total: 4,
            tool_output_truncated_result_lines_total: 1,
            tool_output_aggregate_truncation_ratio_milli: Some(250),
            latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
                output_lines: 4,
                result_lines: 4,
                truncated_result_lines: 1,
                any_truncated: true,
                truncation_ratio_milli: 250,
            }),
            ..SafeLaneEventSummary::default()
        };

        let formatted = format_safe_lane_summary("session-threshold", 32, &config, &summary);
        assert!(formatted.contains("health severity=warn"));
        assert!(formatted.contains("truncation_pressure(0.250)"));
        assert!(!formatted.contains("verify_failure_pressure"));
        assert!(!formatted.contains("replan_pressure"));
    }
}
