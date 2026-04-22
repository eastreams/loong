use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

use loong_protocol::{
    ControlPlaneConnectRequest, ControlPlanePairingListResponse, ControlPlanePairingResolveRequest,
    ControlPlanePairingResolveResponse,
};
use reqwest::blocking::Client as BlockingClient;
use reqwest::{Client, Method, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::CliResult;

use super::{
    read_models::{
        GatewayOperatorSummaryReadModel, GatewayPairingCompleteReadModel,
        GatewayPairingEventsReadModel, GatewayPairingSessionReadModel,
        GatewayPairingStartReadModel,
    },
    state::{
        GatewayOwnerStatus, default_gateway_runtime_state_dir, gateway_control_token_path,
        load_gateway_owner_status,
    },
};

const DEFAULT_GATEWAY_BOOTSTRAP_HOST: &str = "127.0.0.1";
const DEFAULT_GATEWAY_BOOTSTRAP_CONNECT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(500);
const DEFAULT_GATEWAY_BOOTSTRAP_REQUEST_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(2);

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

#[derive(Debug, Clone, Default, Serialize)]
pub struct GatewayPairingRequestsRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GatewayPairingEventsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayPairingStaleCursorReplayWindow {
    pub oldest_retained_seq: Option<u64>,
    pub latest_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayPairingStaleCursorError {
    pub code: String,
    pub message: String,
    pub last_acknowledged_seq: Option<u64>,
    pub earliest_resumable_after_seq: u64,
    pub replay_window: GatewayPairingStaleCursorReplayWindow,
}

impl GatewayPairingStaleCursorError {
    pub fn resume_after_seq(&self) -> u64 {
        self.earliest_resumable_after_seq
    }
}

#[derive(Debug, Clone)]
pub enum GatewayPairingEventsOutcome {
    Events(GatewayPairingEventsReadModel),
    StaleCursor(GatewayPairingStaleCursorError),
}

impl GatewayPairingEventsOutcome {
    pub fn events(&self) -> Option<&GatewayPairingEventsReadModel> {
        match self {
            Self::Events(events) => Some(events),
            Self::StaleCursor(_) => None,
        }
    }

    pub fn stale_cursor(&self) -> Option<&GatewayPairingStaleCursorError> {
        match self {
            Self::Events(_) => None,
            Self::StaleCursor(error) => Some(error),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GatewayLocalDiscovery {
    runtime_dir: PathBuf,
    owner_status: GatewayOwnerStatus,
    socket_address: SocketAddr,
    base_url: String,
    bearer_token: String,
    source: GatewayLocalDiscoverySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayLocalDiscoverySource {
    OwnerState,
    DefaultBootstrap,
    OwnerStateFallback,
}

impl GatewayLocalDiscovery {
    pub fn discover_default() -> CliResult<Self> {
        let runtime_dir = default_gateway_runtime_state_dir();
        let bootstrap_address = default_gateway_bootstrap_socket_address();
        discover_prefer_bootstrap(runtime_dir.as_path(), bootstrap_address)
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
            source: GatewayLocalDiscoverySource::OwnerState,
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

    pub fn source(&self) -> GatewayLocalDiscoverySource {
        self.source
    }

    fn bearer_token(&self) -> &str {
        self.bearer_token.as_str()
    }
}

fn discover_prefer_bootstrap(
    runtime_dir: &Path,
    bootstrap_address: SocketAddr,
) -> CliResult<GatewayLocalDiscovery> {
    match discover_with_bootstrap(runtime_dir, bootstrap_address) {
        Ok(discovery) => Ok(discovery),
        Err(bootstrap_error) => match GatewayLocalDiscovery::discover(runtime_dir) {
            Ok(mut discovery) => {
                discovery.source = GatewayLocalDiscoverySource::OwnerStateFallback;
                Ok(discovery)
            }
            Err(owner_state_error) => Err(format!(
                "gateway bootstrap discovery failed in {}: {bootstrap_error}; owner-state fallback failed: {owner_state_error}",
                runtime_dir.display()
            )),
        },
    }
}

fn discover_with_bootstrap(
    runtime_dir: &Path,
    bootstrap_address: SocketAddr,
) -> CliResult<GatewayLocalDiscovery> {
    let token_path = gateway_control_token_path(runtime_dir);
    let bearer_token = load_gateway_bearer_token(token_path.as_path())?;
    let owner_status =
        request_gateway_owner_status_from_bootstrap(bootstrap_address, bearer_token.as_str())?;
    let socket_address = validate_gateway_local_owner_status(&owner_status)?;
    let base_url = format!("http://{socket_address}");

    Ok(GatewayLocalDiscovery {
        runtime_dir: runtime_dir.to_path_buf(),
        owner_status,
        socket_address,
        base_url,
        bearer_token,
        source: GatewayLocalDiscoverySource::DefaultBootstrap,
    })
}

fn default_gateway_bootstrap_socket_address() -> SocketAddr {
    let gateway_config = crate::mvp::config::GatewayConfig::default();
    let host = DEFAULT_GATEWAY_BOOTSTRAP_HOST
        .parse::<IpAddr>()
        .expect("default gateway bootstrap host should stay valid");
    SocketAddr::new(host, gateway_config.port)
}

fn request_gateway_owner_status_from_bootstrap(
    socket_address: SocketAddr,
    bearer_token: &str,
) -> CliResult<GatewayOwnerStatus> {
    let bootstrap_url = format!("http://{socket_address}/v1/status");
    let http_client = BlockingClient::builder()
        .connect_timeout(DEFAULT_GATEWAY_BOOTSTRAP_CONNECT_TIMEOUT)
        .timeout(DEFAULT_GATEWAY_BOOTSTRAP_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("build gateway bootstrap client failed: {error}"))?;
    let response = http_client
        .get(bootstrap_url.as_str())
        .bearer_auth(bearer_token)
        .send()
        .map_err(|error| {
            format!("gateway bootstrap request failed for {bootstrap_url}: {error}")
        })?;
    let status = response.status();
    if !status.is_success() {
        let response_text = response
            .text()
            .unwrap_or_else(|_| "unable to read gateway bootstrap error body".to_owned());
        return Err(format!(
            "gateway bootstrap request failed for {bootstrap_url} with status {status}: {}",
            response_text.trim()
        ));
    }
    response.json::<GatewayOwnerStatus>().map_err(|error| {
        format!("decode gateway bootstrap response failed for {bootstrap_url}: {error}")
    })
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

    pub async fn nodes(&self) -> CliResult<Value> {
        let path = "/v1/nodes";
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

    pub async fn pairing_requests(
        &self,
        request: &GatewayPairingRequestsRequest<'_>,
    ) -> CliResult<ControlPlanePairingListResponse> {
        let path = "/v1/pairing/requests";
        self.request_json_with_query(Method::GET, path, request)
            .await
    }

    pub async fn pairing_start(&self) -> CliResult<GatewayPairingStartReadModel> {
        let path = "/v1/pairing/start";
        self.request_json(Method::POST, path).await
    }

    pub async fn pairing_resolve(
        &self,
        request: &ControlPlanePairingResolveRequest,
    ) -> CliResult<ControlPlanePairingResolveResponse> {
        let path = "/v1/pairing/resolve";
        let endpoint = self.endpoint_url(path)?;
        let request_builder = self.http_client.post(endpoint.as_str());
        let request_builder = request_builder.bearer_auth(self.discovery.bearer_token());
        let request_builder = request_builder.json(request);
        let response = self
            .send_gateway_request(request_builder, endpoint.as_str())
            .await?;
        self.decode_gateway_json_response(response, endpoint.as_str(), "POST", path)
            .await
    }

    pub async fn pairing_complete(
        &self,
        request: &ControlPlaneConnectRequest,
    ) -> CliResult<GatewayPairingCompleteReadModel> {
        let path = "/v1/pairing/complete";
        let endpoint = self.endpoint_url(path)?;
        let request_builder = self.http_client.post(endpoint.as_str());
        let request_builder = request_builder.bearer_auth(self.discovery.bearer_token());
        let request_builder = request_builder.json(request);
        let response = self
            .send_gateway_request(request_builder, endpoint.as_str())
            .await?;
        self.decode_gateway_json_response(response, endpoint.as_str(), "POST", path)
            .await
    }
    pub async fn pairing_session(
        &self,
        session_token: &str,
    ) -> CliResult<GatewayPairingSessionReadModel> {
        let path = "/v1/pairing/session";
        self.request_pairing_json(Method::GET, path, session_token)
            .await
    }

    pub async fn pairing_events(
        &self,
        session_token: &str,
        request: &GatewayPairingEventsRequest,
    ) -> CliResult<GatewayPairingEventsOutcome> {
        let path = "/v1/pairing/events";
        let endpoint = self.endpoint_url(path)?;
        let request_builder = self.http_client.get(endpoint.as_str());
        let request_builder = request_builder.query(request);
        let request_builder = request_builder.bearer_auth(session_token);
        let response = self
            .send_gateway_request(request_builder, endpoint.as_str())
            .await?;
        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|error| format!("read gateway response failed for {endpoint}: {error}"))?;

        if status == reqwest::StatusCode::CONFLICT
            && let Ok(parsed_error) =
                serde_json::from_str::<GatewayPairingStaleCursorEnvelope>(response_text.as_str())
            && parsed_error.error.code == "stale_cursor"
        {
            return Ok(GatewayPairingEventsOutcome::StaleCursor(parsed_error.error));
        }

        if !status.is_success() {
            let error_message = decode_gateway_error_message_from_text(response_text.as_str());
            let error = format!("gateway GET {path} failed with status {status}: {error_message}");
            return Err(error);
        }

        let payload = serde_json::from_str::<GatewayPairingEventsReadModel>(response_text.as_str())
            .map_err(|error| format!("decode gateway response failed for {endpoint}: {error}"))?;
        Ok(GatewayPairingEventsOutcome::Events(payload))
    }

    pub async fn pairing_stream(
        &self,
        session_token: &str,
        after_seq: Option<u64>,
        limit: Option<usize>,
    ) -> CliResult<Response> {
        let path = "/v1/pairing/stream";
        let endpoint = self.endpoint_url(path)?;
        let mut query = Vec::new();
        if let Some(after_seq) = after_seq {
            query.push(("after_seq", after_seq.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        let request_builder = self.http_client.get(endpoint.as_str());
        let request_builder = request_builder.query(&query);
        let request_builder = request_builder.bearer_auth(session_token);
        self.send_gateway_request(request_builder, endpoint.as_str())
            .await
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

    async fn request_pairing_json<T>(
        &self,
        method: Method,
        path: &str,
        session_token: &str,
    ) -> CliResult<T>
    where
        T: DeserializeOwned,
    {
        let endpoint = self.endpoint_url(path)?;
        let method_name = method.as_str().to_owned();
        let request_builder = self.http_client.request(method, endpoint.as_str());
        let request_builder = request_builder.bearer_auth(session_token);
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
struct GatewayPairingStaleCursorEnvelope {
    error: GatewayPairingStaleCursorError,
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

    decode_gateway_error_message_from_text(response_text.as_str())
}

fn decode_gateway_error_message_from_text(response_text: &str) -> String {
    let parsed_error = serde_json::from_str::<GatewayErrorEnvelope>(response_text);
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
        collections::BTreeSet,
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::gateway::state::GatewayPortSource;
    use loong_protocol::{ControlPlanePrincipal, ControlPlaneRole, ControlPlaneScope};

    fn gateway_owner_status_fixture() -> GatewayOwnerStatus {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_millis() as u64;
        GatewayOwnerStatus {
            runtime_dir: "/tmp/loong-gateway-runtime".to_owned(),
            phase: "running".to_owned(),
            running: true,
            stale: false,
            pid: Some(42),
            mode: super::super::state::GatewayOwnerMode::GatewayHeadless,
            version: "0.1.0".to_owned(),
            config_path: "/tmp/loong.toml".to_owned(),
            attached_cli_session: None,
            started_at_ms: now_ms.saturating_sub(100),
            last_heartbeat_at: now_ms,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            configured_surface_count: 1,
            running_surface_count: 1,
            bind_address: Some("127.0.0.1".to_owned()),
            port: Some(7777),
            port_source: Some(GatewayPortSource::Default),
            token_path: Some("/tmp/loong-gateway-runtime/control-token".to_owned()),
        }
    }

    fn unique_temp_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir();
        temp_dir.join(format!("loong-gateway-client-{label}-{suffix}"))
    }

    fn write_gateway_token_for_test(runtime_dir: &Path, token: &str) -> PathBuf {
        let token_path = gateway_control_token_path(runtime_dir);
        if let Some(parent) = token_path.parent() {
            fs::create_dir_all(parent).expect("create gateway runtime dir");
        }
        fs::write(token_path.as_path(), token).expect("write gateway token");
        token_path
    }

    fn spawn_gateway_status_server_once(
        mut status: GatewayOwnerStatus,
        expected_token: &str,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind bootstrap server");
        let socket_address = listener.local_addr().expect("bootstrap server addr");
        if status.bind_address.is_none() {
            status.bind_address = Some("127.0.0.1".to_owned());
        }
        if status.port.is_none() {
            status.port = Some(socket_address.port());
        }
        let expected_header = format!("Authorization: Bearer {expected_token}");
        let server = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request_buffer = [0_u8; 8192];
                let read = stream
                    .read(&mut request_buffer)
                    .expect("read bootstrap request");
                let request_text = String::from_utf8_lossy(&request_buffer[..read]).into_owned();
                assert!(
                    request_text
                        .to_ascii_lowercase()
                        .contains(expected_header.to_ascii_lowercase().as_str()),
                    "expected bearer token in bootstrap request: {request_text}"
                );
                let body = serde_json::to_string(&status).expect("serialize gateway status");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write bootstrap response");
            }
        });
        (socket_address, server)
    }

    fn unused_loopback_socket_address() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral probe");
        let socket_address = listener.local_addr().expect("ephemeral probe addr");
        drop(listener);
        socket_address
    }

    fn spawn_gateway_json_server_once(
        status_line: &str,
        body: String,
    ) -> (SocketAddr, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind gateway json server");
        let socket_address = listener.local_addr().expect("gateway json server addr");
        let status_line = status_line.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept gateway json request");
            let mut request_buffer = [0_u8; 8192];
            let read = stream
                .read(&mut request_buffer)
                .expect("read gateway json request");
            let request_text = String::from_utf8_lossy(&request_buffer[..read]).into_owned();
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write gateway json response");
            request_text
        });
        (socket_address, server)
    }

    fn gateway_local_client_for_test(
        socket_address: SocketAddr,
        bearer_token: &str,
    ) -> GatewayLocalClient {
        let mut owner_status = gateway_owner_status_fixture();
        owner_status.port = Some(socket_address.port());
        let discovery = GatewayLocalDiscovery {
            runtime_dir: unique_temp_path("client-runtime"),
            owner_status,
            socket_address,
            base_url: format!("http://{socket_address}"),
            bearer_token: bearer_token.to_owned(),
            source: GatewayLocalDiscoverySource::DefaultBootstrap,
        };
        GatewayLocalClient::from_discovery(discovery)
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

    #[test]
    fn gateway_local_discovery_prefers_bootstrap_when_owner_status_is_missing() {
        let runtime_dir = unique_temp_path("bootstrap-runtime");
        fs::create_dir_all(runtime_dir.as_path()).expect("create runtime dir");
        let token_path = write_gateway_token_for_test(runtime_dir.as_path(), "bootstrap-token");
        let mut status = gateway_owner_status_fixture();
        status.token_path = Some(token_path.display().to_string());
        status.port = None;
        let (socket_address, server) = spawn_gateway_status_server_once(status, "bootstrap-token");

        let discovery =
            discover_prefer_bootstrap(runtime_dir.as_path(), socket_address).expect("bootstrap");

        assert_eq!(discovery.socket_address(), socket_address);
        assert_eq!(discovery.owner_status().port, Some(socket_address.port()));
        assert_eq!(
            discovery.source(),
            GatewayLocalDiscoverySource::DefaultBootstrap
        );

        server.join().expect("bootstrap server join");
        fs::remove_dir_all(runtime_dir).ok();
    }

    #[test]
    fn gateway_local_discovery_falls_back_to_owner_status_when_bootstrap_is_unavailable() {
        let runtime_dir = unique_temp_path("fallback-runtime");
        fs::create_dir_all(runtime_dir.as_path()).expect("create runtime dir");
        let token_path = write_gateway_token_for_test(runtime_dir.as_path(), "fallback-token");
        let mut status = gateway_owner_status_fixture();
        status.port = Some(7_777);
        status.token_path = Some(token_path.display().to_string());
        crate::gateway::state::write_gateway_owner_snapshot_for_test(
            runtime_dir.as_path(),
            &status,
        )
        .expect("write gateway owner snapshot");
        let unused_bootstrap = unused_loopback_socket_address();

        let discovery =
            discover_prefer_bootstrap(runtime_dir.as_path(), unused_bootstrap).expect("fallback");

        assert_eq!(discovery.socket_address().port(), 7_777);
        assert_eq!(discovery.owner_status().port, Some(7_777));
        assert_eq!(
            discovery.source(),
            GatewayLocalDiscoverySource::OwnerStateFallback
        );

        fs::remove_dir_all(runtime_dir).ok();
    }

    #[tokio::test]
    async fn gateway_local_client_pairing_session_uses_session_token() {
        let payload = GatewayPairingSessionReadModel {
            status: "active".to_owned(),
            connection_token_expires_at_ms: 1_700_000_000_000,
            principal: ControlPlanePrincipal {
                connection_id: "conn-1".to_owned(),
                client_id: "client-1".to_owned(),
                role: ControlPlaneRole::Operator,
                scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
                device_id: Some("device-1".to_owned()),
            },
            last_acknowledged_seq: Some(7),
            resume_status: "resumed".to_owned(),
            resume_from_after_seq: 7,
            earliest_resumable_after_seq: 4,
            replay_window: super::super::read_models::GatewayPairingReplayWindowReadModel {
                oldest_retained_seq: Some(5),
                latest_seq: Some(9),
            },
        };
        let (socket_address, server) =
            spawn_gateway_json_server_once("200 OK", serde_json::to_string(&payload).unwrap());
        let client = gateway_local_client_for_test(socket_address, "control-token");

        let session = client
            .pairing_session("session-token")
            .await
            .expect("pairing session payload");

        assert_eq!(session.resume_status, "resumed");
        assert_eq!(session.resume_from_after_seq, 7);

        let request_text = server.join().expect("join gateway json server");
        assert!(
            request_text.starts_with("GET /v1/pairing/session HTTP/1.1"),
            "unexpected request line: {request_text}"
        );
        let request_text_lower = request_text.to_ascii_lowercase();
        assert!(
            request_text_lower.contains("authorization: bearer session-token"),
            "expected session token auth header: {request_text}"
        );
        assert!(
            !request_text_lower.contains("authorization: bearer control-token"),
            "control token should not be reused for paired-session calls: {request_text}"
        );
    }

    #[tokio::test]
    async fn gateway_local_client_pairing_events_surfaces_stale_cursor_outcome() {
        let stale_cursor = serde_json::json!({
            "error": {
                "code": "stale_cursor",
                "message": "requested after_seq=0 is older than retained replay window 5..9",
                "last_acknowledged_seq": 7,
                "earliest_resumable_after_seq": 4,
                "replay_window": {
                    "oldest_retained_seq": 5,
                    "latest_seq": 9
                }
            }
        });
        let (socket_address, server) = spawn_gateway_json_server_once(
            "409 Conflict",
            serde_json::to_string(&stale_cursor).unwrap(),
        );
        let client = gateway_local_client_for_test(socket_address, "control-token");

        let outcome = client
            .pairing_events(
                "session-token",
                &GatewayPairingEventsRequest {
                    after_seq: Some(0),
                    limit: Some(10),
                    ack_seq: Some(7),
                },
            )
            .await
            .expect("stale cursor outcome");

        let stale_cursor = outcome
            .stale_cursor()
            .expect("stale cursor should surface as a recoverable outcome");
        assert_eq!(stale_cursor.resume_after_seq(), 4);
        assert_eq!(stale_cursor.last_acknowledged_seq, Some(7));
        assert_eq!(stale_cursor.replay_window.oldest_retained_seq, Some(5));
        assert_eq!(stale_cursor.replay_window.latest_seq, Some(9));

        let request_text = server.join().expect("join gateway json server");
        assert!(
            request_text
                .starts_with("GET /v1/pairing/events?after_seq=0&limit=10&ack_seq=7 HTTP/1.1"),
            "unexpected request line: {request_text}"
        );
        assert!(
            request_text
                .to_ascii_lowercase()
                .contains("authorization: bearer session-token"),
            "expected session token auth header: {request_text}"
        );
    }

    #[tokio::test]
    async fn gateway_local_client_pairing_events_decodes_event_payloads() {
        let payload = serde_json::json!({
            "after_seq": 7,
            "effective_after_seq": 7,
            "returned_count": 2,
            "last_acknowledged_seq": 9,
            "resume_status": "resumed",
            "next_after_seq": 9,
            "earliest_resumable_after_seq": 4,
            "replay_window": {
                "oldest_retained_seq": 5,
                "latest_seq": 9
            },
            "events": [
                {
                    "seq": 8,
                    "payload": {"event_type": "gateway.status"}
                },
                {
                    "seq": 9,
                    "payload": {"event_type": "gateway.turn.completed"}
                }
            ]
        });
        let (socket_address, server) =
            spawn_gateway_json_server_once("200 OK", serde_json::to_string(&payload).unwrap());
        let client = gateway_local_client_for_test(socket_address, "control-token");

        let outcome = client
            .pairing_events(
                "session-token",
                &GatewayPairingEventsRequest {
                    after_seq: Some(7),
                    limit: Some(10),
                    ack_seq: Some(9),
                },
            )
            .await
            .expect("event payload outcome");

        let events = outcome.events().expect("event payload");
        assert_eq!(events.returned_count, 2);
        assert_eq!(events.next_after_seq, 9);
        assert_eq!(events.last_acknowledged_seq, Some(9));
        assert_eq!(events.events.len(), 2);

        let request_text = server.join().expect("join gateway json server");
        assert!(
            request_text
                .starts_with("GET /v1/pairing/events?after_seq=7&limit=10&ack_seq=9 HTTP/1.1"),
            "unexpected request line: {request_text}"
        );
    }
}
