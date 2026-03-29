#[cfg(feature = "provider-bedrock")]
use std::time::SystemTime;

#[cfg(feature = "provider-bedrock")]
use aws_config::{
    Region,
    default_provider::{credentials::DefaultCredentialsChain, region::DefaultRegionChain},
};
#[cfg(feature = "provider-bedrock")]
use aws_credential_types::{Credentials, provider::ProvideCredentials};
#[cfg(feature = "provider-bedrock")]
use aws_sigv4::{
    http_request::{self, SignableBody, SignableRequest, SigningSettings},
    sign::v4,
};
use bytes::Bytes;
use futures_util::Stream;
use futures_util::StreamExt;
use reqwest::header::{
    AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT,
};
use serde_json::{Value, json};

use crate::CliResult;
use crate::config::{ProviderAuthScheme, ProviderConfig, ProviderKind, active_cli_command_name};

use super::auth_profile_runtime::ProviderAuthProfile;

#[derive(Debug, Clone, Default)]
pub(super) struct RequestAuthContext {
    pub(super) bedrock_region: Option<String>,
    #[cfg(feature = "provider-bedrock")]
    bedrock_signing: Option<BedrockSigningContext>,
}

impl RequestAuthContext {
    pub(super) fn has_bedrock_sigv4_fallback(&self) -> bool {
        #[cfg(feature = "provider-bedrock")]
        {
            self.bedrock_signing.is_some()
        }

        #[cfg(not(feature = "provider-bedrock"))]
        {
            false
        }
    }
}

#[cfg(feature = "provider-bedrock")]
#[derive(Debug, Clone)]
struct BedrockSigningContext {
    region: String,
    credentials: Credentials,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BedrockService {
    Runtime,
    ModelCatalog,
}

impl BedrockService {
    fn signing_name(self) -> &'static str {
        match self {
            Self::Runtime => "bedrock-runtime",
            Self::ModelCatalog => "bedrock",
        }
    }
}

