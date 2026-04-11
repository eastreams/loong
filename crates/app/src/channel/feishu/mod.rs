#[cfg(feature = "channel-feishu")]
use std::path::Path;
#[cfg(feature = "channel-feishu")]
use std::sync::Arc;

#[cfg(feature = "channel-feishu")]
use axum::{Router, routing::post};

#[cfg(feature = "channel-feishu")]
use crate::CliResult;
#[cfg(feature = "channel-feishu")]
use crate::KernelContext;
#[cfg(feature = "channel-feishu")]
use crate::channel::{
    ChannelOutboundTarget, ChannelServeStopHandle, FeishuChannelSendRequest,
    runtime::state::ChannelOperationRuntimeTracker,
};
#[cfg(feature = "channel-feishu")]
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedFeishuChannelConfig,
};

#[cfg(feature = "channel-feishu")]
mod adapter;
pub mod api;
#[cfg(feature = "channel-feishu")]
mod payload;
#[cfg(feature = "channel-feishu")]
pub(crate) mod send;
#[cfg(feature = "channel-feishu")]
mod webhook;
#[cfg(feature = "channel-feishu")]
mod websocket;

#[cfg(feature = "channel-feishu")]
use adapter::FeishuAdapter;
#[cfg(feature = "channel-feishu")]
use payload::normalize_webhook_path;
#[cfg(feature = "channel-feishu")]
use send::send_channel_message_via_message_send_api;
#[cfg(feature = "channel-feishu")]
use webhook::{FeishuWebhookState, feishu_webhook_handler};

const FEISHU_ALLOWLIST_ALL_SENTINEL: &str = "*";

pub(in crate::channel) fn feishu_allowlist_allows_all<'a, I>(allowed_chat_ids: I) -> bool
where
    I: IntoIterator<Item = &'a String>,
{
    allowed_chat_ids
        .into_iter()
        .any(|chat_id| chat_id.trim() == FEISHU_ALLOWLIST_ALL_SENTINEL)
}

pub(in crate::channel) fn feishu_allowlist_allows_chat<'a, I>(
    allowed_chat_ids: I,
    chat_id: &str,
) -> bool
where
    I: IntoIterator<Item = &'a String> + Clone,
{
    let chat_id = chat_id.trim();
    if chat_id.is_empty() {
        return false;
    }
    feishu_allowlist_allows_all(allowed_chat_ids.clone())
        || allowed_chat_ids
            .into_iter()
            .any(|allowed| allowed.trim() == chat_id)
}

#[cfg(feature = "channel-feishu")]
pub(super) async fn run_feishu_send(
    config: &ResolvedFeishuChannelConfig,
    request: &FeishuChannelSendRequest,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(config)?;
    adapter.refresh_tenant_token().await?;
    let mut target = ChannelOutboundTarget::feishu_receive_id(request.receive_id.clone());
    if let Some(receive_id_type) = request
        .receive_id_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        target = target.with_feishu_receive_id_type(receive_id_type.to_owned());
    }
    if let Some(uuid) = request
        .uuid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        target = target.with_idempotency_key(uuid.to_owned());
    }
    let action = format!("{} feishu-send", crate::config::active_cli_command_name());
    let message = adapter
        .resolve_operator_outbound_message(
            action.as_str(),
            &api::FeishuOperatorOutboundMessageInput {
                text: request.text.clone(),
                card: request.card,
                post_json: request.post_json.clone(),
                image_key: request.image_key.clone(),
                image_path: request.image_path.clone(),
                file_key: request.file_key.clone(),
                file_path: request.file_path.clone(),
                file_type: request.file_type.clone(),
            },
        )
        .await?;
    send_channel_message_via_message_send_api(&adapter, &target, message)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "channel-feishu")]
