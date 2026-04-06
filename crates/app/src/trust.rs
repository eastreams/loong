use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) const TRUST_EVENT_PAYLOAD_KEY: &str = "trust_event";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustActorKind {
    Operator,
    ConversationRuntime,
    ProviderRuntime,
    DelegateChildRuntime,
    ConnectorCaller,
    PluginRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustEventKind {
    ApprovalRequired,
    IdentityBound,
    IdentityMismatch,
    DelegationCreated,
    DelegationRejected,
    ProvenanceMismatch,
    TrustAttested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustStateHint {
    Unknown,
    Bound,
    Degraded,
    Attested,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustProvenanceKind {
    SessionLineage,
    RuntimeBinding,
    ConnectorCaller,
    PluginAttestation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEventEnvelope {
    pub event_kind: TrustEventKind,
    pub actor_id: String,
    pub actor_kind: TrustActorKind,
    pub source_surface: String,
    pub trust_state_hint: TrustStateHint,
    pub provenance_kind: TrustProvenanceKind,
    pub provenance_ref: String,
    pub reason_code: String,
    pub evidence_ref: String,
}

pub(crate) fn embed_trust_event_payload(payload: Value, trust_event: TrustEventEnvelope) -> Value {
    let trust_event_value = match serde_json::to_value(trust_event) {
        Ok(trust_event_value) => trust_event_value,
        Err(_error) => return payload,
    };

    let mut payload_object = match payload {
        Value::Object(payload_object) => payload_object,
        payload @ Value::Null
        | payload @ Value::Bool(_)
        | payload @ Value::Number(_)
        | payload @ Value::String(_)
        | payload @ Value::Array(_) => {
            let mut payload_object = Map::new();
            payload_object.insert("payload".to_owned(), payload);
            payload_object
        }
    };

    payload_object.insert(TRUST_EVENT_PAYLOAD_KEY.to_owned(), trust_event_value);

    Value::Object(payload_object)
}

pub(crate) fn extract_trust_event_payload(payload: &Value) -> Option<TrustEventEnvelope> {
    let payload_object = payload.as_object()?;
    let trust_event_value = payload_object.get(TRUST_EVENT_PAYLOAD_KEY)?;
    let trust_event_value = trust_event_value.clone();
    let trust_event = serde_json::from_value(trust_event_value).ok()?;

    Some(trust_event)
}

pub(crate) fn delegate_child_trust_event(
    parent_session_id: &str,
    child_session_id: &str,
    source_surface: &str,
) -> TrustEventEnvelope {
    TrustEventEnvelope {
        event_kind: TrustEventKind::DelegationCreated,
        actor_id: child_session_id.to_owned(),
        actor_kind: TrustActorKind::DelegateChildRuntime,
        source_surface: source_surface.to_owned(),
        trust_state_hint: TrustStateHint::Bound,
        provenance_kind: TrustProvenanceKind::SessionLineage,
        provenance_ref: parent_session_id.to_owned(),
        reason_code: "delegate_child_session_created".to_owned(),
        evidence_ref: format!("session:{child_session_id}"),
    }
}

/// Build the additive trust envelope for a missing runtime binding.
///
/// `provenance_ref` is usually `"kernel"` for kernel-bound sessions and
/// `"direct"` for direct bindings that reached a core-only path without kernel
/// context.
pub(crate) fn runtime_binding_missing_trust_event(
    session_id: &str,
    source_surface: &str,
    provenance_ref: &str,
) -> TrustEventEnvelope {
    TrustEventEnvelope {
        event_kind: TrustEventKind::ProvenanceMismatch,
        actor_id: session_id.to_owned(),
        actor_kind: TrustActorKind::ConversationRuntime,
        source_surface: source_surface.to_owned(),
        trust_state_hint: TrustStateHint::Rejected,
        provenance_kind: TrustProvenanceKind::RuntimeBinding,
        provenance_ref: provenance_ref.to_owned(),
        reason_code: "no_kernel_context".to_owned(),
        evidence_ref: format!("session:{session_id}"),
    }
}

pub(crate) fn approval_required_trust_event(
    session_id: &str,
    source_surface: &str,
    provenance_ref: &str,
    rule_id: &str,
    approval_request_id: Option<&str>,
    tool_name: Option<&str>,
) -> TrustEventEnvelope {
    let evidence_ref = match approval_request_id {
        Some(approval_request_id) => format!("approval_request:{approval_request_id}"),
        None => match tool_name {
            Some(tool_name) => format!("tool:{tool_name}"),
            None => format!("session:{session_id}"),
        },
    };

    TrustEventEnvelope {
        event_kind: TrustEventKind::ApprovalRequired,
        actor_id: session_id.to_owned(),
        actor_kind: TrustActorKind::ConversationRuntime,
        source_surface: source_surface.to_owned(),
        trust_state_hint: TrustStateHint::Unknown,
        provenance_kind: TrustProvenanceKind::RuntimeBinding,
        provenance_ref: provenance_ref.to_owned(),
        reason_code: rule_id.to_owned(),
        evidence_ref,
    }
}

pub(crate) fn provider_failover_trust_event(
    provider_id: &str,
    source_surface: &str,
    provenance_ref: &str,
    reason_code: &str,
    model: &str,
    stage: &str,
) -> TrustEventEnvelope {
    let trust_state_hint = provider_failover_trust_state_hint(reason_code);

    TrustEventEnvelope {
        event_kind: TrustEventKind::TrustAttested,
        actor_id: provider_id.to_owned(),
        actor_kind: TrustActorKind::ProviderRuntime,
        source_surface: source_surface.to_owned(),
        trust_state_hint,
        provenance_kind: TrustProvenanceKind::RuntimeBinding,
        provenance_ref: provenance_ref.to_owned(),
        reason_code: reason_code.to_owned(),
        evidence_ref: format!("provider:{provider_id}:model:{model}:stage:{stage}"),
    }
}

fn provider_failover_trust_state_hint(reason_code: &str) -> TrustStateHint {
    match reason_code {
        "auth_rejected" => TrustStateHint::Rejected,
        "model_mismatch" => TrustStateHint::Rejected,
        "payload_incompatible" => TrustStateHint::Rejected,
        "request_rejected" => TrustStateHint::Rejected,
        "rate_limited" => TrustStateHint::Degraded,
        "provider_overloaded" => TrustStateHint::Degraded,
        "transport_failure" => TrustStateHint::Degraded,
        "response_decode_failure" => TrustStateHint::Degraded,
        "response_shape_invalid" => TrustStateHint::Degraded,
        _ => TrustStateHint::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TRUST_EVENT_PAYLOAD_KEY, TrustActorKind, TrustEventEnvelope, TrustEventKind,
        TrustProvenanceKind, TrustStateHint, approval_required_trust_event,
        delegate_child_trust_event, embed_trust_event_payload, extract_trust_event_payload,
        provider_failover_trust_event,
    };
    use serde_json::json;

    #[test]
    fn trust_event_envelope_round_trips_through_json() {
        let envelope = TrustEventEnvelope {
            event_kind: TrustEventKind::DelegationCreated,
            actor_id: "child-session".to_owned(),
            actor_kind: TrustActorKind::DelegateChildRuntime,
            source_surface: "delegate.async".to_owned(),
            trust_state_hint: TrustStateHint::Bound,
            provenance_kind: TrustProvenanceKind::SessionLineage,
            provenance_ref: "root-session".to_owned(),
            reason_code: "delegate_child_session_created".to_owned(),
            evidence_ref: "session:child-session".to_owned(),
        };

        let encoded = serde_json::to_value(&envelope).expect("encode trust event");
        let decoded: TrustEventEnvelope =
            serde_json::from_value(encoded).expect("decode trust event");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn embed_trust_event_payload_preserves_existing_object_fields() {
        let trust_event =
            delegate_child_trust_event("root-session", "child-session", "delegate.async");
        let payload = json!({
            "task": "child async task",
            "timeout_seconds": 60,
        });

        let enriched = embed_trust_event_payload(payload, trust_event);

        assert_eq!(enriched["task"], "child async task");
        assert_eq!(
            enriched[TRUST_EVENT_PAYLOAD_KEY]["event_kind"],
            "delegation_created"
        );
        assert_eq!(
            enriched[TRUST_EVENT_PAYLOAD_KEY]["actor_kind"],
            "delegate_child_runtime"
        );
    }

    #[test]
    fn embed_trust_event_payload_wraps_non_object_payload() {
        let trust_event =
            delegate_child_trust_event("root-session", "child-session", "delegate.inline");
        let enriched = embed_trust_event_payload(json!("raw payload"), trust_event);

        assert_eq!(enriched["payload"], "raw payload");
        assert_eq!(
            enriched[TRUST_EVENT_PAYLOAD_KEY]["source_surface"],
            "delegate.inline"
        );
    }

    #[test]
    fn provider_failover_trust_event_marks_degraded_provider_runtime_state() {
        let envelope = provider_failover_trust_event(
            "openai",
            "provider.failover",
            "kernel",
            "rate_limited",
            "gpt-4o",
            "status_failure",
        );

        assert_eq!(envelope.event_kind, TrustEventKind::TrustAttested);
        assert_eq!(envelope.actor_kind, TrustActorKind::ProviderRuntime);
        assert_eq!(envelope.trust_state_hint, TrustStateHint::Degraded);
        assert_eq!(
            envelope.provenance_kind,
            TrustProvenanceKind::RuntimeBinding
        );
        assert_eq!(envelope.provenance_ref, "kernel");
        assert_eq!(envelope.reason_code, "rate_limited");
        assert_eq!(
            envelope.evidence_ref,
            "provider:openai:model:gpt-4o:stage:status_failure"
        );
    }

    #[test]
    fn provider_failover_trust_event_marks_rejected_provider_runtime_state_for_auth_failures() {
        let envelope = provider_failover_trust_event(
            "openai",
            "provider.failover",
            "kernel",
            "auth_rejected",
            "gpt-4o",
            "status_failure",
        );

        assert_eq!(envelope.trust_state_hint, TrustStateHint::Rejected);
    }

    #[test]
    fn approval_required_trust_event_uses_approval_request_evidence_when_present() {
        let envelope = approval_required_trust_event(
            "root-session",
            "conversation.approval",
            "kernel",
            "governed_tool_requires_approval",
            Some("apr-123"),
            Some("delegate"),
        );

        assert_eq!(envelope.event_kind, TrustEventKind::ApprovalRequired);
        assert_eq!(envelope.actor_kind, TrustActorKind::ConversationRuntime);
        assert_eq!(envelope.trust_state_hint, TrustStateHint::Unknown);
        assert_eq!(envelope.provenance_ref, "kernel");
        assert_eq!(envelope.reason_code, "governed_tool_requires_approval");
        assert_eq!(envelope.evidence_ref, "approval_request:apr-123");
    }

    #[test]
    fn extract_trust_event_payload_reads_embedded_trust_event() {
        let trust_event =
            delegate_child_trust_event("root-session", "child-session", "delegate.inline");
        let payload = embed_trust_event_payload(json!({ "task": "child task" }), trust_event);

        let extracted = extract_trust_event_payload(&payload).expect("extract trust event");

        assert_eq!(extracted.event_kind, TrustEventKind::DelegationCreated);
        assert_eq!(extracted.actor_kind, TrustActorKind::DelegateChildRuntime);
    }

    #[test]
    fn extract_trust_event_payload_rejects_malformed_embedded_value() {
        let payload = json!({
            TRUST_EVENT_PAYLOAD_KEY: {
                "event_kind": "delegation_created",
                "actor_id": "child-session",
                "actor_kind": "delegate_child_runtime",
                "source_surface": "delegate.inline"
            }
        });

        let extracted = extract_trust_event_payload(&payload);

        assert!(extracted.is_none());
    }
}
