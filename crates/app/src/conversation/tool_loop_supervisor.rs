use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{Map, Value, json};

use super::turn_engine::{ApprovalRequirement, ToolIntent, TurnResult};

const MAX_RECENT_ROUNDS: usize = 24;
const VOLATILE_TOOL_RESULT_ENVELOPE_FIELDS: &[&str] = &[
    "tool_call_id",
    "approval_request_id",
    "lease",
    "expires_at_unix",
    "token_id",
    "session_id",
    "turn_id",
];
const VOLATILE_DISCOVERY_ENTRY_FIELDS: &[&str] = &["lease", "expires_at_unix"];

#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolLoopSupervisorPolicy {
    pub(crate) max_repeated_tool_call_rounds: usize,
    pub(crate) max_ping_pong_cycles: usize,
    pub(crate) max_same_tool_failure_rounds: usize,
    pub(crate) max_consecutive_same_tool: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolLoopSupervisorVerdict {
    Continue,
    InjectWarning { reason: String },
    HardStop { reason: String },
}

#[derive(Debug, Clone)]
pub(crate) struct ToolLoopRoundOutcome {
    pub(crate) fingerprint: String,
    pub(crate) failed: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolLoopSupervisor {
    last_pattern: Option<String>,
    last_pattern_streak: usize,
    warned_reason_key: Option<String>,
    recent_rounds: VecDeque<ToolLoopObservation>,
    consecutive_same_tool: usize,
    last_tool_signature: Option<String>,
}

#[derive(Debug, Clone)]
struct ToolLoopObservation {
    pattern: String,
    tool_name_signature: String,
    failed: bool,
}

#[derive(Debug, Clone)]
struct LoopDetectionReason {
    key: String,
    text: String,
}

impl ToolLoopSupervisor {
    pub(crate) fn observe_round(
        &mut self,
        policy: &ToolLoopSupervisorPolicy,
        tool_signature: &str,
        tool_name_signature: &str,
        outcome_fingerprint: &str,
        failed: bool,
    ) -> ToolLoopSupervisorVerdict {
        let same_tool_verdict = self.observe_same_tool(policy, tool_signature, tool_name_signature);
        let pattern = format!("{tool_signature}::{outcome_fingerprint}");
        self.observe_pattern(tool_name_signature, pattern, failed);

        if let Some(reason) = same_tool_verdict {
            return self.verdict_for_reason(reason);
        }

        let no_progress = self.check_no_progress(policy.max_repeated_tool_call_rounds);
        let ping_pong = self.check_ping_pong(policy.max_ping_pong_cycles);
        let failure_streak = self.check_failure_streak(policy.max_same_tool_failure_rounds);
        let detection = no_progress.or(ping_pong).or(failure_streak);

        match detection {
            Some(reason) => self.verdict_for_reason(reason),
            None => {
                self.warned_reason_key = None;
                ToolLoopSupervisorVerdict::Continue
            }
        }
    }

    pub(crate) fn clear_pending_warning(&mut self) {
        self.warned_reason_key = None;
    }

    fn observe_same_tool(
        &mut self,
        policy: &ToolLoopSupervisorPolicy,
        tool_signature: &str,
        tool_name_signature: &str,
    ) -> Option<LoopDetectionReason> {
        if self.last_tool_signature.as_deref() == Some(tool_signature) {
            self.consecutive_same_tool += 1;
        } else {
            self.last_tool_signature = Some(tool_signature.to_owned());
            self.consecutive_same_tool = 1;
        }

        if self.consecutive_same_tool < policy.max_consecutive_same_tool {
            return None;
        }

        Some(LoopDetectionReason {
            key: format!("consecutive_same_tool:{tool_signature}"),
            text: format!(
                "consecutive_same_tool_call: {tool_name_signature} repeated {} times \
                 with identical arguments \
                 (limit={})",
                self.consecutive_same_tool, policy.max_consecutive_same_tool
            ),
        })
    }

    fn observe_pattern(&mut self, tool_name_signature: &str, pattern: String, failed: bool) {
        if self.last_pattern.as_deref() == Some(pattern.as_str()) {
            self.last_pattern_streak += 1;
        } else {
            self.last_pattern = Some(pattern.clone());
            self.last_pattern_streak = 1;
        }

        self.recent_rounds.push_back(ToolLoopObservation {
            pattern,
            tool_name_signature: tool_name_signature.to_owned(),
            failed,
        });

        if self.recent_rounds.len() > MAX_RECENT_ROUNDS {
            self.recent_rounds.pop_front();
        }
    }

