use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecision {
    pub allow: bool,
    pub deny: bool,
    pub reason: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecisionKind {
    Allow,
    ApprovalRequired,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDecisionTelemetry {
    pub tool_name: String,
    pub decision_kind: ToolDecisionKind,
    pub allow: bool,
    pub deny: bool,
    pub reason: String,
    pub rule_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autonomy_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_action_class: Option<String>,
}

impl ToolDecisionTelemetry {
    pub(super) fn allow(
        tool_name: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision_kind: ToolDecisionKind::Allow,
            allow: true,
            deny: false,
            reason: reason.into(),
            rule_id: rule_id.into(),
            reason_code: None,
            policy_source: None,
            autonomy_profile: None,
            capability_action_class: None,
        }
    }

    pub(super) fn approval_required(
        tool_name: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision_kind: ToolDecisionKind::ApprovalRequired,
            allow: false,
            deny: false,
            reason: reason.into(),
            rule_id: rule_id.into(),
            reason_code: None,
            policy_source: None,
            autonomy_profile: None,
            capability_action_class: None,
        }
    }

    pub(super) fn deny(
        tool_name: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision_kind: ToolDecisionKind::Deny,
            allow: false,
            deny: true,
            reason: reason.into(),
            rule_id: rule_id.into(),
            reason_code: None,
            policy_source: None,
            autonomy_profile: None,
            capability_action_class: None,
        }
    }

    pub(super) fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }

    pub(super) fn with_policy_source(mut self, policy_source: impl Into<String>) -> Self {
        self.policy_source = Some(policy_source.into());
        self
    }

    pub(super) fn with_autonomy_profile(mut self, autonomy_profile: impl Into<String>) -> Self {
        self.autonomy_profile = Some(autonomy_profile.into());
        self
    }

    pub(super) fn with_capability_action_class(
        mut self,
        capability_action_class: impl Into<String>,
    ) -> Self {
        self.capability_action_class = Some(capability_action_class.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolOutcomeTelemetry {
    pub tool_name: String,
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
}
