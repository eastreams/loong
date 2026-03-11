use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLaneFinalStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneMetricsSnapshot {
    pub rounds_started: u32,
    pub rounds_succeeded: u32,
    pub rounds_failed: u32,
    pub verify_failures: u32,
    pub replans_triggered: u32,
    pub total_attempts_used: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneEventSummary {
    pub lane_selected_events: u32,
    pub round_started_events: u32,
    pub round_completed_succeeded_events: u32,
    pub round_completed_failed_events: u32,
    pub verify_failed_events: u32,
    pub verify_policy_adjusted_events: u32,
    pub replan_triggered_events: u32,
    pub final_status_events: u32,
    pub session_governor_engaged_events: u32,
    pub session_governor_force_no_replan_events: u32,
    pub session_governor_failed_threshold_triggered_events: u32,
    pub session_governor_backpressure_threshold_triggered_events: u32,
    pub session_governor_trend_threshold_triggered_events: u32,
    pub session_governor_recovery_threshold_triggered_events: u32,
    pub session_governor_metrics_snapshots_seen: u32,
    pub session_governor_latest_trend_samples: Option<u32>,
    pub session_governor_latest_trend_min_samples: Option<u32>,
    pub session_governor_latest_trend_failure_ewma_milli: Option<u32>,
    pub session_governor_latest_trend_backpressure_ewma_milli: Option<u32>,
    pub session_governor_latest_recovery_success_streak: Option<u32>,
    pub session_governor_latest_recovery_success_streak_threshold: Option<u32>,
    pub final_status: Option<SafeLaneFinalStatus>,
    pub final_failure_code: Option<String>,
    pub final_route_decision: Option<String>,
    pub final_route_reason: Option<String>,
    pub latest_metrics: Option<SafeLaneMetricsSnapshot>,
    pub metrics_snapshots_seen: u32,
    pub route_decision_counts: BTreeMap<String, u32>,
    pub route_reason_counts: BTreeMap<String, u32>,
    pub failure_code_counts: BTreeMap<String, u32>,
    pub final_status_counts: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationEventRecord {
    pub event: String,
    pub payload: Value,
}

pub fn parse_conversation_event(content: &str) -> Option<ConversationEventRecord> {
    let parsed = serde_json::from_str::<Value>(content).ok()?;
    if parsed.get("type")?.as_str()? != "conversation_event" {
        return None;
    }
    let event = parsed.get("event")?.as_str()?.to_owned();
    let payload = parsed.get("payload").cloned().unwrap_or(Value::Null);
    Some(ConversationEventRecord { event, payload })
}

pub fn summarize_safe_lane_events<'a, I>(contents: I) -> SafeLaneEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    let mut summary = SafeLaneEventSummary::default();

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        if !is_safe_lane_event_name(record.event.as_str()) {
            continue;
        }

        match record.event.as_str() {
            "lane_selected" => {
                summary.lane_selected_events = summary.lane_selected_events.saturating_add(1);
            }
            "plan_round_started" => {
                summary.round_started_events = summary.round_started_events.saturating_add(1);
            }
            "plan_round_completed" => {
                let is_succeeded = record
                    .payload
                    .get("status")
                    .and_then(Value::as_str)
                    .map(|status| status == "succeeded")
                    .unwrap_or(false);
                if is_succeeded {
                    summary.round_completed_succeeded_events =
                        summary.round_completed_succeeded_events.saturating_add(1);
                } else {
                    summary.round_completed_failed_events =
                        summary.round_completed_failed_events.saturating_add(1);
                }
            }
            "verify_failed" => {
                summary.verify_failed_events = summary.verify_failed_events.saturating_add(1);
            }
            "verify_policy_adjusted" => {
                summary.verify_policy_adjusted_events =
                    summary.verify_policy_adjusted_events.saturating_add(1);
            }
            "replan_triggered" => {
                summary.replan_triggered_events = summary.replan_triggered_events.saturating_add(1);
            }
            "final_status" => {
                summary.final_status_events = summary.final_status_events.saturating_add(1);
                match record.payload.get("status").and_then(Value::as_str) {
                    Some("succeeded") => {
                        summary.final_status = Some(SafeLaneFinalStatus::Succeeded);
                        bump_count(&mut summary.final_status_counts, "succeeded");
                    }
                    Some("failed") => {
                        summary.final_status = Some(SafeLaneFinalStatus::Failed);
                        bump_count(&mut summary.final_status_counts, "failed");
                    }
                    _ => {}
                }
                summary.final_failure_code = record
                    .payload
                    .get("failure_code")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                summary.final_route_decision = record
                    .payload
                    .get("route_decision")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                summary.final_route_reason = record
                    .payload
                    .get("route_reason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            _ => {}
        }

        if let Some(route_decision) = record
            .payload
            .get("route_decision")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            bump_count(&mut summary.route_decision_counts, route_decision);
        }
        if let Some(failure_code) = record
            .payload
            .get("failure_code")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            bump_count(&mut summary.failure_code_counts, failure_code);
        }
        if let Some(route_reason) = record
            .payload
            .get("route_reason")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            bump_count(&mut summary.route_reason_counts, route_reason);
        }
        fold_session_governor_summary(record.payload.get("session_governor"), &mut summary);

        if let Some(metrics) = parse_metrics_snapshot(record.payload.get("metrics")) {
            summary.metrics_snapshots_seen = summary.metrics_snapshots_seen.saturating_add(1);
            summary.latest_metrics = Some(metrics);
        }
    }

    summary
}