    fn verdict_for_reason(&mut self, reason: LoopDetectionReason) -> ToolLoopSupervisorVerdict {
        if self.warned_reason_key.as_deref() == Some(reason.key.as_str()) {
            return ToolLoopSupervisorVerdict::HardStop {
                reason: reason.text,
            };
        }

        self.warned_reason_key = Some(reason.key);
        ToolLoopSupervisorVerdict::InjectWarning {
            reason: reason.text,
        }
    }

    fn check_no_progress(&self, threshold: usize) -> Option<LoopDetectionReason> {
        let pattern = self.last_pattern.as_deref()?;
        if self.last_pattern_streak <= threshold {
            return None;
        }

        Some(LoopDetectionReason {
            key: format!("no_progress:{pattern}"),
            text: format!(
                "repeated_tool_call_no_progress signature_streak={} threshold={threshold}",
                self.last_pattern_streak
            ),
        })
    }

    fn check_ping_pong(&self, cycles: usize) -> Option<LoopDetectionReason> {
        let minimum_rounds = cycles.saturating_mul(2);
        if cycles == 0 || self.recent_rounds.len() < minimum_rounds {
            return None;
        }

        let tail = self
            .recent_rounds
            .iter()
            .rev()
            .take(minimum_rounds)
            .collect::<Vec<_>>();
        let first = tail.first()?.pattern.as_str();
        let second = tail.get(1)?.pattern.as_str();
        if first == second {
            return None;
        }

        let alternating = tail.iter().enumerate().all(|(index, round)| {
            if index % 2 == 0 {
                round.pattern == first
            } else {
                round.pattern == second
            }
        });
        if !alternating {
            return None;
        }

        let (left, right) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };

        Some(LoopDetectionReason {
            key: format!("ping_pong:{left}<->{right}"),
            text: format!(
                "ping_pong_tool_patterns cycles={} threshold={cycles}",
                minimum_rounds / 2
            ),
        })
    }

    fn check_failure_streak(&self, threshold: usize) -> Option<LoopDetectionReason> {
        let last = self.recent_rounds.back()?;
        if !last.failed {
            return None;
        }

        let streak = self
            .recent_rounds
            .iter()
            .rev()
            .take_while(|round| {
                round.failed && round.tool_name_signature == last.tool_name_signature
            })
            .count();

        if streak < threshold {
            return None;
        }

        Some(LoopDetectionReason {
            key: format!("failure_streak:{}", last.tool_name_signature),
            text: format!(
                "tool_failure_streak rounds={streak} threshold={threshold} tool={}",
                last.tool_name_signature
            ),
        })
    }
}

pub(crate) fn tool_loop_round_outcome(turn_result: &TurnResult) -> Option<ToolLoopRoundOutcome> {
    match turn_result {
        TurnResult::FinalText(text)
        | TurnResult::StreamingText(text)
        | TurnResult::StreamingDone(text) => Some(ToolLoopRoundOutcome {
            fingerprint: text_fingerprint("tool_final_text", text),
            failed: false,
        }),
        TurnResult::NeedsApproval(requirement) => Some(ToolLoopRoundOutcome {
            fingerprint: approval_fingerprint(requirement),
            failed: false,
        }),
        TurnResult::ToolDenied(failure) => {
            let payload = failure_fingerprint_payload(failure);
            Some(ToolLoopRoundOutcome {
                fingerprint: value_fingerprint("tool_denied", payload),
                failed: true,
            })
        }
        TurnResult::ToolError(failure) => {
            let payload = failure_fingerprint_payload(failure);
            Some(ToolLoopRoundOutcome {
                fingerprint: value_fingerprint("tool_error", payload),
                failed: true,
            })
        }
        TurnResult::ProviderError(_) => None,
    }
}

pub(crate) fn tool_intent_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| {
            let args = serde_json::to_string(&intent.args_json)
                .unwrap_or_else(|_| "<invalid_tool_args_json>".to_owned());
            format!("{}:{args}", intent.tool_name.trim())
        })
        .collect::<Vec<_>>()
        .join("||")
}

pub(crate) fn tool_name_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| intent.tool_name.trim())
        .collect::<Vec<_>>()
        .join("||")
}

