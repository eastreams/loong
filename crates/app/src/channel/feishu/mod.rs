use std::path::Path;
use std::sync::Arc;

use axum::{Router, routing::post};

use crate::CliResult;
use crate::KernelContext;
use crate::channel::{
    ChannelAdapter, ChannelOutboundTarget, FeishuChannelSendRequest,
    runtime_state::ChannelOperationRuntimeTracker,
};
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedFeishuChannelConfig,
};

pub mod adapter;
pub mod auth;
pub mod client;
pub mod error;
pub mod outbound;
pub mod payload;
pub mod principal;
pub mod resources;
pub mod runtime;
pub mod token_store;
pub mod webhook;
pub mod websocket;

use adapter::FeishuAdapter;
use payload::normalize_webhook_path;
use webhook::{FeishuWebhookState, feishu_webhook_handler};

pub use auth::{
    FEISHU_DOC_WRITE_ACCEPTED_SCOPES, FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES,
    FEISHU_MESSAGE_WRITE_RECOMMENDED_SCOPES, FeishuAuthStartSpec, FeishuGrantAnyScopeStatus,
    FeishuGrantStatus, FeishuTokenExchangeRequest, build_authorize_url, map_user_info_to_principal,
    parse_token_exchange_response, summarize_doc_write_scope_status,
    summarize_message_write_scope_status,
};
pub use client::{
    FeishuClient, FeishuUserInfo, FeishuWsEndpoint, FeishuWsEndpointClientConfig,
    parse_user_info_response,
};
pub use error::FeishuApiError;
pub use outbound::{
    FeishuOperatorOutboundMessageInput, parse_post_json_argument,
    resolve_operator_outbound_message_body, validate_operator_outbound_message_input,
};
pub use principal::{FeishuAccountBinding, FeishuGrantScopeSet, FeishuUserPrincipal};
pub use resources::types::{
    FeishuCalendarEntry, FeishuCalendarFreebusyResult, FeishuCalendarFreebusySlot,
    FeishuCalendarListPage, FeishuCardUpdateReceipt, FeishuDocumentContent, FeishuDocumentMetadata,
    FeishuDownloadedMessageResource, FeishuMessageDetail, FeishuMessageHistoryPage,
    FeishuMessageResourceType, FeishuMessageSummary, FeishuMessageWriteReceipt,
    FeishuPrimaryCalendarEntry, FeishuPrimaryCalendarList, FeishuSearchMessagePage,
    FeishuUploadedFile, FeishuUploadedImage,
};
pub use runtime::{
    FeishuGrantInventory, FeishuGrantResolution, describe_grant_selection_error,
    describe_grant_selection_error_for_display, effective_selected_open_id,
    ensure_fresh_user_grant, inspect_grants_for_account, resolve_grant_selection,
    resolve_requested_feishu_account, resolve_selected_grant, unix_ts_now,
};
pub use token_store::{
    FeishuGrant, FeishuOauthStateRecord, FeishuStoredOauthState, FeishuTokenStore,
};

pub async fn run_feishu_send(
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
    let message = adapter
        .resolve_operator_outbound_message(
            "loongclaw feishu-send",
            &crate::channel::feishu::FeishuOperatorOutboundMessageInput {
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
    adapter.send_message(&target, &message).await
}

#[allow(clippy::print_stdout)]
pub(crate) async fn run_feishu_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedFeishuChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
) -> CliResult<()> {
    if resolved.mode == crate::config::FeishuChannelServeMode::Websocket {
        return websocket::run_feishu_websocket_channel(
            config,
            resolved,
            resolved_path,
            selected_by_default,
            default_account_source,
            kernel_ctx,
            runtime,
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
        .await
        .map_err(|error| format!("feishu webhook server stopped: {error}"))
}

#[cfg(test)]
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

    async fn spawn_mock_feishu_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock feishu server");
        let address = listener.local_addr().expect("mock server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock feishu api");
        });
        (format!("http://{address}"), handle)
    }

    async fn record_request(State(state): State<MockServerState>, request: Request) {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .expect("read mock request body");
        state.requests.lock().await.push(MockRequest {
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(ToOwned::to_owned),
            authorization: parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: String::from_utf8(body.to_vec()).expect("mock request body utf8"),
        });
    }

    fn resolved_config(base_url: &str) -> ResolvedFeishuChannelConfig {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.account_id = Some("feishu_work".to_owned());
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("secret-123".to_owned());
        config.feishu.base_url = Some(base_url.to_owned());
        config.feishu.receive_id_type = "chat_id".to_owned();
        config.feishu.verification_token = Some("verify-token".to_owned());
        config.feishu.encrypt_key = Some("encrypt-key".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_demo".to_owned()];
        config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu test account")
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
        let (base_url, server) = spawn_mock_feishu_server(router).await;

        run_feishu_send(
            &resolved_config(&base_url),
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
        .await
        .expect("run feishu send");

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