#[allow(clippy::print_stdout)] // CLI startup banner
pub(super) async fn run_feishu_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedFeishuChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    if resolved.mode == crate::config::FeishuChannelServeMode::Websocket {
        if bind_override
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || path_override
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        {
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "warning: feishu websocket mode does not open a local HTTP listener; --bind/--path are ignored"
                );
            }
        }
        return websocket::run_feishu_websocket_channel(
            config,
            resolved,
            resolved_path,
            selected_by_default,
            default_account_source,
            kernel_ctx,
            runtime,
            stop,
        )
        .await;
    }

    let mut adapter = FeishuAdapter::new(resolved)?;
    adapter.refresh_tenant_token().await?;

    let bind = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| resolved.webhook_bind.trim().to_owned());
    if bind.is_empty() {
        return Err("feishu webhook bind address is empty".to_owned());
    }

    let path = normalize_webhook_path(
        path_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(resolved.webhook_path.as_str()),
    );

    let state = FeishuWebhookState::new_with_resolved_path(
        config.clone(),
        resolved_path.to_path_buf(),
        resolved,
        adapter,
        kernel_ctx,
        runtime,
    );
    let app = Router::new()
        .route(path.as_str(), post(feishu_webhook_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind.as_str())
        .await
        .map_err(|error| format!("bind feishu webhook listener failed: {error}"))?;

    println!(
        "feishu channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, bind={}, path={})",
        resolved_path.display(),
        resolved.configured_account_id,
        resolved.account.label,
        selected_by_default,
        default_account_source.as_str(),
        bind,
        path
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            stop.wait().await;
        })
        .await
        .map_err(|error| format!("feishu webhook server stopped: {error}"))
}