#[derive(Debug)]
pub(super) enum RequestExecutionError {
    Transport(reqwest::Error),
    Setup(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(super) enum SseLine {
    EventType { name: String },
    Data { content: String },
    Retry { timeout_ms: u64 },
    Comment,
    Empty,
}

pub(super) fn parse_sse_line(line: &str) -> SseLine {
    if line.is_empty() {
        return SseLine::Empty;
    }
    if line.starts_with(':') {
        return SseLine::Comment;
    }
    if let Some(rest) = line.strip_prefix("event:") {
        let trimmed = rest.trim();
        return SseLine::EventType {
            name: trimmed.to_owned(),
        };
    }
    if let Some(rest) = line.strip_prefix("retry:") {
        let trimmed = rest.trim();
        let timeout_ms = trimmed.parse().unwrap_or(3000);
        return SseLine::Retry { timeout_ms };
    }
    if let Some(rest) = line.strip_prefix("data:") {
        let content = rest.trim().to_owned();
        return SseLine::Data { content };
    }
    SseLine::Empty
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum SseStreamEvent {
    Message {
        data: Value,
        event_type: Option<String>,
    },
    #[allow(dead_code)]
    Error { message: String },
    #[allow(dead_code)]
    Done,
}

impl SseStreamEvent {
    pub(super) fn from_sse_lines(
        event_type: Option<String>,
        data_lines: &[String],
    ) -> Result<Option<Self>, serde_json::Error> {
        if data_lines.is_empty() {
            return Ok(None);
        }
        let combined = data_lines.join("\n");
        if combined.is_empty() {
            return Ok(None);
        }
        let parsed: Value = serde_json::from_str(&combined)?;
        Ok(Some(SseStreamEvent::Message {
            data: parsed,
            event_type,
        }))
    }
}

pub(super) async fn resolve_request_auth_context(
    provider: &ProviderConfig,
) -> CliResult<RequestAuthContext> {
    if provider.kind != ProviderKind::Bedrock {
        return Ok(RequestAuthContext::default());
    }

    let region = resolve_bedrock_region(provider).await?;

    #[cfg(feature = "provider-bedrock")]
    {
        match resolve_bedrock_credentials(region.as_str()).await {
            Ok(credentials) => Ok(RequestAuthContext {
                bedrock_region: Some(region.clone()),
                bedrock_signing: Some(BedrockSigningContext {
                    region,
                    credentials,
                }),
            }),
            Err(error) => {
                if provider.resolved_auth_secret().is_some() {
                    return Ok(RequestAuthContext {
                        bedrock_region: Some(region),
                        bedrock_signing: None,
                    });
                }
                Err(error)
            }
        }
    }

    #[cfg(not(feature = "provider-bedrock"))]
    {
        if provider.resolved_auth_secret().is_some() {
            return Ok(RequestAuthContext {
                bedrock_region: Some(region),
            });
        }
        let support_facts = provider.support_facts();
        let feature_support = support_facts.feature;
        Err(feature_support.disabled_message)
    }
}

pub(super) fn resolve_request_endpoint(
    provider: &ProviderConfig,
    endpoint_template: &str,
    model: &str,
) -> String {
    match provider.kind.protocol_family() {
        crate::config::ProviderProtocolFamily::BedrockConverse => {
            endpoint_template.replace("{modelId}", percent_encode_path_segment(model).as_str())
        }
        crate::config::ProviderProtocolFamily::AnthropicMessages
        | crate::config::ProviderProtocolFamily::OpenAiChatCompletions => {
            endpoint_template.to_owned()
        }
    }
}

pub(super) fn resolve_request_url(
    provider: &ProviderConfig,
    url: &str,
    auth_context: &RequestAuthContext,
) -> CliResult<String> {
    if provider.kind != ProviderKind::Bedrock || !url.contains("<region>") {
        return Ok(url.to_owned());
    }

    let Some(region) = auth_context.bedrock_region.as_deref() else {
        return Err(
            "bedrock request endpoint still contains `<region>` and no AWS region could be resolved"
                .to_owned(),
        );
    };

    Ok(url.replace("<region>", region))
}

fn request_host_label(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

fn message_contains_token(message: &str, token: &str) -> bool {
    message
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|part| part == token)
}

fn message_looks_like_dns_failure(message: &str) -> bool {
    message_contains_token(message, "dns")
        || message.contains("lookup address")
        || message.contains("name or service not known")
        || message.contains("nodename nor servname")
        || message.contains("temporary failure in name resolution")
        || message.contains("failed to lookup address information")
        || message.contains("no such host")
}

fn message_looks_like_proxy_route_failure(message: &str) -> bool {
    message.contains("proxy")
        || message.contains("tunnel")
        || message.contains("socks")
        || message_contains_token(message, "tun")
        || message.contains("utun")
        || message.contains("tun0")
}

pub(super) fn render_transport_route_hint(
    url: &str,
    error_message: &str,
    is_timeout: bool,
    is_connect: bool,
) -> Option<String> {
    let host = request_host_label(url)?;
    let lower = error_message.to_ascii_lowercase();
    let doctor_command = format!("{} doctor", active_cli_command_name());

    if is_timeout {
        return Some(format!(
            "request host {host}: the transport timed out before an HTTP response arrived. if you're using a proxy/TUN/fake-ip setup, verify that the route stays healthy for longer-lived requests, then run `{doctor_command}` to inspect provider route diagnostics"
        ));
    }

    if is_connect && message_looks_like_dns_failure(lower.as_str()) {
        return Some(format!(
            "request host {host}: dns resolution failed before the request reached the provider. check local dns / proxy / TUN rules, then run `{doctor_command}` to inspect provider route diagnostics"
        ));
    }

    if message_looks_like_proxy_route_failure(lower.as_str()) {
        return Some(format!(
            "request host {host}: the transport failed while crossing a proxy/TUN route. verify that the local proxy path is healthy, then run `{doctor_command}` to inspect provider route diagnostics"
        ));
    }

    if is_connect {
        return Some(format!(
            "request host {host}: the connection failed before an HTTP status was returned. this usually points to dns, proxy/TUN routing, or another local network-path problem. run `{doctor_command}` to inspect provider route diagnostics"
        ));
    }

    None
}

pub(super) fn build_request_headers_without_provider_auth(
    provider: &ProviderConfig,
) -> CliResult<HeaderMap> {
    build_request_headers_internal(provider, false)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn build_request_headers(provider: &ProviderConfig) -> CliResult<HeaderMap> {
    build_request_headers_internal(provider, true)
}

fn build_request_headers_internal(
    provider: &ProviderConfig,
    include_provider_auth: bool,
) -> CliResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (key, value) in &provider.headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|error| format!("invalid provider header name `{key}`: {error}"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid provider header value for `{key}`: {error}"))?;
        headers.insert(name, header_value);
    }
    if !headers.contains_key(USER_AGENT)
        && let Some(default_user_agent) = provider.kind.default_user_agent()
    {
        let header_value = HeaderValue::from_str(default_user_agent).map_err(|error| {
            format!("invalid default provider user-agent `{default_user_agent}`: {error}")
        })?;
        headers.insert(USER_AGENT, header_value);
    }
    for (key, value) in provider.kind.default_headers() {
        if headers.contains_key(*key) {
            continue;
        }
        let name = HeaderName::from_static(key);
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid default provider header `{key}`: {error}"))?;
        headers.insert(name, header_value);
    }
    if include_provider_auth && let Some(secret) = provider.resolved_auth_secret() {
        apply_raw_auth_secret(&mut headers, provider.kind.auth_scheme(), secret.as_str())?;
    }
    Ok(headers)
}

pub(super) fn apply_auth_profile_headers(
    headers: &mut HeaderMap,
    profile: Option<&ProviderAuthProfile>,
) -> CliResult<()> {
    let Some(profile) = profile else {
        return Ok(());
    };
    if let Some(value) = profile.authorization_header.as_deref()
        && !headers.contains_key(AUTHORIZATION)
    {
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid provider authorization header: {error}"))?;
        headers.insert(AUTHORIZATION, header_value);
    }
    if let Some(value) = profile.x_api_key_header.as_deref()
        && !headers.contains_key("x-api-key")
    {
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid provider x-api-key header: {error}"))?;
        headers.insert(HeaderName::from_static("x-api-key"), header_value);
    }
    Ok(())
}

fn apply_raw_auth_secret(
    headers: &mut HeaderMap,
    auth_scheme: ProviderAuthScheme,
    secret: &str,
) -> CliResult<()> {
    match auth_scheme {
        ProviderAuthScheme::Bearer => {
            if headers.contains_key(AUTHORIZATION) {
                return Ok(());
            }
            let header_value = HeaderValue::from_str(format!("Bearer {secret}").as_str())
                .map_err(|error| format!("invalid provider authorization header: {error}"))?;
            headers.insert(AUTHORIZATION, header_value);
        }
        ProviderAuthScheme::XApiKey => {
            if headers.contains_key("x-api-key") {
                return Ok(());
            }
            let header_value = HeaderValue::from_str(secret)
                .map_err(|error| format!("invalid provider x-api-key header: {error}"))?;
            headers.insert(HeaderName::from_static("x-api-key"), header_value);
        }
    }
    Ok(())
}

pub(super) fn encode_json_request_body(body: &Value) -> CliResult<Vec<u8>> {
    serde_json::to_vec(body)
        .map_err(|error| format!("encode provider request body failed: {error}"))
}

pub(super) fn apply_json_request_defaults(headers: &mut HeaderMap) -> CliResult<()> {
    if headers.contains_key(CONTENT_TYPE) {
        return Ok(());
    }
    let content_type = HeaderValue::from_static("application/json");
    headers.insert(CONTENT_TYPE, content_type);
    Ok(())
}

pub(super) async fn execute_request(
    client: &reqwest::Client,
    mut request: reqwest::Request,
    body_bytes: Option<&[u8]>,
    auth_context: &RequestAuthContext,
    bedrock_service: Option<BedrockService>,
) -> Result<reqwest::Response, RequestExecutionError> {
    #[cfg(feature = "provider-bedrock")]
    if let Some(signing) = auth_context.bedrock_signing.as_ref()
        && !request.headers().contains_key(AUTHORIZATION)
        && !request.headers().contains_key("x-api-key")
    {
        let Some(service) = bedrock_service else {
            return Err(RequestExecutionError::Setup(
                "bedrock request missing service classification for SigV4 signing".to_owned(),
            ));
        };
        request = sign_bedrock_request(request, body_bytes.unwrap_or(&[]), signing, service)
            .map_err(RequestExecutionError::Setup)?;
    }

    client
        .execute(request)
        .await
        .map_err(RequestExecutionError::Transport)
}

pub(super) async fn decode_response_body(response: reqwest::Response) -> CliResult<Value> {
    let status = response.status().as_u16();
    let content_encoding = response
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none")
        .to_owned();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none")
        .to_owned();
    let bytes = response.bytes().await.map_err(|error| {
        format!(
            "read response body failed: {error} [status={status}, content-encoding={content_encoding}, content-type={content_type}]"
        )
    })?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({"raw_body": text.as_ref()})))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn decode_streaming_response(
    response: reqwest::Response,
) -> impl Stream<Item = Result<Bytes, RequestExecutionError>> + Unpin {
    response
        .bytes_stream()
        .map(|result: Result<Bytes, reqwest::Error>| {
            result.map_err(RequestExecutionError::Transport)
        })
}

async fn resolve_bedrock_region(provider: &ProviderConfig) -> CliResult<String> {
    let derived_endpoint = provider.endpoint();
    let derived_models_endpoint = provider.models_endpoint();
    for candidate in [
        provider.endpoint.as_deref(),
        provider.models_endpoint.as_deref(),
        Some(provider.base_url.as_str()),
        Some(derived_endpoint.as_str()),
        Some(derived_models_endpoint.as_str()),
    ] {
        if let Some(region) = candidate.and_then(extract_bedrock_region_from_url) {
            return Ok(region);
        }
    }

    for key in ["BEDROCK_AWS_REGION", "AWS_REGION", "AWS_DEFAULT_REGION"] {
        if let Some(region) = std::env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            return Ok(region);
        }
    }

    #[cfg(feature = "provider-bedrock")]
    {
        let chain = DefaultRegionChain::builder().build();
        if let Some(region) = chain.region().await {
            return Ok(region.as_ref().to_owned());
        }
    }

    Err(
        "Bedrock region could not be resolved from endpoint configuration, environment, or the AWS default region chain"
            .to_owned(),
    )
}

fn extract_bedrock_region_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    extract_bedrock_region_from_host(parsed.host_str()?)
}