fn approval_fingerprint(requirement: &ApprovalRequirement) -> String {
    let payload = json!({
        "kind": requirement.kind,
        "reason": requirement.reason,
        "rule_id": requirement.rule_id,
        "tool_name": requirement.tool_name,
        "approval_key": requirement.approval_key,
    });

    value_fingerprint("tool_approval_required", payload)
}

fn failure_fingerprint_payload(failure: &super::turn_engine::TurnFailure) -> Value {
    json!({
        "kind": failure.kind,
        "code": failure.code,
        "reason": failure.reason,
        "retryable": failure.retryable,
        "supports_discovery_recovery": failure.supports_discovery_recovery,
    })
}

fn text_fingerprint(label: &str, text: &str) -> String {
    let normalized = normalize_tool_result_text(text);
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let digest = hasher.finish();
    format!("{label}:{digest:016x}")
}

fn value_fingerprint(label: &str, value: Value) -> String {
    let normalized = normalize_tool_result_value(value);
    let normalized_text = serde_json::to_string(&normalized).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    normalized_text.hash(&mut hasher);
    let digest = hasher.finish();
    format!("{label}:{digest:016x}")
}

fn normalize_tool_result_text(text: &str) -> String {
    let trimmed = text.trim();
    let parsed_value = serde_json::from_str::<Value>(trimmed);

    match parsed_value {
        Ok(value) => {
            let normalized_value = normalize_tool_result_value(value);
            serde_json::to_string(&normalized_value).unwrap_or_else(|_| trimmed.to_owned())
        }
        Err(_) => normalize_status_prefixed_tool_result(trimmed),
    }
}

fn normalize_status_prefixed_tool_result(text: &str) -> String {
    let Some(json_start) = text.find('{') else {
        return text.to_owned();
    };
    let status_prefix = text[..json_start].trim();
    let json_text = text[json_start..].trim();
    let parsed_value = serde_json::from_str::<Value>(json_text);

    match parsed_value {
        Ok(value) => {
            let normalized_value = normalize_tool_result_value(value);
            let normalized_text = serde_json::to_string(&normalized_value);
            let normalized_json = normalized_text.unwrap_or_else(|_| json_text.to_owned());
            format!("{status_prefix} {normalized_json}")
        }
        Err(_) => text.to_owned(),
    }
}

fn normalize_tool_result_value(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            let normalized_items = items
                .into_iter()
                .map(normalize_nested_result_value)
                .collect::<Vec<_>>();
            Value::Array(normalized_items)
        }
        Value::Object(entries) => normalize_tool_result_envelope_object(entries),
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_) => other,
    }
}

fn normalize_tool_result_envelope_object(entries: Map<String, Value>) -> Value {
    let result_is_envelope = is_tool_result_envelope(&entries);
    let mut normalized_entries = Map::new();

    for (key, value) in entries {
        let key_is_volatile = VOLATILE_TOOL_RESULT_ENVELOPE_FIELDS.contains(&key.as_str());
        if result_is_envelope && key_is_volatile {
            continue;
        }

        let normalized_value = if key == "payload_summary" {
            normalize_payload_summary_value(value)
        } else {
            normalize_nested_result_value(value)
        };
        normalized_entries.insert(key, normalized_value);
    }

    Value::Object(normalized_entries)
}

fn is_tool_result_envelope(entries: &Map<String, Value>) -> bool {
    if entries.contains_key("payload_summary") {
        return true;
    }

    let has_status = entries.contains_key("status");
    let has_tool_name = entries.contains_key("tool");
    let has_tool_call_id = entries.contains_key("tool_call_id");
    has_status && (has_tool_name || has_tool_call_id)
}

fn normalize_nested_result_value(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            let normalized_items = items
                .into_iter()
                .map(normalize_nested_result_value)
                .collect::<Vec<_>>();
            Value::Array(normalized_items)
        }
        Value::Object(entries) => normalize_nested_result_object(entries),
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_) => other,
    }
}

fn normalize_nested_result_object(entries: Map<String, Value>) -> Value {
    let mut normalized_entries = Map::new();

    for (key, value) in entries {
        let normalized_value = normalize_nested_result_value(value);
        normalized_entries.insert(key, normalized_value);
    }

    Value::Object(normalized_entries)
}

