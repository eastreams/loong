use serde_json::{Value, json};

use super::contracts::ProviderApiError;
use super::rate_limit::RateLimitObservation;
use super::response_debug_context::ProviderResponseDebugContext;

const PROVIDER_FAILOVER_MARKER: &str = "provider_failover=";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderFailoverReason {
    ModelMismatch,
    RateLimited,
    ProviderOverloaded,
    AuthRejected,
    PayloadIncompatible,
    TransportFailure,
    ResponseDecodeFailure,
    ResponseShapeInvalid,
    RequestRejected,
}

impl ProviderFailoverReason {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::ModelMismatch => "model_mismatch",
            Self::RateLimited => "rate_limited",
            Self::ProviderOverloaded => "provider_overloaded",
            Self::AuthRejected => "auth_rejected",
            Self::PayloadIncompatible => "payload_incompatible",
            Self::TransportFailure => "transport_failure",
            Self::ResponseDecodeFailure => "response_decode_failure",
            Self::ResponseShapeInvalid => "response_shape_invalid",
            Self::RequestRejected => "request_rejected",
        }
    }

    pub(super) fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "model_mismatch" => Some(Self::ModelMismatch),
            "rate_limited" => Some(Self::RateLimited),
            "provider_overloaded" => Some(Self::ProviderOverloaded),
            "auth_rejected" => Some(Self::AuthRejected),
            "payload_incompatible" => Some(Self::PayloadIncompatible),
            "transport_failure" => Some(Self::TransportFailure),
            "response_decode_failure" => Some(Self::ResponseDecodeFailure),
            "response_shape_invalid" => Some(Self::ResponseShapeInvalid),
            "request_rejected" => Some(Self::RequestRejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderFailoverStage {
    StatusFailure,
    TransportFailure,
    ResponseDecode,
    ResponseShapeInvalid,
    ModelCandidateRejected,
}

impl ProviderFailoverStage {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::StatusFailure => "status_failure",
            Self::TransportFailure => "transport_failure",
            Self::ResponseDecode => "response_decode",
            Self::ResponseShapeInvalid => "response_shape_invalid",
            Self::ModelCandidateRejected => "model_candidate_rejected",
        }
    }

    fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "status_failure" => Some(Self::StatusFailure),
            "transport_failure" => Some(Self::TransportFailure),
            "response_decode" => Some(Self::ResponseDecode),
            "response_shape_invalid" => Some(Self::ResponseShapeInvalid),
            "model_candidate_rejected" => Some(Self::ModelCandidateRejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProviderFailoverSnapshot {
    pub(super) reason: ProviderFailoverReason,
    pub(super) stage: ProviderFailoverStage,
    pub(super) model: String,
    pub(super) attempt: usize,
    pub(super) max_attempts: usize,
    pub(super) status_code: Option<u16>,
    pub(super) response_debug_context: Option<ProviderResponseDebugContext>,
}

impl ProviderFailoverSnapshot {
    pub(super) fn to_json_value(&self) -> Value {
        let mut payload = serde_json::Map::new();
        payload.insert("reason".to_owned(), json!(self.reason.as_str()));
        payload.insert("stage".to_owned(), json!(self.stage.as_str()));
        payload.insert("model".to_owned(), json!(self.model));
        payload.insert("attempt".to_owned(), json!(self.attempt));
        payload.insert("max_attempts".to_owned(), json!(self.max_attempts));
        if let Some(status_code) = self.status_code {
            payload.insert("status_code".to_owned(), json!(status_code));
        }
        if let Some(context) = &self.response_debug_context {
            if let Some(request_id) = &context.request_id {
                payload.insert("request_id".to_owned(), json!(request_id));
            }
            if let Some(cf_ray) = &context.cf_ray {
                payload.insert("cf_ray".to_owned(), json!(cf_ray));
            }
            if let Some(auth_error) = &context.auth_error {
                payload.insert("auth_error".to_owned(), json!(auth_error));
            }
            if let Some(auth_error_code) = &context.auth_error_code {
                payload.insert("auth_error_code".to_owned(), json!(auth_error_code));
            }
        }
        Value::Object(payload)
    }
}

#[derive(Debug)]
pub(super) struct ModelRequestError {
    pub(super) message: String,
    pub(super) try_next_model: bool,
    pub(super) reason: ProviderFailoverReason,
    pub(super) snapshot: ProviderFailoverSnapshot,
    pub(super) api_error: Option<ProviderApiError>,
    pub(super) rate_limit: Option<RateLimitObservation>,
}

pub(super) fn build_model_request_error(
    message: String,
    try_next_model: bool,
    reason: ProviderFailoverReason,
    stage: ProviderFailoverStage,
    model: &str,
    attempt: usize,
    max_attempts: usize,
    status_code: Option<u16>,
    api_error: Option<ProviderApiError>,
) -> ModelRequestError {
    build_model_request_error_with_rate_limit(
        message,
        try_next_model,
        reason,
        stage,
        model,
        attempt,
        max_attempts,
        status_code,
        api_error,
        None,
    )
}

pub(super) fn build_model_request_error_with_rate_limit(
    message: String,
    try_next_model: bool,
    reason: ProviderFailoverReason,
    stage: ProviderFailoverStage,
    model: &str,
    attempt: usize,
    max_attempts: usize,
    status_code: Option<u16>,
    api_error: Option<ProviderApiError>,
    rate_limit: Option<RateLimitObservation>,
) -> ModelRequestError {
    build_model_request_error_with_context_and_rate_limit(
        message,
        try_next_model,
        reason,
        stage,
        model,
        attempt,
        max_attempts,
        status_code,
        api_error,
        rate_limit,
        None,
    )
}

pub(super) fn build_model_request_error_with_context_and_rate_limit(
    message: String,
    try_next_model: bool,
    reason: ProviderFailoverReason,
    stage: ProviderFailoverStage,
    model: &str,
    attempt: usize,
    max_attempts: usize,
    status_code: Option<u16>,
    api_error: Option<ProviderApiError>,
    rate_limit: Option<RateLimitObservation>,
    response_debug_context: Option<ProviderResponseDebugContext>,
) -> ModelRequestError {
    let snapshot = ProviderFailoverSnapshot {
        reason,
        stage,
        model: model.to_owned(),
        attempt,
        max_attempts,
        status_code,
        response_debug_context,
    };
    let message = format!("{message} | provider_failover={}", snapshot.to_json_value());
    ModelRequestError {
        message,
        try_next_model,
        reason,
        snapshot,
        api_error,
        rate_limit,
    }
}

pub fn parse_provider_failover_snapshot_payload(error: &str) -> Option<Value> {
    let (_prefix, payload_raw) = error.rsplit_once(PROVIDER_FAILOVER_MARKER)?;
    let payload: Value = serde_json::from_str(payload_raw).ok()?;
    validate_provider_failover_snapshot_payload(payload)
}

fn validate_provider_failover_snapshot_payload(payload: Value) -> Option<Value> {
    let payload_object = payload.as_object()?;
    let has_only_known_keys = payload_object.keys().all(|key| {
        matches!(
            key.as_str(),
            "reason"
                | "stage"
                | "model"
                | "attempt"
                | "max_attempts"
                | "status_code"
                | "request_id"
                | "cf_ray"
                | "auth_error"
                | "auth_error_code"
        )
    });
    if !has_only_known_keys {
        return None;
    }

    let reason_value = payload_object.get("reason")?;
    let reason_raw = reason_value.as_str()?;
    let _reason = ProviderFailoverReason::from_str(reason_raw)?;

    let stage_value = payload_object.get("stage")?;
    let stage_raw = stage_value.as_str()?;
    let _stage = ProviderFailoverStage::from_str(stage_raw)?;

    let model_value = payload_object.get("model")?;
    let _model = model_value.as_str()?;

    let attempt_value = payload_object.get("attempt")?;
    let _attempt = attempt_value.as_u64()?;

    let max_attempts_value = payload_object.get("max_attempts")?;
    let _max_attempts = max_attempts_value.as_u64()?;

    let status_code_value = payload_object.get("status_code");
    if let Some(status_code_value) = status_code_value {
        let _status_code = status_code_value.as_u64()?;
    }
    if let Some(request_id) = payload_object.get("request_id") {
        let _request_id = request_id.as_str()?;
    }
    if let Some(cf_ray) = payload_object.get("cf_ray") {
        let _cf_ray = cf_ray.as_str()?;
    }
    if let Some(auth_error) = payload_object.get("auth_error") {
        let _auth_error = auth_error.as_str()?;
    }
    if let Some(auth_error_code) = payload_object.get("auth_error_code") {
        let _auth_error_code = auth_error_code.as_str()?;
    }

    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderFailoverReason, ProviderFailoverSnapshot, ProviderFailoverStage,
        build_model_request_error, parse_provider_failover_snapshot_payload,
    };
    use serde_json::json;

    #[test]
    fn reason_string_mapping_is_bidirectional() {
        let cases = [
            (ProviderFailoverReason::ModelMismatch, "model_mismatch"),
            (ProviderFailoverReason::RateLimited, "rate_limited"),
            (
                ProviderFailoverReason::ProviderOverloaded,
                "provider_overloaded",
            ),
            (ProviderFailoverReason::AuthRejected, "auth_rejected"),
            (
                ProviderFailoverReason::PayloadIncompatible,
                "payload_incompatible",
            ),
            (
                ProviderFailoverReason::TransportFailure,
                "transport_failure",
            ),
            (
                ProviderFailoverReason::ResponseDecodeFailure,
                "response_decode_failure",
            ),
            (
                ProviderFailoverReason::ResponseShapeInvalid,
                "response_shape_invalid",
            ),
            (ProviderFailoverReason::RequestRejected, "request_rejected"),
        ];

        for (reason, raw) in cases {
            assert_eq!(reason.as_str(), raw);
            assert_eq!(ProviderFailoverReason::from_str(raw), Some(reason));
        }
        assert_eq!(ProviderFailoverReason::from_str("unknown"), None);
    }

    #[test]
    fn snapshot_json_omits_status_code_when_absent() {
        let snapshot = ProviderFailoverSnapshot {
            reason: ProviderFailoverReason::TransportFailure,
            stage: ProviderFailoverStage::TransportFailure,
            model: "openai/gpt-4o".to_owned(),
            attempt: 2,
            max_attempts: 4,
            status_code: None,
            response_debug_context: None,
        };
        assert_eq!(
            snapshot.to_json_value(),
            json!({
                "reason": "transport_failure",
                "stage": "transport_failure",
                "model": "openai/gpt-4o",
                "attempt": 2,
                "max_attempts": 4
            })
        );
    }

    #[test]
    fn build_model_request_error_embeds_parseable_snapshot() {
        let error = build_model_request_error(
            "request failed".to_owned(),
            false,
            ProviderFailoverReason::RateLimited,
            ProviderFailoverStage::StatusFailure,
            "openai/gpt-4o",
            1,
            3,
            Some(429),
            None,
        );
        let (_, payload_raw) = error
            .message
            .split_once("provider_failover=")
            .expect("snapshot suffix should exist");
        let payload: serde_json::Value =
            serde_json::from_str(payload_raw).expect("snapshot payload should be valid JSON");
        assert_eq!(
            payload,
            json!({
                "reason": "rate_limited",
                "stage": "status_failure",
                "model": "openai/gpt-4o",
                "attempt": 1,
                "max_attempts": 3,
                "status_code": 429
            })
        );
    }

    #[test]
    fn parse_provider_failover_snapshot_payload_extracts_structured_suffix() {
        let error = "provider request failed | provider_failover={\"reason\":\"transport_failure\",\"stage\":\"transport_failure\",\"model\":\"openai/gpt-4o\",\"attempt\":2,\"max_attempts\":4}";

        let payload = parse_provider_failover_snapshot_payload(error)
            .expect("provider failover suffix should parse");

        assert_eq!(
            payload,
            json!({
                "reason": "transport_failure",
                "stage": "transport_failure",
                "model": "openai/gpt-4o",
                "attempt": 2,
                "max_attempts": 4
            })
        );
    }

    #[test]
    fn parse_provider_failover_snapshot_payload_rejects_invalid_shape() {
        let error = "provider request failed | provider_failover={\"reason\":\"unknown\",\"stage\":\"transport_failure\",\"model\":\"openai/gpt-4o\",\"attempt\":2,\"max_attempts\":4}";

        let payload = parse_provider_failover_snapshot_payload(error);

        assert!(payload.is_none());
    }

    #[test]
    fn parse_provider_failover_snapshot_payload_uses_last_marker() {
        let error = concat!(
            "provider request failed | provider_failover=",
            "{\"reason\":\"rate_limited\",\"stage\":\"status_failure\",\"model\":\"openai/gpt-4o\",\"attempt\":1,\"max_attempts\":3}",
            " extra context | provider_failover=",
            "{\"reason\":\"transport_failure\",\"stage\":\"transport_failure\",\"model\":\"openai/gpt-4o\",\"attempt\":2,\"max_attempts\":4}",
        );

        let payload = parse_provider_failover_snapshot_payload(error)
            .expect("provider failover suffix should parse from the last marker");

        assert_eq!(payload["reason"], "transport_failure");
        assert_eq!(payload["stage"], "transport_failure");
        assert_eq!(payload["attempt"], 2);
        assert_eq!(payload["max_attempts"], 4);
    }

    #[test]
    fn parse_provider_failover_snapshot_payload_rejects_unknown_keys() {
        let error = concat!(
            "provider request failed | provider_failover=",
            "{\"reason\":\"transport_failure\",\"stage\":\"transport_failure\",\"model\":\"openai/gpt-4o\",\"attempt\":2,\"max_attempts\":4,\"unexpected\":true}",
        );

        let payload = parse_provider_failover_snapshot_payload(error);

        assert!(payload.is_none());
    }

    #[test]
    fn parse_provider_failover_snapshot_payload_accepts_response_debug_context() {
        let error = concat!(
            "provider request failed | provider_failover=",
            "{\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"openai/gpt-4o\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401,\"request_id\":\"req-123\",\"cf_ray\":\"ray-123\",\"auth_error\":\"missing_authorization_header\",\"auth_error_code\":\"token_expired\"}",
        );

        let payload = parse_provider_failover_snapshot_payload(error)
            .expect("provider failover suffix with debug context should parse");

        assert_eq!(payload["request_id"], "req-123");
        assert_eq!(payload["cf_ray"], "ray-123");
        assert_eq!(payload["auth_error"], "missing_authorization_header");
        assert_eq!(payload["auth_error_code"], "token_expired");
    }
}
