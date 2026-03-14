use serde_json::{Value, json};

use super::contracts::ProviderApiError;

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
}

#[derive(Debug, Clone)]
pub(super) struct ProviderFailoverSnapshot {
    pub(super) reason: ProviderFailoverReason,
    pub(super) stage: ProviderFailoverStage,
    pub(super) model: String,
    pub(super) attempt: usize,
    pub(super) max_attempts: usize,
    pub(super) status_code: Option<u16>,
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
    let snapshot = ProviderFailoverSnapshot {
        reason,
        stage,
        model: model.to_owned(),
        attempt,
        max_attempts,
        status_code,
    };
    let message = format!("{message} | provider_failover={}", snapshot.to_json_value());
    ModelRequestError {
        message,
        try_next_model,
        reason,
        snapshot,
        api_error,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderFailoverReason, ProviderFailoverSnapshot, ProviderFailoverStage,
        build_model_request_error,
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
}