fn normalize_payload_summary_value(value: Value) -> Value {
    let Value::String(summary) = value else {
        return normalize_nested_result_value(value);
    };
    let parsed_summary = serde_json::from_str::<Value>(summary.as_str());

    match parsed_summary {
        Ok(parsed_value) => {
            let normalized_value = normalize_payload_summary_tree(parsed_value);
            let normalized_summary = serde_json::to_string(&normalized_value);
            Value::String(normalized_summary.unwrap_or(summary))
        }
        Err(_) => Value::String(summary.trim().to_owned()),
    }
}

fn normalize_payload_summary_tree(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            let normalized_items = items
                .into_iter()
                .map(normalize_payload_summary_tree)
                .collect::<Vec<_>>();
            Value::Array(normalized_items)
        }
        Value::Object(entries) => normalize_payload_summary_object(entries),
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_) => other,
    }
}

fn normalize_payload_summary_object(entries: Map<String, Value>) -> Value {
    let entry_has_tool_lease = entries.contains_key("tool_id") && entries.contains_key("lease");
    let mut normalized_entries = Map::new();

    for (key, value) in entries {
        if entry_has_tool_lease && VOLATILE_DISCOVERY_ENTRY_FIELDS.contains(&key.as_str()) {
            continue;
        }

        let normalized_value = normalize_payload_summary_tree(value);
        normalized_entries.insert(key, normalized_value);
    }

    Value::Object(normalized_entries)
}

#[cfg(test)]
mod tests {
    use super::super::turn_engine::{ApprovalRequirement, TurnFailure};
    use super::*;

    fn test_policy_with_consecutive_limit(limit: usize) -> ToolLoopSupervisorPolicy {
        ToolLoopSupervisorPolicy {
            max_repeated_tool_call_rounds: 100,
            max_ping_pong_cycles: 100,
            max_same_tool_failure_rounds: 100,
            max_consecutive_same_tool: limit,
        }
    }

    fn observe(
        supervisor: &mut ToolLoopSupervisor,
        policy: &ToolLoopSupervisorPolicy,
        tool_name: &str,
    ) -> ToolLoopSupervisorVerdict {
        observe_call(supervisor, policy, tool_name, tool_name, "ok")
    }

    fn observe_call(
        supervisor: &mut ToolLoopSupervisor,
        policy: &ToolLoopSupervisorPolicy,
        tool_signature: &str,
        tool_name_signature: &str,
        outcome_fingerprint: &str,
    ) -> ToolLoopSupervisorVerdict {
        supervisor.observe_round(
            policy,
            tool_signature,
            tool_name_signature,
            outcome_fingerprint,
            false,
        )
    }

