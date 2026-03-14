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
use reqwest::header::{
    AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT,
};
use serde_json::{Value, json};

use crate::CliResult;
use crate::config::{ProviderAuthScheme, ProviderConfig, ProviderKind};

use super::auth_profile_runtime::ProviderAuthProfile;

#[derive(Debug, Clone, Default)]
pub(super) struct RequestAuthContext {
    pub(super) bedrock_region: Option<String>,
    #[cfg(feature = "provider-bedrock")]
    bedrock_signing: Option<BedrockSigningContext>,
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

pub(super) async fn resolve_request_auth_context(
    provider: &ProviderConfig,
) -> CliResult<RequestAuthContext> {
    if provider.kind != ProviderKind::Bedrock {
        return Ok(RequestAuthContext::default());
    }

    let region = resolve_bedrock_region(provider).await?;
    if provider.resolved_auth_secret().is_some() {
        return Ok(RequestAuthContext {
            bedrock_region: Some(region),
            #[cfg(feature = "provider-bedrock")]
            bedrock_signing: None,
        });
    }

    #[cfg(feature = "provider-bedrock")]
    {
        let credentials = resolve_bedrock_credentials(region.as_str()).await?;
        Ok(RequestAuthContext {
            bedrock_region: Some(region.clone()),
            bedrock_signing: Some(BedrockSigningContext {
                region,
                credentials,
            }),
        })
    }

    #[cfg(not(feature = "provider-bedrock"))]
    {
        Err("bedrock provider family is disabled (enable feature `provider-bedrock`)".to_owned())
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

pub(super) fn build_request_headers_without_provider_auth(
    provider: &ProviderConfig,
) -> CliResult<HeaderMap> {
    let mut headers = build_request_headers(provider)?;
    headers.remove(AUTHORIZATION);
    headers.remove(HeaderName::from_static("x-api-key"));
    Ok(headers)
}

pub(super) fn build_request_headers(provider: &ProviderConfig) -> CliResult<HeaderMap> {
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
    if let Some(secret) = provider.resolved_auth_secret() {
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
    if let Some(signing) = auth_context.bedrock_signing.as_ref() {
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
