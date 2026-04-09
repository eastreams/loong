use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::CliResult;
use crate::config::ProviderAuthScheme;

use super::auth_profile_runtime::ProviderAuthProfile;
use super::failover::{ProviderFailoverReason, ProviderFailoverStage};

pub(super) type TransportEventStream =
    Pin<Box<dyn Stream<Item = Result<Value, TransportError>> + Send>>;

#[derive(Debug, Clone)]
pub(super) struct PreparedTransportAuth {
    header_name: HeaderName,
    header_value: HeaderValue,
}

impl PreparedTransportAuth {
    pub(super) fn apply(&self, headers: &mut HeaderMap) {
        if headers.contains_key(&self.header_name) {
            return;
        }
        headers.insert(self.header_name.clone(), self.header_value.clone());
    }
}

#[derive(Debug, Clone)]
pub(super) struct TransportRequest {
    pub(super) method: reqwest::Method,
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct TransportResponse {
    pub(super) status: reqwest::StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Value,
}

pub(super) enum TransportStream {
    Events { events: TransportEventStream },
    Response(TransportResponse),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransportErrorKind {
    Timeout,
    Connect,
    Request,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransportError {
    kind: TransportErrorKind,
    reason: ProviderFailoverReason,
    stage: ProviderFailoverStage,
    message: String,
}

impl TransportError {
    pub(super) fn new(kind: TransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            reason: ProviderFailoverReason::TransportFailure,
            stage: ProviderFailoverStage::TransportFailure,
            message: message.into(),
        }
    }

    pub(super) fn other(message: impl Into<String>) -> Self {
        Self::new(TransportErrorKind::Other, message)
    }

    pub(super) fn is_timeout(&self) -> bool {
        self.kind == TransportErrorKind::Timeout
    }

    pub(super) fn is_connect(&self) -> bool {
        self.kind == TransportErrorKind::Connect
    }

    pub(super) fn is_request(&self) -> bool {
        self.kind == TransportErrorKind::Request
    }

    pub(super) fn reason(&self) -> ProviderFailoverReason {
        self.reason
    }

    pub(super) fn stage(&self) -> ProviderFailoverStage {
        self.stage
    }

    pub(super) fn response_decode(message: impl Into<String>) -> Self {
        Self {
            kind: TransportErrorKind::Other,
            reason: ProviderFailoverReason::ResponseDecodeFailure,
            stage: ProviderFailoverStage::ResponseDecode,
            message: message.into(),
        }
    }

    pub(super) fn response_shape_invalid(message: impl Into<String>) -> Self {
        Self {
            kind: TransportErrorKind::Other,
            reason: ProviderFailoverReason::ResponseShapeInvalid,
            stage: ProviderFailoverStage::ResponseShapeInvalid,
            message: message.into(),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message.as_str())
    }
}

impl std::error::Error for TransportError {}

impl From<reqwest::Error> for TransportError {
    fn from(error: reqwest::Error) -> Self {
        let kind = if error.is_timeout() {
            TransportErrorKind::Timeout
        } else if error.is_connect() {
            TransportErrorKind::Connect
        } else if error.is_request() {
            TransportErrorKind::Request
        } else {
            TransportErrorKind::Other
        };
        Self::new(kind, error.to_string())
    }
}

pub(super) fn resolve_transport_auth(
    profile: Option<&ProviderAuthProfile>,
    auth_scheme: ProviderAuthScheme,
) -> CliResult<Option<PreparedTransportAuth>> {
    let Some(profile) = profile else {
        return Ok(None);
    };

    match auth_scheme {
        ProviderAuthScheme::Bearer => {
            let Some(secret) = profile
                .authorization_secret
                .as_deref()
                .or(profile.api_key_secret.as_deref())
            else {
                return Ok(None);
            };
            let header_value = HeaderValue::from_str(format!("Bearer {secret}").as_str())
                .map_err(|error| format!("invalid provider authorization header: {error}"))?;
            Ok(Some(PreparedTransportAuth {
                header_name: HeaderName::from_static("authorization"),
                header_value,
            }))
        }
        ProviderAuthScheme::XApiKey => {
            let Some(secret) = profile.api_key_secret.as_deref() else {
                return Ok(None);
            };
            let header_value = HeaderValue::from_str(secret)
                .map_err(|error| format!("invalid provider x-api-key header: {error}"))?;
            Ok(Some(PreparedTransportAuth {
                header_name: HeaderName::from_static("x-api-key"),
                header_value,
            }))
        }
        ProviderAuthScheme::XGoogApiKey => {
            let Some(secret) = profile.api_key_secret.as_deref() else {
                return Ok(None);
            };
            let header_value = HeaderValue::from_str(secret)
                .map_err(|error| format!("invalid provider x-goog-api-key header: {error}"))?;
            Ok(Some(PreparedTransportAuth {
                header_name: HeaderName::from_static("x-goog-api-key"),
                header_value,
            }))
        }
    }
}

#[async_trait]
pub(super) trait ProviderTransport: Send + Sync {
    async fn execute(&self, request: TransportRequest)
    -> Result<TransportResponse, TransportError>;

    async fn stream(&self, request: TransportRequest) -> Result<TransportStream, TransportError>;
}
