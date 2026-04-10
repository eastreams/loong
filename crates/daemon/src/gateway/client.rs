use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

use reqwest::{Client, Method, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::CliResult;

use super::{
    read_models::GatewayOperatorSummaryReadModel,
    state::{GatewayOwnerStatus, default_gateway_runtime_state_dir, load_gateway_owner_status},
};

#[derive(Debug, Clone, Default, Serialize)]
pub struct GatewayAcpSessionsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GatewayAcpStatusRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_session_id: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct GatewayLocalDiscovery {
    runtime_dir: PathBuf,
    owner_status: GatewayOwnerStatus,
    socket_address: SocketAddr,
    base_url: String,
    bearer_token: String,
}

impl GatewayLocalDiscovery {
    pub fn discover_default() -> CliResult<Self> {
        let runtime_dir = default_gateway_runtime_state_dir();
        Self::discover(runtime_dir.as_path())
    }

    pub fn discover(runtime_dir: &Path) -> CliResult<Self> {
        let owner_status = load_gateway_owner_status(runtime_dir);
        let Some(owner_status) = owner_status else {
            let runtime_dir_text = runtime_dir.display().to_string();
            let error = format!("gateway owner status is unavailable in {runtime_dir_text}");
            return Err(error);
        };

        let socket_address = validate_gateway_local_owner_status(&owner_status)?;
        let token_path = gateway_token_path_from_status(&owner_status)?;
        let bearer_token = load_gateway_bearer_token(token_path.as_path())?;
        let base_url = format!("http://{socket_address}");
        let runtime_dir = runtime_dir.to_path_buf();

        Ok(Self {
            runtime_dir,
            owner_status,
            socket_address,
            base_url,
            bearer_token,
        })
    }

    pub fn runtime_dir(&self) -> &Path {
        self.runtime_dir.as_path()
    }

    pub fn owner_status(&self) -> &GatewayOwnerStatus {
        &self.owner_status
    }

    pub fn socket_address(&self) -> SocketAddr {
        self.socket_address
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    fn bearer_token(&self) -> &str {
        self.bearer_token.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct GatewayLocalClient {
    discovery: GatewayLocalDiscovery,
    http_client: Client,
}

impl GatewayLocalClient {
    pub fn discover_default() -> CliResult<Self> {
        let discovery = GatewayLocalDiscovery::discover_default()?;
        Ok(Self::from_discovery(discovery))
    }

    pub fn discover(runtime_dir: &Path) -> CliResult<Self> {
        let discovery = GatewayLocalDiscovery::discover(runtime_dir)?;
        Ok(Self::from_discovery(discovery))
    }

    pub fn from_discovery(discovery: GatewayLocalDiscovery) -> Self {
        let http_client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            discovery,
            http_client,
        }
    }

    pub fn discovery(&self) -> &GatewayLocalDiscovery {
        &self.discovery
    }

    pub async fn status(&self) -> CliResult<GatewayOwnerStatus> {
        let path = "/v1/status";
        self.request_json(Method::GET, path).await
    }

    pub async fn channels(&self) -> CliResult<Value> {
        let path = "/v1/channels";
        self.request_json(Method::GET, path).await
    }

    pub async fn runtime_snapshot(&self) -> CliResult<Value> {
        let path = "/v1/runtime/snapshot";
        self.request_json(Method::GET, path).await
    }

    pub async fn operator_summary(&self) -> CliResult<GatewayOperatorSummaryReadModel> {
        let path = "/api/gateway/operator-summary";
        self.request_json(Method::GET, path).await
    }

    pub async fn acp_sessions(&self, request: &GatewayAcpSessionsRequest) -> CliResult<Value> {
        let path = "/api/gateway/acp/sessions";
        self.request_json_with_query(Method::GET, path, request)
            .await
    }

    pub async fn acp_status(&self, request: &GatewayAcpStatusRequest<'_>) -> CliResult<Value> {
        let path = "/api/gateway/acp/status";
        self.request_json_with_query(Method::GET, path, request)
            .await
    }

    pub async fn acp_observability(&self) -> CliResult<Value> {
        let path = "/v1/acp/observability";
        self.request_json(Method::GET, path).await
    }

    pub async fn acp_status_for_address(
        &self,
        session_id: &str,
        channel_id: Option<&str>,
        conversation_id: Option<&str>,
        account_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> CliResult<Value> {
        let path = "/v1/acp/status";
        let query = build_gateway_acp_address_query(
            session_id,
            channel_id,
            conversation_id,
            account_id,
            thread_id,
        );
        self.request_json_with_query(Method::GET, path, &query)
            .await
    }

    pub async fn acp_dispatch(
        &self,
        session_id: &str,
        channel_id: Option<&str>,
        conversation_id: Option<&str>,
        account_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> CliResult<Value> {
        let path = "/v1/acp/dispatch";
        let query = build_gateway_acp_address_query(
            session_id,
            channel_id,
            conversation_id,
            account_id,
            thread_id,
        );
        self.request_json_with_query(Method::GET, path, &query)
            .await
    }

    pub async fn stop(&self) -> CliResult<GatewayStopResponse> {
        let path = "/api/gateway/stop";
        self.request_json(Method::POST, path).await
    }

    pub async fn health(&self) -> CliResult<Value> {
        let url = format!("{}/health", self.discovery.base_url);
        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|error| format!("gateway health request failed: {error}"))?;
        parse_json_response(response).await
    }

    pub async fn turn(&self, session_id: &str, input: &str) -> CliResult<Value> {
        let url = format!("{}/v1/turn", self.discovery.base_url);
        let body = serde_json::json!({
            "session_id": session_id,
            "input": input,
        });
        let response = self
            .http_client
            .post(&url)
            .bearer_auth(&self.discovery.bearer_token)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("gateway turn request failed: {error}"))?;
        parse_json_response(response).await
    }

    async fn request_json<T>(&self, method: Method, path: &str) -> CliResult<T>
    where
        T: DeserializeOwned,
    {
        let endpoint = self.endpoint_url(path)?;
        let method_name = method.as_str().to_owned();
        let request_builder = self.http_client.request(method, endpoint.as_str());
        let request_builder = request_builder.bearer_auth(self.discovery.bearer_token());
        let response = self
            .send_gateway_request(request_builder, endpoint.as_str())
            .await?;
        self.decode_gateway_json_response(response, endpoint.as_str(), method_name.as_str(), path)
            .await
    }

    async fn request_json_with_query<T, Q>(
        &self,
        method: Method,
        path: &str,
        query: &Q,
    ) -> CliResult<T>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        let endpoint = self.endpoint_url(path)?;
        let method_name = method.as_str().to_owned();
        let request_builder = self.http_client.request(method, endpoint.as_str());
        let request_builder = request_builder.query(query);
        let request_builder = request_builder.bearer_auth(self.discovery.bearer_token());
        let response = self
            .send_gateway_request(request_builder, endpoint.as_str())
            .await?;
        self.decode_gateway_json_response(response, endpoint.as_str(), method_name.as_str(), path)
            .await
    }

    async fn send_gateway_request(
        &self,
        request_builder: reqwest::RequestBuilder,
        endpoint: &str,
    ) -> CliResult<Response> {
        let response = request_builder
            .send()
            .await
            .map_err(|error| format!("send gateway request failed for {endpoint}: {error}"))?;
        Ok(response)
    }

    async fn decode_gateway_json_response<T>(
        &self,
        response: Response,
        endpoint: &str,
        method_name: &str,
        path: &str,
    ) -> CliResult<T>
    where
        T: DeserializeOwned,
    {
        let status = response.status();
        if !status.is_success() {
            let error_message = decode_gateway_error_message(response).await;
            let error = format!(
                "gateway {method_name} {path} failed with status {status}: {error_message}"
            );
            return Err(error);
        }

        response
            .json::<T>()
            .await
            .map_err(|error| format!("decode gateway response failed for {endpoint}: {error}"))
    }

    fn endpoint_url(&self, path: &str) -> CliResult<String> {
        if !path.starts_with('/') {
            let error = format!("gateway client path must start with `/`: {path}");
            return Err(error);
        }

        let base_url = self.discovery.base_url();
        let endpoint = format!("{base_url}{path}");
        Ok(endpoint)
    }
}

fn build_gateway_acp_address_query(
    session_id: &str,
    channel_id: Option<&str>,
    conversation_id: Option<&str>,
    account_id: Option<&str>,
    thread_id: Option<&str>,
) -> Vec<(String, String)> {
    let mut query = Vec::new();
    query.push(("session_id".to_owned(), session_id.to_owned()));

    let channel_id = trimmed_non_empty(channel_id);
    if let Some(channel_id) = channel_id {
        query.push(("channel_id".to_owned(), channel_id));
    }

    let conversation_id = trimmed_non_empty(conversation_id);
    if let Some(conversation_id) = conversation_id {
        query.push(("conversation_id".to_owned(), conversation_id));
    }

    let account_id = trimmed_non_empty(account_id);
    if let Some(account_id) = account_id {
        query.push(("account_id".to_owned(), account_id));
    }

    let thread_id = trimmed_non_empty(thread_id);
    if let Some(thread_id) = thread_id {
        query.push(("thread_id".to_owned(), thread_id));
    }

    query
}

fn trimmed_non_empty(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

async fn parse_json_response(response: Response) -> CliResult<Value> {
    let status = response.status();
    if !status.is_success() {
        let error_message = decode_gateway_error_message(response).await;
        return Err(format!(
            "gateway request failed with status {status}: {error_message}"
        ));
    }
    response
        .json::<Value>()
        .await
        .map_err(|error| format!("decode gateway JSON response failed: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayStopResponseOutcome {
    Requested,
    AlreadyRequested,
    AlreadyStopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayStopResponse {
    pub outcome: GatewayStopResponseOutcome,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct GatewayErrorEnvelope {
    error: GatewayErrorBody,
}

#[derive(Debug, Deserialize)]
struct GatewayErrorBody {
    code: String,
    message: String,
}

fn validate_gateway_local_owner_status(status: &GatewayOwnerStatus) -> CliResult<SocketAddr> {
    if status.stale {
        return Err("gateway owner status is stale".to_owned());
    }
    if !status.running {
        return Err("gateway owner is not running".to_owned());
    }

    let bind_address = status
        .bind_address
        .as_deref()
        .ok_or_else(|| "gateway owner status is missing bind_address".to_owned())?;
    let port = status
        .port
        .ok_or_else(|| "gateway owner status is missing port".to_owned())?;
    let ip_address = bind_address.parse::<IpAddr>().map_err(|error| {
        format!("gateway owner bind_address is not a valid IP address: {error}")
    })?;

    if !ip_address.is_loopback() {
        let error = format!("gateway control surface must use loopback bind, found {bind_address}");
        return Err(error);
    }

    let socket_address = SocketAddr::new(ip_address, port);
    Ok(socket_address)
}

fn gateway_token_path_from_status(status: &GatewayOwnerStatus) -> CliResult<PathBuf> {
    let token_path = status
        .token_path
        .as_deref()
        .ok_or_else(|| "gateway owner status is missing token_path".to_owned())?;
    let token_path = PathBuf::from(token_path);
    Ok(token_path)
}

fn load_gateway_bearer_token(path: &Path) -> CliResult<String> {
    let token = fs::read_to_string(path).map_err(|error| {
        format!(
            "read gateway control token failed for {}: {error}",
            path.display()
        )
    })?;
    let token = token.trim().to_owned();

    if token.is_empty() {
        let error = format!("gateway control token is empty at {}", path.display());
        return Err(error);
    }

    Ok(token)
}

async fn decode_gateway_error_message(response: Response) -> String {
    let response_text = response.text().await;
    let response_text = match response_text {
        Ok(response_text) => response_text,
        Err(error) => {
            return format!("unable to read gateway error response: {error}");
        }
    };

    let parsed_error = serde_json::from_str::<GatewayErrorEnvelope>(response_text.as_str());
    if let Ok(parsed_error) = parsed_error {
        let code = parsed_error.error.code;
        let message = parsed_error.error.message;
        return format!("{code}: {message}");
    }

    let trimmed = response_text.trim();
    if trimmed.is_empty() {
        return "request failed without an error body".to_owned();
    }

    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn gateway_owner_status_fixture() -> GatewayOwnerStatus {
        GatewayOwnerStatus {
            runtime_dir: "/tmp/loongclaw-gateway-runtime".to_owned(),
            phase: "running".to_owned(),
            running: true,
            stale: false,
            pid: Some(42),
            mode: super::super::state::GatewayOwnerMode::GatewayHeadless,
            version: "0.1.0".to_owned(),
            config_path: "/tmp/loongclaw.toml".to_owned(),
            attached_cli_session: None,
            started_at_ms: 100,
            last_heartbeat_at: 200,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            configured_surface_count: 1,
            running_surface_count: 1,
            bind_address: Some("127.0.0.1".to_owned()),
            port: Some(7777),
            token_path: Some("/tmp/loongclaw-gateway-runtime/control-token".to_owned()),
        }
    }

    fn unique_temp_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir();
        temp_dir.join(format!("loongclaw-gateway-client-{label}-{suffix}"))
    }

    #[test]
    fn gateway_local_discovery_accepts_loopback_owner_status() {
        let status = gateway_owner_status_fixture();

        let socket_address =
            validate_gateway_local_owner_status(&status).expect("validate loopback owner status");

        assert_eq!(socket_address.to_string(), "127.0.0.1:7777");
    }

    #[test]
    fn gateway_local_discovery_rejects_stale_owner_status() {
        let mut status = gateway_owner_status_fixture();
        status.stale = true;
        status.running = false;

        let error = validate_gateway_local_owner_status(&status)
            .expect_err("stale gateway owner status should be rejected");

        assert!(error.contains("stale"), "unexpected error: {error}");
    }

    #[test]
    fn gateway_local_discovery_rejects_non_loopback_bind_address() {
        let mut status = gateway_owner_status_fixture();
        status.bind_address = Some("192.168.1.10".to_owned());

        let error = validate_gateway_local_owner_status(&status)
            .expect_err("non-loopback gateway bind should be rejected");

        assert!(error.contains("loopback"), "unexpected error: {error}");
    }

    #[test]
    fn gateway_local_discovery_loads_trimmed_bearer_token() {
        let token_path = unique_temp_path("token-trimmed");
        fs::write(token_path.as_path(), "abc123\n").expect("write token");

        let token =
            load_gateway_bearer_token(token_path.as_path()).expect("load gateway bearer token");

        assert_eq!(token, "abc123");

        fs::remove_file(token_path).ok();
    }

    #[test]
    fn gateway_local_discovery_rejects_empty_bearer_token() {
        let token_path = unique_temp_path("token-empty");
        fs::write(token_path.as_path(), "\n").expect("write empty token");

        let error = load_gateway_bearer_token(token_path.as_path())
            .expect_err("empty gateway token should be rejected");

        assert!(error.contains("empty"), "unexpected error: {error}");

        fs::remove_file(token_path).ok();
    }
}
