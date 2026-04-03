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

#[cfg(test)]
mod tests {
    use super::{
        TRUST_EVENT_PAYLOAD_KEY, TrustActorKind, TrustEventEnvelope, TrustEventKind,
        TrustProvenanceKind, TrustStateHint, delegate_child_trust_event, embed_trust_event_payload,
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
}