    #[test]
    fn consecutive_same_tool_injects_warning_at_threshold() {
        let policy = test_policy_with_consecutive_limit(3);
        let mut supervisor = ToolLoopSupervisor::default();

        assert_eq!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::Continue
        );
        assert_eq!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::Continue
        );
        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::InjectWarning { .. }
        ));
    }

    #[test]
    fn consecutive_same_tool_hard_stops_on_repeat_warning() {
        let policy = test_policy_with_consecutive_limit(3);
        let mut supervisor = ToolLoopSupervisor::default();

        observe(&mut supervisor, &policy, "shell.exec");
        observe(&mut supervisor, &policy, "shell.exec");
        observe(&mut supervisor, &policy, "shell.exec");

        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::HardStop { .. }
        ));
    }

    #[test]
    fn consecutive_same_tool_resets_on_tool_name_change() {
        let policy = test_policy_with_consecutive_limit(3);
        let mut supervisor = ToolLoopSupervisor::default();

        observe(&mut supervisor, &policy, "shell.exec");
        observe(&mut supervisor, &policy, "shell.exec");

        assert_eq!(
            observe(&mut supervisor, &policy, "file.read"),
            ToolLoopSupervisorVerdict::Continue
        );
        assert_eq!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::Continue
        );
    }

    #[test]
    fn consecutive_same_tool_allows_same_tool_with_different_arguments() {
        let policy = test_policy_with_consecutive_limit(2);
        let mut supervisor = ToolLoopSupervisor::default();

        assert_eq!(
            observe_call(
                &mut supervisor,
                &policy,
                "file.read:{\"path\":\"a.txt\"}",
                "file.read",
                "alpha",
            ),
            ToolLoopSupervisorVerdict::Continue
        );
        assert_eq!(
            observe_call(
                &mut supervisor,
                &policy,
                "file.read:{\"path\":\"b.txt\"}",
                "file.read",
                "beta",
            ),
            ToolLoopSupervisorVerdict::Continue
        );
        assert_eq!(
            observe_call(
                &mut supervisor,
                &policy,
                "file.read:{\"path\":\"c.txt\"}",
                "file.read",
                "gamma",
            ),
            ToolLoopSupervisorVerdict::Continue
        );
    }

    #[test]
    fn semantic_no_progress_warns_across_volatile_tool_results() {
        let policy = ToolLoopSupervisorPolicy {
            max_repeated_tool_call_rounds: 1,
            max_ping_pong_cycles: 100,
            max_same_tool_failure_rounds: 100,
            max_consecutive_same_tool: 100,
        };
        let first_result = json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-1",
            "lease": "lease-1",
            "expires_at_unix": 1_777_000_001_u64,
            "payload_summary": json!({
                "results": [
                    {
                        "tool_id": "read",
                        "summary": "Read a workspace file.",
                        "lease": "lease-1",
                    }
                ],
            }).to_string(),
        });
        let second_result = json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-2",
            "lease": "lease-2",
            "expires_at_unix": 1_777_000_002_u64,
            "payload_summary": json!({
                "results": [
                    {
                        "tool_id": "read",
                        "summary": "Read a workspace file.",
                        "lease": "lease-2",
                    }
                ],
            }).to_string(),
        });
        let first_outcome = TurnResult::FinalText(first_result.to_string());
        let second_outcome = TurnResult::FinalText(second_result.to_string());
        let first_round =
            tool_loop_round_outcome(&first_outcome).expect("tool result should be fingerprintable");
        let second_round = tool_loop_round_outcome(&second_outcome)
            .expect("tool result should be fingerprintable");
        let mut supervisor = ToolLoopSupervisor::default();

        assert_eq!(first_round.fingerprint, second_round.fingerprint);
        assert_eq!(
            supervisor.observe_round(
                &policy,
                "file.read:{\"path\":\"README.md\"}",
                "file.read",
                first_round.fingerprint.as_str(),
                false,
            ),
            ToolLoopSupervisorVerdict::Continue
        );
        assert!(matches!(
            supervisor.observe_round(
                &policy,
                "file.read:{\"path\":\"README.md\"}",
                "file.read",
                second_round.fingerprint.as_str(),
                false,
            ),
            ToolLoopSupervisorVerdict::InjectWarning { .. }
        ));
    }

    #[test]
    fn tool_result_fingerprint_ignores_prefixed_volatile_identifiers() {
        let first_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-1",
                "payload_summary": json!({
                    "results": [
                        {
                            "tool_id": "read",
                            "summary": "Read a workspace file.",
                            "lease": "lease-1",
                        }
                    ],
                }).to_string(),
            })
        );
        let second_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-2",
                "payload_summary": json!({
                    "results": [
                        {
                            "tool_id": "read",
                            "summary": "Read a workspace file.",
                            "lease": "lease-2",
                        }
                    ],
                }).to_string(),
            })
        );
        let first_turn_result = TurnResult::FinalText(first_result);
        let second_turn_result = TurnResult::FinalText(second_result);
        let first_outcome = tool_loop_round_outcome(&first_turn_result)
            .expect("prefixed tool result should be fingerprintable");
        let second_outcome = tool_loop_round_outcome(&second_turn_result)
            .expect("prefixed tool result should be fingerprintable");

        assert_eq!(first_outcome.fingerprint, second_outcome.fingerprint);
    }

    #[test]
    fn tool_result_fingerprint_preserves_nested_business_identifiers() {
        let first_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "crm.lookup",
                "tool_call_id": "call-1",
                "payload_summary": json!({
                    "customer": {
                        "name": "Ada",
                        "session_id": "business-session-a",
                    }
                }).to_string(),
            })
        );
        let second_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "crm.lookup",
                "tool_call_id": "call-2",
                "payload_summary": json!({
                    "customer": {
                        "name": "Ada",
                        "session_id": "business-session-b",
                    }
                }).to_string(),
            })
        );
        let first_turn_result = TurnResult::FinalText(first_result);
        let second_turn_result = TurnResult::FinalText(second_result);
        let first_outcome = tool_loop_round_outcome(&first_turn_result)
            .expect("business tool result should be fingerprintable");
        let second_outcome = tool_loop_round_outcome(&second_turn_result)
            .expect("business tool result should be fingerprintable");

        assert_ne!(first_outcome.fingerprint, second_outcome.fingerprint);
    }

    #[test]
    fn tool_result_fingerprint_preserves_top_level_business_identifiers() {
        let first_result = format!(
            "[ok] {}",
            json!({
                "content": "same",
                "session_id": "business-session-a",
            })
        );
        let second_result = format!(
            "[ok] {}",
            json!({
                "content": "same",
                "session_id": "business-session-b",
            })
        );
        let first_turn_result = TurnResult::FinalText(first_result);
        let second_turn_result = TurnResult::FinalText(second_result);
        let first_outcome = tool_loop_round_outcome(&first_turn_result)
            .expect("business tool result should be fingerprintable");
        let second_outcome = tool_loop_round_outcome(&second_turn_result)
            .expect("business tool result should be fingerprintable");

        assert_ne!(first_outcome.fingerprint, second_outcome.fingerprint);
    }

    #[test]
    fn tool_result_fingerprint_preserves_payload_semantics_boundaries() {
        let first_result = json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-1",
            "payload_semantics": "discovery_result",
            "payload_summary": json!({
                "results": [
                    {
                        "tool_id": "read",
                        "summary": "Read a workspace file.",
                        "lease": "lease-1",
                    }
                ],
            }).to_string(),
        });
        let second_result = json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-2",
            "payload_semantics": "external_skill_context",
            "payload_summary": json!({
                "results": [
                    {
                        "tool_id": "read",
                        "summary": "Read a workspace file.",
                        "lease": "lease-2",
                    }
                ],
            }).to_string(),
        });
        let first_turn_result = TurnResult::FinalText(first_result.to_string());
        let second_turn_result = TurnResult::FinalText(second_result.to_string());
        let first_outcome = tool_loop_round_outcome(&first_turn_result)
            .expect("semantic tool result should be fingerprintable");
        let second_outcome = tool_loop_round_outcome(&second_turn_result)
            .expect("semantic tool result should be fingerprintable");

        assert_ne!(first_outcome.fingerprint, second_outcome.fingerprint);
    }

    #[test]
    fn tool_loop_round_outcome_classifies_tool_failures_and_skips_provider_errors() {
        let denied =
            TurnResult::ToolDenied(TurnFailure::policy_denied("policy", "denied by policy"));
        let provider = TurnResult::provider_error("provider", "stream failed");
        let denied_outcome =
            tool_loop_round_outcome(&denied).expect("tool denial should be fingerprintable");

        assert!(denied_outcome.failed);
        assert!(tool_loop_round_outcome(&provider).is_none());
    }

    #[test]
    fn failure_fingerprint_includes_discovery_recovery_support() {
        let plain_failure = TurnFailure::policy_denied("tool_not_found", "missing tool");
        let recovery_failure =
            TurnFailure::policy_denied_with_discovery_recovery("tool_not_found", "missing tool");
        let plain_result = TurnResult::ToolDenied(plain_failure);
        let recovery_result = TurnResult::ToolDenied(recovery_failure);
        let plain_outcome =
            tool_loop_round_outcome(&plain_result).expect("plain failure should fingerprint");
        let recovery_outcome =
            tool_loop_round_outcome(&recovery_result).expect("recovery failure should fingerprint");

        assert_ne!(plain_outcome.fingerprint, recovery_outcome.fingerprint);
    }

    #[test]
    fn approval_fingerprint_ignores_generated_request_ids() {
        let first_requirement = ApprovalRequirement::governed_tool(
            "shell.exec",
            "tool:shell.exec:ls",
            "approval required",
            "shell_exec_requires_approval",
            Some("apr-first".to_owned()),
        );
        let second_requirement = ApprovalRequirement::governed_tool(
            "shell.exec",
            "tool:shell.exec:ls",
            "approval required",
            "shell_exec_requires_approval",
            Some("apr-second".to_owned()),
        );
        let first_result = TurnResult::NeedsApproval(first_requirement);
        let second_result = TurnResult::NeedsApproval(second_requirement);
        let first_outcome = tool_loop_round_outcome(&first_result)
            .expect("approval requirement should be fingerprintable");
        let second_outcome = tool_loop_round_outcome(&second_result)
            .expect("approval requirement should be fingerprintable");

        assert_eq!(first_outcome.fingerprint, second_outcome.fingerprint);
    }
}
