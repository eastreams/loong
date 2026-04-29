use loong_app as mvp;
use serde_json::{Value, json};

pub(crate) async fn load_session_safe_lane_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
) -> Value {
    let summary_limit = runtime_truth_summary_limit(memory_config);
    let binding = mvp::conversation::ConversationRuntimeBinding::direct();
    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let summary_result = mvp::conversation::load_safe_lane_event_summary(
        session_id,
        summary_limit,
        binding,
        &session_store_config,
    )
    .await;

    match summary_result {
        Ok(summary) => {
            let available = summary != mvp::conversation::SafeLaneEventSummary::default();
            json!({
                "available": available,
                "limit": summary_limit,
                "summary": summary,
            })
        }
        Err(error) => json!({
            "available": false,
            "limit": summary_limit,
            "error": error,
        }),
    }
}

pub(crate) async fn load_session_turn_checkpoint_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
) -> Value {
    let summary_limit = runtime_truth_summary_limit(memory_config);
    let binding = mvp::conversation::ConversationRuntimeBinding::direct();
    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let summary_result = mvp::conversation::load_turn_checkpoint_event_summary(
        session_id,
        summary_limit,
        binding,
        &session_store_config,
    )
    .await;

    match summary_result {
        Ok(summary) => {
            let available = summary != mvp::conversation::TurnCheckpointEventSummary::default();
            json!({
                "available": available,
                "limit": summary_limit,
                "summary": summary,
            })
        }
        Err(error) => json!({
            "available": false,
            "limit": summary_limit,
            "error": error,
        }),
    }
}

pub(crate) fn render_safe_lane_summary(safe_lane: Option<&Value>) -> String {
    let Some(safe_lane) = safe_lane else {
        return "-".to_owned();
    };

    let available = safe_lane
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !available {
        let error = safe_lane
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("-");
        return format!("unavailable error={error}");
    }

    let Some(summary) = safe_lane.get("summary") else {
        return "present".to_owned();
    };

    let status = summary
        .get("final_status")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let rounds_started = summary
        .get("round_started_events")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_owned());
    let verify_failed = summary
        .get("verify_failed_events")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_owned());
    let replans = summary
        .get("replan_triggered_events")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_owned());
    let failure_code = summary
        .get("final_failure_code")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let route_decision = summary
        .get("final_route_decision")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let route_reason = summary
        .get("final_route_reason")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let health = summary
        .get("latest_health_signal")
        .and_then(|value| value.get("severity"))
        .and_then(Value::as_str)
        .unwrap_or("-");

    format!(
        "status={status} rounds_started={rounds_started} verify_failed={verify_failed} replans={replans} failure_code={failure_code} route={route_decision}/{route_reason} health={health}"
    )
}

pub(crate) fn render_turn_checkpoint_summary(turn_checkpoint: Option<&Value>) -> String {
    let Some(turn_checkpoint) = turn_checkpoint else {
        return "-".to_owned();
    };

    let available = turn_checkpoint
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !available {
        let error = turn_checkpoint
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("-");
        return format!("unavailable error={error}");
    }

    let Some(summary) = turn_checkpoint.get("summary") else {
        return "present".to_owned();
    };

    let session_state = summary
        .get("session_state")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let durable = summary
        .get("checkpoint_durable")
        .and_then(Value::as_bool)
        .map(render_bool_flag)
        .unwrap_or("-");
    let reply_durable = summary
        .get("reply_durable")
        .and_then(Value::as_bool)
        .map(render_bool_flag)
        .unwrap_or("-");
    let requires_recovery = summary
        .get("requires_recovery")
        .and_then(Value::as_bool)
        .map(render_bool_flag)
        .unwrap_or("-");
    let stage = summary
        .get("latest_stage")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let after_turn = summary
        .get("latest_after_turn")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let compaction = summary
        .get("latest_compaction")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let compaction_diagnostics = render_turn_checkpoint_compaction_diagnostics(summary);

    let mut rendered = format!(
        "session_state={session_state} durable={durable} reply_durable={reply_durable} requires_recovery={requires_recovery} stage={stage} after_turn={after_turn} compaction={compaction}"
    );
    if let Some(compaction_diagnostics) = compaction_diagnostics {
        rendered.push(' ');
        rendered.push_str(compaction_diagnostics.as_str());
    }
    rendered
}

fn render_turn_checkpoint_compaction_diagnostics(summary: &Value) -> Option<String> {
    let diagnostics = summary.get("latest_compaction_diagnostics")?.as_object()?;
    let summary_turns = diagnostics.get("summary_turn_count")?.as_u64()?;
    let retained_turns = diagnostics.get("retained_turn_count")?.as_u64()?;
    let demoted_recent_turns = diagnostics.get("demoted_recent_turn_count")?.as_u64()?;
    let low_signal_turns = diagnostics.get("low_signal_turns")?.as_u64()?;
    let tool_result_line_prunes = diagnostics.get("tool_result_line_prunes")?.as_u64()?;
    let tool_outcome_record_prunes = diagnostics.get("tool_outcome_record_prunes")?.as_u64()?;

    Some(format!(
        "compaction_diag=summary:{summary_turns} retained:{retained_turns} demoted:{demoted_recent_turns} low_signal:{low_signal_turns} tool_results:{tool_result_line_prunes} tool_outcomes:{tool_outcome_record_prunes}"
    ))
}

fn render_bool_flag(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn runtime_truth_summary_limit(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
) -> usize {
    let scaled_limit = memory_config.sliding_window.saturating_mul(4);
    scaled_limit.clamp(16, 128)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn render_turn_checkpoint_summary_includes_compaction_diagnostics_when_present() {
        let payload = json!({
            "available": true,
            "summary": {
                "session_state": "finalized",
                "checkpoint_durable": true,
                "reply_durable": true,
                "requires_recovery": false,
                "latest_stage": "finalized",
                "latest_after_turn": "completed",
                "latest_compaction": "failed_open",
                "latest_compaction_diagnostics": {
                    "summary_turn_count": 6,
                    "retained_turn_count": 3,
                    "demoted_recent_turn_count": 1,
                    "total_turns": 9,
                    "assistant_turns": 4,
                    "low_signal_turns": 2,
                    "tool_result_line_prunes": 1,
                    "tool_outcome_record_prunes": 0
                }
            }
        });

        let rendered = super::render_turn_checkpoint_summary(Some(&payload));

        assert!(rendered.contains("session_state=finalized"));
        assert!(rendered.contains("compaction=failed_open"));
        assert!(rendered.contains("compaction_diag=summary:6 retained:3 demoted:1"));
        assert!(rendered.contains("tool_results:1"));
    }

    #[test]
    fn render_turn_checkpoint_summary_omits_compaction_diagnostics_when_missing() {
        let payload = json!({
            "available": true,
            "summary": {
                "session_state": "finalized",
                "checkpoint_durable": true,
                "reply_durable": true,
                "requires_recovery": false,
                "latest_stage": "finalized",
                "latest_after_turn": "completed",
                "latest_compaction": "skipped"
            }
        });

        let rendered = super::render_turn_checkpoint_summary(Some(&payload));

        assert!(rendered.contains("compaction=skipped"));
        assert!(!rendered.contains("compaction_diag="));
    }
}