#[cfg(all(test, feature = "channel-feishu"))]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        routing::post,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn set(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|value| value.to_string()).collect()
    }

    fn btree_set(ids: &[&str]) -> std::collections::BTreeSet<String> {
        ids.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn allowlist_allows_all_when_wildcard_present() {
        assert!(feishu_allowlist_allows_all(&set(&["oc_demo", "*"])));
        assert!(feishu_allowlist_allows_all(&set(&["  *  "])));
        assert!(!feishu_allowlist_allows_all(&set(&["oc_demo"])));
    }

    #[test]
    fn allowlist_allows_chat_for_exact_and_wildcard_matches() {
        let exact_only = set(&["oc_demo"]);
        assert!(feishu_allowlist_allows_chat(&exact_only, "oc_demo"));
        assert!(!feishu_allowlist_allows_chat(&exact_only, "oc_other"));
        assert!(!feishu_allowlist_allows_chat(&exact_only, " "));

        let spaced_exact_only = set(&["  oc_demo  "]);
        assert!(feishu_allowlist_allows_chat(&spaced_exact_only, "oc_demo"));
        assert!(!feishu_allowlist_allows_chat(
            &spaced_exact_only,
            "oc_other"
        ));

        let wildcard = set(&["*"]);
        assert!(feishu_allowlist_allows_chat(&wildcard, "oc_other"));
    }

    /// Regression tests with BTreeSet to ensure container-agnostic behavior.
    #[test]
    fn btree_set_allowlist_allows_all_when_wildcard_present() {
        assert!(feishu_allowlist_allows_all(&btree_set(&["oc_demo", "*"])));
        assert!(feishu_allowlist_allows_all(&btree_set(&["  *  "])));
        assert!(!feishu_allowlist_allows_all(&btree_set(&["oc_demo"])));
    }

    #[test]
    fn btree_set_allowlist_allows_chat_for_exact_and_wildcard_matches() {
        let exact_only = btree_set(&["oc_demo"]);
        assert!(feishu_allowlist_allows_chat(&exact_only, "oc_demo"));
        assert!(!feishu_allowlist_allows_chat(&exact_only, "oc_other"));
        assert!(!feishu_allowlist_allows_chat(&exact_only, " "));

        let spaced_exact_only = btree_set(&["  oc_demo  "]);
        assert!(feishu_allowlist_allows_chat(&spaced_exact_only, "oc_demo"));
        assert!(!feishu_allowlist_allows_chat(
            &spaced_exact_only,
            "oc_other"
        ));

        let wildcard = btree_set(&["*"]);
        assert!(feishu_allowlist_allows_chat(&wildcard, "oc_other"));
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        path: String,
        query: Option<String>,
        authorization: Option<String>,
        body: String,
    }

    #[derive(Clone, Default)]
    struct MockServerState {
        requests: Arc<Mutex<Vec<MockRequest>>>,
    }

    async fn spawn_mock_feishu_server(
        router: Router,
    ) -> CliResult<(String, tokio::task::JoinHandle<()>)> {
        let bind_result = tokio::net::TcpListener::bind("127.0.0.1:0").await;
        let listener = bind_result.map_err(|error| format!("bind mock feishu server: {error}"))?;
        let address_result = listener.local_addr();
        let address = address_result.map_err(|error| format!("mock server addr: {error}"))?;
        let handle = tokio::spawn(async move {
            let serve_result = axum::serve(listener, router).await;
            assert!(
                serve_result.is_ok(),
                "serve mock feishu api: {:?}",
                serve_result.err()
            );
        });
        let base_url = format!("http://{address}");
        Ok((base_url, handle))
    }

    async fn record_request(State(state): State<MockServerState>, request: Request) {
        let (parts, body) = request.into_parts();
        let body_result = to_bytes(body, usize::MAX).await;
        assert!(
            body_result.is_ok(),
            "read mock request body: {:?}",
            body_result.as_ref().err()
        );
        let body = match body_result {
            Ok(body) => body,
            Err(_) => return,
        };
        let body_text_result = String::from_utf8(body.to_vec());
        assert!(
            body_text_result.is_ok(),
            "mock request body utf8: {:?}",
            body_text_result.as_ref().err()
        );
        let body_text = match body_text_result {
            Ok(body_text) => body_text,
            Err(_) => return,
        };
        let authorization = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let path = parts.uri.path().to_owned();
        let query = parts.uri.query().map(ToOwned::to_owned);
        let request = MockRequest {
            path,
            query,
            authorization,
            body: body_text,
        };
        let mut requests = state.requests.lock().await;
        requests.push(request);
    }

    fn resolved_config(base_url: &str) -> CliResult<ResolvedFeishuChannelConfig> {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.account_id = Some("feishu_work".to_owned());
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "secret-123".to_owned(),
        ));
        config.feishu.base_url = Some(base_url.to_owned());
        config.feishu.receive_id_type = "chat_id".to_owned();
        config.feishu.verification_token = Some(loongclaw_contracts::SecretRef::Inline(
            "verify-token".to_owned(),
        ));
        config.feishu.encrypt_key = Some(loongclaw_contracts::SecretRef::Inline(
            "encrypt-key".to_owned(),
        ));
        config.feishu.allowed_chat_ids = vec!["oc_demo".to_owned()];
        let resolved_result = config.feishu.resolve_account(None);
        let resolved =
            resolved_result.map_err(|error| format!("resolve feishu test account: {error}"))?;
        Ok(resolved)
    }

    #[tokio::test]
    async fn run_feishu_send_supports_post_receive_id_overrides_and_uuid() {
        let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let state = MockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-run-feishu-send"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_run_feishu_send_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let server_result = spawn_mock_feishu_server(router).await;
        assert!(
            server_result.is_ok(),
            "spawn mock feishu server: {:?}",
            server_result.as_ref().err()
        );
        let (base_url, server) = match server_result {
            Ok(server) => server,
            Err(_) => return,
        };
        let config_result = resolved_config(&base_url);
        assert!(
            config_result.is_ok(),
            "resolve feishu test config: {:?}",
            config_result.as_ref().err()
        );
        let config = match config_result {
            Ok(config) => config,
            Err(_) => return,
        };

        let send_result = run_feishu_send(
            &config,
            &FeishuChannelSendRequest {
                receive_id: "ou_demo".to_owned(),
                receive_id_type: Some("open_id".to_owned()),
                post_json: Some(
                    "{\"zh_cn\":{\"title\":\"Channel send\",\"content\":[[{\"tag\":\"text\",\"text\":\"rich channel\"}]]}}"
                        .to_owned(),
                ),
                uuid: Some("channel-send-uuid-1".to_owned()),
                ..FeishuChannelSendRequest::default()
            },
        )
        .await;
        assert!(
            send_result.is_ok(),
            "run feishu send: {:?}",
            send_result.as_ref().err()
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(
            requests[1]
                .query
                .as_deref()
                .is_some_and(|query| query.contains("receive_id_type=open_id"))
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-run-feishu-send")
        );
        assert!(
            requests[1]
                .body
                .contains("\"uuid\":\"channel-send-uuid-1\"")
        );
        assert!(requests[1].body.contains("\"msg_type\":\"post\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"title\\\":\\\"Channel send\\\"")
        );

        server.abort();
    }
}
