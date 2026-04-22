use base64::Engine;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REQUEST_ID_HEADER: &str = "x-request-id";
const OAI_REQUEST_ID_HEADER: &str = "x-oai-request-id";
const CF_RAY_HEADER: &str = "cf-ray";
const AUTH_ERROR_HEADER: &str = "x-openai-authorization-error";
const X_ERROR_JSON_HEADER: &str = "x-error-json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderResponseDebugContext {
    pub request_id: Option<String>,
    pub cf_ray: Option<String>,
    pub auth_error: Option<String>,
    pub auth_error_code: Option<String>,
}

impl ProviderResponseDebugContext {
    pub fn is_empty(&self) -> bool {
        self.request_id.is_none()
            && self.cf_ray.is_none()
            && self.auth_error.is_none()
            && self.auth_error_code.is_none()
    }
}

pub fn extract_provider_response_debug_context(
    headers: &HeaderMap,
    body: &Value,
) -> Option<ProviderResponseDebugContext> {
    let extract_header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };

    let auth_error_code = extract_header(X_ERROR_JSON_HEADER)
        .and_then(|encoded| decode_auth_error_code(encoded.as_str()))
        .or_else(|| {
            body.get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });

    let context = ProviderResponseDebugContext {
        request_id: extract_header(REQUEST_ID_HEADER)
            .or_else(|| extract_header(OAI_REQUEST_ID_HEADER)),
        cf_ray: extract_header(CF_RAY_HEADER),
        auth_error: extract_header(AUTH_ERROR_HEADER),
        auth_error_code,
    };

    (!context.is_empty()).then_some(context)
}

fn decode_auth_error_code(encoded: &str) -> Option<String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let parsed = serde_json::from_slice::<Value>(&decoded).ok()?;
    parsed
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{ProviderResponseDebugContext, extract_provider_response_debug_context};
    use reqwest::header::{HeaderMap, HeaderValue};
    use serde_json::json;

    #[test]
    fn response_debug_context_extracts_identity_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-oai-request-id", HeaderValue::from_static("req-123"));
        headers.insert("cf-ray", HeaderValue::from_static("ray-123"));
        headers.insert(
            "x-openai-authorization-error",
            HeaderValue::from_static("missing_authorization_header"),
        );
        headers.insert(
            "x-error-json",
            HeaderValue::from_static("eyJlcnJvciI6eyJjb2RlIjoidG9rZW5fZXhwaXJlZCJ9fQ=="),
        );

        let context = extract_provider_response_debug_context(&headers, &json!({}))
            .expect("debug context should exist");

        assert_eq!(
            context,
            ProviderResponseDebugContext {
                request_id: Some("req-123".to_owned()),
                cf_ray: Some("ray-123".to_owned()),
                auth_error: Some("missing_authorization_header".to_owned()),
                auth_error_code: Some("token_expired".to_owned()),
            }
        );
    }

    #[test]
    fn response_debug_context_falls_back_to_body_error_code() {
        let headers = HeaderMap::new();
        let context = extract_provider_response_debug_context(
            &headers,
            &json!({"error": {"code": "context_length_exceeded"}}),
        )
        .expect("body error code should produce context");

        assert_eq!(
            context.auth_error_code.as_deref(),
            Some("context_length_exceeded")
        );
    }

    #[test]
    fn response_debug_context_returns_none_when_empty() {
        assert!(extract_provider_response_debug_context(&HeaderMap::new(), &json!({})).is_none());
    }
}