fn extract_bedrock_region_from_host(host: &str) -> Option<String> {
    for prefix in ["bedrock-runtime.", "bedrock."] {
        let Some(rest) = host.strip_prefix(prefix) else {
            continue;
        };
        for suffix in [".amazonaws.com", ".amazonaws.com.cn"] {
            let Some(region) = rest.strip_suffix(suffix) else {
                continue;
            };
            let trimmed = region.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

#[cfg(feature = "provider-bedrock")]
async fn resolve_bedrock_credentials(region: &str) -> CliResult<Credentials> {
    let provider = DefaultCredentialsChain::builder()
        .region(Region::new(region.to_owned()))
        .build()
        .await;
    provider
        .provide_credentials()
        .await
        .map_err(|error| format!("Bedrock AWS credential chain resolution failed: {error}"))
}

#[cfg(feature = "provider-bedrock")]
fn sign_bedrock_request(
    mut request: reqwest::Request,
    body_bytes: &[u8],
    signing: &BedrockSigningContext,
    service: BedrockService,
) -> CliResult<reqwest::Request> {
    let uri = request.url().as_str().to_owned();
    let signable_headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            value
                .to_str()
                .map(|value| (name.as_str(), value))
                .map_err(|error| {
                    format!(
                        "bedrock request header `{}` is not valid UTF-8 for SigV4 signing: {error}",
                        name.as_str()
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let signable_request = SignableRequest::new(
        request.method().as_str(),
        uri.as_str(),
        signable_headers.iter().copied(),
        SignableBody::Bytes(body_bytes),
    )
    .map_err(|error| format!("construct Bedrock SigV4 signable request failed: {error}"))?;

    let identity = signing.credentials.clone().into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(signing.region.as_str())
        .name(service.signing_name())
        .time(SystemTime::now())
        .settings(SigningSettings::default())
        .build()
        .map_err(|error| format!("build Bedrock SigV4 signing params failed: {error}"))?
        .into();
    let (instructions, _signature) = http_request::sign(signable_request, &signing_params)
        .map_err(|error| format!("Bedrock request signing failed: {error}"))?
        .into_parts();

    let mut http_request = http::Request::builder()
        .method(request.method().clone())
        .uri(uri.as_str())
        .body(())
        .map_err(|error| format!("construct intermediate Bedrock HTTP request failed: {error}"))?;
    for (name, value) in request.headers() {
        http_request
            .headers_mut()
            .insert(name.clone(), value.clone());
    }
    instructions.apply_to_request_http1x(&mut http_request);

    let signed_url = reqwest::Url::parse(http_request.uri().to_string().as_str())
        .map_err(|error| format!("apply signed Bedrock URI failed: {error}"))?;
    *request.url_mut() = signed_url;

    let mut signed_headers = HeaderMap::new();
    for (name, value) in http_request.headers() {
        signed_headers.insert(
            HeaderName::from_bytes(name.as_str().as_bytes())
                .map_err(|error| format!("convert signed header name `{name}` failed: {error}"))?,
            HeaderValue::from_bytes(value.as_bytes()).map_err(|error| {
                format!("convert signed header value for `{name}` failed: {error}")
            })?,
        );
    }
    *request.headers_mut() = signed_headers;
    Ok(request)
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        let char = char::from(byte);
        if char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.' | '~') {
            encoded.push(char);
        } else {
            encoded.push('%');
            encoded.push_str(format!("{byte:02X}").as_str());
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    use std::collections::BTreeMap;

    #[test]
    fn build_request_headers_without_provider_auth_preserves_manual_auth_headers() {
        let provider = ProviderConfig {
            kind: ProviderKind::Custom,
            headers: BTreeMap::from([
                ("authorization".to_owned(), "Token custom-auth".to_owned()),
                ("x-api-key".to_owned(), "custom-key".to_owned()),
            ]),
            ..ProviderConfig::default()
        };

        let headers =
            build_request_headers_without_provider_auth(&provider).expect("transport headers");
        assert_eq!(
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Token custom-auth")
        );
        assert_eq!(
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("custom-key")
        );
    }

    #[test]
    fn render_transport_route_hint_identifies_dns_failures() {
        let hint = render_transport_route_hint(
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            "dns error: failed to lookup address information: nodename nor servname provided, or not known",
            false,
            true,
        )
        .expect("dns failure should surface a route hint");

        assert!(hint.contains("ark.cn-beijing.volces.com:443"));
        assert!(hint.contains("dns"));
        assert!(hint.contains("loong doctor"));
    }

    #[test]
    fn render_transport_route_hint_identifies_proxy_timeout_failures() {
        let hint = render_transport_route_hint(
            "https://api.openai.com/v1/chat/completions",
            "operation timed out",
            true,
            false,
        )
        .expect("timeouts should surface a route hint");

        assert!(hint.contains("api.openai.com:443"));
        assert!(hint.contains("proxy"));
        assert!(hint.contains("TUN"));
    }

    #[test]
    fn render_transport_route_hint_does_not_treat_tuning_as_proxy_route_failure() {
        let hint = render_transport_route_hint(
            "https://api.openai.com/v1/chat/completions",
            "provider tuning metadata could not be loaded",
            false,
            false,
        );

        assert!(
            hint.is_none(),
            "unrelated words like `tuning` should not be classified as proxy/TUN transport failures: {hint:#?}"
        );
    }

    #[cfg(feature = "provider-bedrock")]
    #[tokio::test]
    async fn resolve_request_auth_context_keeps_bedrock_sigv4_fallback_with_bearer_secret() {
        let mut env = ScopedEnv::new();
        env.set("AWS_ACCESS_KEY_ID", "test-access-key");
        env.set("AWS_SECRET_ACCESS_KEY", "test-secret-key");
        env.set("AWS_REGION", "us-west-2");
        env.remove("AWS_SESSION_TOKEN");

        let provider = ProviderConfig {
            kind: ProviderKind::Bedrock,
            api_key: Some(loongclaw_contracts::SecretRef::Inline(
                "bedrock-bearer-token".to_owned(),
            )),
            ..ProviderConfig::default()
        };

        let auth_context = resolve_request_auth_context(&provider)
            .await
            .expect("bedrock auth context");
        assert_eq!(auth_context.bedrock_region.as_deref(), Some("us-west-2"));
        assert!(auth_context.bedrock_signing.is_some());
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_extracts_data_field() {
        let line = "data: {\"type\":\"content_block_delta\",\"text\":\"Hello\"}";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::Data { content } => {
                assert_eq!(
                    content,
                    "{\"type\":\"content_block_delta\",\"text\":\"Hello\"}"
                );
            }
            other => {
                panic!("expected SseLine::Data, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_extracts_event_type() {
        let line = "event: content_block_delta";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::EventType { name } => {
                assert_eq!(name.as_str(), "content_block_delta");
            }
            other => {
                panic!("expected SseLine::EventType, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_extracts_retry_field() {
        let line = "retry: 1000";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::Retry { timeout_ms } => {
                assert_eq!(timeout_ms, 1000);
            }
            other => {
                panic!("expected SseLine::Retry, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_handles_empty_line() {
        let parsed = parse_sse_line("");
        match parsed {
            SseLine::Empty => {}
            other => {
                panic!("expected SseLine::Empty, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_handles_comment_line() {
        let parsed = parse_sse_line(": this is a comment");
        match parsed {
            SseLine::Comment => {}
            other => {
                panic!("expected SseLine::Comment, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_line_parser_data_field_without_json_value() {
        let line = "data:";
        let parsed = parse_sse_line(line);
        match parsed {
            SseLine::Data { content } => {
                assert_eq!(content, "");
            }
            other => {
                panic!("expected SseLine::Data, got {:?}", other)
            }
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    #[test]
    fn sse_lines_accumulate_into_complete_event() {
        let event_type_line = parse_sse_line("event: content_block_delta");
        let data_line = parse_sse_line("data: {\"type\":\"text_delta\",\"text\":\"Hello\"}");

        let (event_type, data) = match (&event_type_line, &data_line) {
            (SseLine::EventType { name: e1 }, SseLine::Data { content: d2 }) => {
                (e1.clone(), d2.clone())
            }
            _ => panic!("expected EventType and Data"),
        };

        assert_eq!(event_type.as_str(), "content_block_delta");
        assert_eq!(data, "{\"type\":\"text_delta\",\"text\":\"Hello\"}");
    }

    #[test]
    fn sse_stream_event_from_lines_parses_json() {
        let event_type = Some("content_block_delta".to_owned());
        let data_lines = vec!["{\"type\":\"text_delta\",\"text\":\"Hello\"}".to_owned()];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);

        match event {
            Ok(Some(SseStreamEvent::Message { data, event_type })) => {
                assert_eq!(event_type.as_deref(), Some("content_block_delta"));
                assert_eq!(
                    data.get("type").and_then(|v| v.as_str()),
                    Some("text_delta")
                );
                assert_eq!(data.get("text").and_then(|v| v.as_str()), Some("Hello"));
            }
            Ok(Some(SseStreamEvent::Error { .. } | SseStreamEvent::Done)) | Err(_) | Ok(None) => {
                panic!("expected SseStreamEvent::Message, got {:?}", event)
            }
        }
    }

    #[test]
    fn sse_stream_event_from_lines_returns_none_for_empty_data() {
        let event_type = Some("content_block_delta".to_owned());
        let data_lines: Vec<String> = vec![];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);
        assert!(event.unwrap().is_none());
    }

    #[test]
    fn sse_stream_event_from_lines_returns_err_for_invalid_json() {
        let event_type = Some("content_block_delta".to_owned());
        let data_lines = vec!["not valid json".to_owned()];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);
        assert!(event.is_err());
    }
}