fn parse_metrics_snapshot(value: Option<&Value>) -> Option<SafeLaneMetricsSnapshot> {
    let metrics = value?;
    let has_any = [
        "rounds_started",
        "rounds_succeeded",
        "rounds_failed",
        "verify_failures",
        "replans_triggered",
        "total_attempts_used",
    ]
    .iter()
    .any(|key| metrics.get(*key).is_some());
    if !has_any {
        return None;
    }

    Some(SafeLaneMetricsSnapshot {
        rounds_started: read_u32(metrics, "rounds_started"),
        rounds_succeeded: read_u32(metrics, "rounds_succeeded"),
        rounds_failed: read_u32(metrics, "rounds_failed"),
        verify_failures: read_u32(metrics, "verify_failures"),
        replans_triggered: read_u32(metrics, "replans_triggered"),
        total_attempts_used: metrics
            .get("total_attempts_used")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    })
}

fn is_safe_lane_event_name(event_name: &str) -> bool {
    matches!(
        event_name,
        "lane_selected"
            | "plan_round_started"
            | "plan_round_completed"
            | "verify_failed"
            | "verify_policy_adjusted"
            | "replan_triggered"
            | "final_status"
    )
}

fn read_u32(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|num| num.min(u32::MAX as u64) as u32)
        .unwrap_or_default()
}

fn bump_count(map: &mut BTreeMap<String, u32>, key: &str) {
    let entry = map.entry(key.to_owned()).or_insert(0);
    *entry = entry.saturating_add(1);
}

fn fold_session_governor_summary(
    session_governor: Option<&Value>,
    summary: &mut SafeLaneEventSummary,
) {
    let Some(governor) = session_governor else {
        return;
    };
    summary.session_governor_metrics_snapshots_seen = summary
        .session_governor_metrics_snapshots_seen
        .saturating_add(1);

    if governor
        .get("engaged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_engaged_events =
            summary.session_governor_engaged_events.saturating_add(1);
    }
    if governor
        .get("force_no_replan")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_force_no_replan_events = summary
            .session_governor_force_no_replan_events
            .saturating_add(1);
    }
    if governor
        .get("failed_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_failed_threshold_triggered_events = summary
            .session_governor_failed_threshold_triggered_events
            .saturating_add(1);
    }
    if governor
        .get("backpressure_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_backpressure_threshold_triggered_events = summary
            .session_governor_backpressure_threshold_triggered_events
            .saturating_add(1);
    }
    if governor
        .get("trend_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_trend_threshold_triggered_events = summary
            .session_governor_trend_threshold_triggered_events
            .saturating_add(1);
    }
    if governor
        .get("recovery_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_recovery_threshold_triggered_events = summary
            .session_governor_recovery_threshold_triggered_events
            .saturating_add(1);
    }

    summary.session_governor_latest_trend_samples = read_u32_opt(governor, "trend_samples");
    summary.session_governor_latest_trend_min_samples = read_u32_opt(governor, "trend_min_samples");
    summary.session_governor_latest_trend_failure_ewma_milli =
        read_f64_milli_opt(governor, "trend_failure_ewma");
    summary.session_governor_latest_trend_backpressure_ewma_milli =
        read_f64_milli_opt(governor, "trend_backpressure_ewma");
    summary.session_governor_latest_recovery_success_streak =
        read_u32_opt(governor, "recovery_success_streak");
    summary.session_governor_latest_recovery_success_streak_threshold =
        read_u32_opt(governor, "recovery_success_streak_threshold");
}

fn read_u32_opt(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|num| num.min(u32::MAX as u64) as u32)
}

fn read_f64_milli_opt(value: &Value, key: &str) -> Option<u32> {
    let raw = value.get(key)?.as_f64()?;
    if !raw.is_finite() {
        return None;
    }
    let clamped = raw.clamp(0.0, 1.0);
    let milli = (clamped * 1000.0).round();
    Some(milli.min(u32::MAX as f64) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_conversation_event_rejects_non_event_payloads() {
        assert!(parse_conversation_event("not-json").is_none());
        assert!(parse_conversation_event(r#"{"type":"tool_outcome"}"#).is_none());
    }

    #[test]
    fn summarize_safe_lane_events_counts_and_final_fields() {
        let payloads = [
            r#"{"type":"conversation_event","event":"lane_selected","payload":{"lane":"safe"}}"#,
            r#"{"type":"conversation_event","event":"plan_round_started","payload":{"round":0}}"#,
            r#"{"type":"conversation_event","event":"plan_round_completed","payload":{"round":0,"status":"failed"}}"#,
            r#"{"type":"conversation_event","event":"verify_policy_adjusted","payload":{"round":0,"min_anchor_matches":1}}"#,
            r#"{"type":"conversation_event","event":"replan_triggered","payload":{"round":0}}"#,
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_verify_failed","route_decision":"terminal"}}"#,
        ];
        let summary = summarize_safe_lane_events(payloads.iter().copied());

        assert_eq!(summary.lane_selected_events, 1);
        assert_eq!(summary.round_started_events, 1);
        assert_eq!(summary.round_completed_failed_events, 1);
        assert_eq!(summary.verify_policy_adjusted_events, 1);
        assert_eq!(summary.replan_triggered_events, 1);
        assert_eq!(summary.final_status_events, 1);
        assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Failed));
        assert_eq!(
            summary.final_failure_code.as_deref(),
            Some("safe_lane_plan_verify_failed")
        );
        assert_eq!(summary.final_route_decision.as_deref(), Some("terminal"));
        assert_eq!(
            summary.route_decision_counts.get("terminal").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .failure_code_counts
                .get("safe_lane_plan_verify_failed")
                .copied(),
            Some(1)
        );
        assert_eq!(summary.final_status_counts.get("failed").copied(), Some(1));
    }

    #[test]
    fn summarize_safe_lane_events_tracks_latest_metrics_snapshot() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "plan_round_started",
                "payload": {
                    "round": 0,
                    "metrics": {
                        "rounds_started": 1,
                        "rounds_succeeded": 0,
                        "rounds_failed": 0,
                        "verify_failures": 0,
                        "replans_triggered": 0,
                        "total_attempts_used": 0
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "succeeded",
                    "metrics": {
                        "rounds_started": 2,
                        "rounds_succeeded": 1,
                        "rounds_failed": 1,
                        "verify_failures": 0,
                        "replans_triggered": 1,
                        "total_attempts_used": 4
                    }
                }
            })
            .to_string(),
        ];
        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        let metrics = summary.latest_metrics.expect("latest metrics");
        assert_eq!(
            metrics,
            SafeLaneMetricsSnapshot {
                rounds_started: 2,
                rounds_succeeded: 1,
                rounds_failed: 1,
                verify_failures: 0,
                replans_triggered: 1,
                total_attempts_used: 4,
            }
        );
        assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Succeeded));
        assert_eq!(summary.metrics_snapshots_seen, 2);
        assert_eq!(
            summary.final_status_counts.get("succeeded").copied(),
            Some(1)
        );
    }

    #[test]
    fn summarize_safe_lane_events_accepts_partial_metrics_payload() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "verify_failed",
            "payload": {
                "round": 1,
                "failure_code": "safe_lane_plan_verify_failed",
                "metrics": {
                    "verify_failures": 2
                }
            }
        })
        .to_string()];
        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        let metrics = summary.latest_metrics.expect("latest metrics");
        assert_eq!(metrics.verify_failures, 2);
        assert_eq!(metrics.rounds_started, 0);
        assert_eq!(metrics.total_attempts_used, 0);
        assert_eq!(summary.metrics_snapshots_seen, 1);
    }

    #[test]
    fn summarize_safe_lane_events_handles_sparse_sampled_stream() {
        let payloads = [
            r#"{"type":"conversation_event","event":"lane_selected","payload":{"lane":"safe"}}"#,
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_node_retryable_error","route_decision":"terminal","route_reason":"session_governor_no_replan"}}"#,
        ];
        let summary = summarize_safe_lane_events(payloads.iter().copied());
        assert_eq!(summary.lane_selected_events, 1);
        assert_eq!(summary.round_started_events, 0);
        assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Failed));
        assert_eq!(
            summary
                .failure_code_counts
                .get("safe_lane_plan_node_retryable_error")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary.route_decision_counts.get("terminal").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .route_reason_counts
                .get("session_governor_no_replan")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary.final_route_reason.as_deref(),
            Some("session_governor_no_replan")
        );
    }

    #[test]
    fn summarize_safe_lane_events_tracks_session_governor_signals() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "lane_selected",
                "payload": {
                    "lane": "safe",
                    "session_governor": {
                        "engaged": true,
                        "force_no_replan": true,
                        "failed_threshold_triggered": true,
                        "backpressure_threshold_triggered": false,
                        "trend_threshold_triggered": true,
                        "recovery_threshold_triggered": false,
                        "trend_samples": 4,
                        "trend_min_samples": 4,
                        "trend_failure_ewma": 0.688,
                        "trend_backpressure_ewma": 0.000,
                        "recovery_success_streak": 0,
                        "recovery_success_streak_threshold": 3
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "plan_round_started",
                "payload": {
                    "round": 0,
                    "session_governor": {
                        "engaged": true,
                        "force_no_replan": true,
                        "failed_threshold_triggered": true,
                        "backpressure_threshold_triggered": false,
                        "trend_threshold_triggered": false,
                        "recovery_threshold_triggered": true,
                        "trend_samples": 5,
                        "trend_min_samples": 4,
                        "trend_failure_ewma": 0.250,
                        "trend_backpressure_ewma": 0.063,
                        "recovery_success_streak": 4,
                        "recovery_success_streak_threshold": 3
                    }
                }
            })
            .to_string(),
        ];

        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.session_governor_engaged_events, 2);
        assert_eq!(summary.session_governor_force_no_replan_events, 2);
        assert_eq!(
            summary.session_governor_failed_threshold_triggered_events,
            2
        );
        assert_eq!(
            summary.session_governor_backpressure_threshold_triggered_events,
            0
        );
        assert_eq!(summary.session_governor_trend_threshold_triggered_events, 1);
        assert_eq!(
            summary.session_governor_recovery_threshold_triggered_events,
            1
        );
        assert_eq!(summary.session_governor_metrics_snapshots_seen, 2);
        assert_eq!(summary.session_governor_latest_trend_samples, Some(5));
        assert_eq!(summary.session_governor_latest_trend_min_samples, Some(4));
        assert_eq!(
            summary.session_governor_latest_trend_failure_ewma_milli,
            Some(250)
        );
        assert_eq!(
            summary.session_governor_latest_trend_backpressure_ewma_milli,
            Some(63)
        );
        assert_eq!(
            summary.session_governor_latest_recovery_success_streak,
            Some(4)
        );
        assert_eq!(
            summary.session_governor_latest_recovery_success_streak_threshold,
            Some(3)
        );
    }
}
