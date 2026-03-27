use async_trait::async_trait;

use crate::CliResult;
use crate::channel::{
    ChannelAdapter, ChannelInboundMessage, ChannelOutboundMessage, ChannelOutboundTarget,
    ChannelOutboundTargetKind, ChannelPlatform,
};
use crate::config::{FeishuIntegrationConfig, ResolvedFeishuChannelConfig};
use crate::feishu::FeishuClient;
use crate::feishu::resources::messages::{self, FeishuOutboundMessageBody};
use crate::feishu::{FeishuOperatorOutboundMessageInput, resolve_operator_outbound_message_body};

const FEISHU_CARD_MESSAGE_CONTENT_LIMIT_BYTES: usize = 30 * 1024;

pub(super) struct FeishuAdapter {
    client: FeishuClient,
    receive_id_type: String,
    tenant_access_token: Option<String>,
}

impl FeishuAdapter {
    pub(super) fn new(config: &ResolvedFeishuChannelConfig) -> CliResult<Self> {
        let app_id = config
            .app_id()
            .ok_or_else(|| "missing Feishu app id (feishu.app_id or env)".to_owned())?;
        let app_secret = config
            .app_secret()
            .ok_or_else(|| "missing Feishu app secret (feishu.app_secret or env)".to_owned())?;
        Ok(Self {
            client: FeishuClient::new(
                config.resolved_base_url(),
                app_id,
                app_secret,
                FeishuIntegrationConfig::default().request_timeout_s,
            )?,
            receive_id_type: config.receive_id_type.clone(),
            tenant_access_token: None,
        })
    }

    pub(super) async fn refresh_tenant_token(&mut self) -> CliResult<()> {
        self.tenant_access_token = Some(self.client.get_tenant_access_token().await?);
        Ok(())
    }

    pub(super) async fn resolve_operator_outbound_message(
        &self,
        action: &str,
        input: &FeishuOperatorOutboundMessageInput,
    ) -> CliResult<ChannelOutboundMessage> {
        let body = resolve_operator_outbound_message_body(
            action,
            &self.client,
            self.tenant_access_token()?,
            input,
        )
        .await?;
        Ok(channel_outbound_message_from_body(body))
    }

    fn tenant_access_token(&self) -> CliResult<&str> {
        self.tenant_access_token.as_deref().ok_or_else(|| {
            "feishu tenant token is missing, call refresh_tenant_token first".to_owned()
        })
    }

    fn feishu_body(message: &ChannelOutboundMessage) -> CliResult<FeishuOutboundMessageBody> {
        match message {
            ChannelOutboundMessage::Text(text) => messages::resolve_outbound_message_body(
                "feishu channel outbound send",
                "message.text",
                "message.as_card",
                "message.post",
                "message.image_key",
                "message.file_key",
                Some(text.as_str()),
                false,
                None,
                None,
                None,
            ),
            ChannelOutboundMessage::MarkdownCard(text) => messages::resolve_outbound_message_body(
                "feishu channel outbound send",
                "message.text",
                "message.as_card",
                "message.post",
                "message.image_key",
                "message.file_key",
                Some(text.as_str()),
                true,
                None,
                None,
                None,
            ),
            ChannelOutboundMessage::Post(post) => messages::resolve_outbound_message_body(
                "feishu channel outbound send",
                "message.text",
                "message.as_card",
                "message.post",
                "message.image_key",
                "message.file_key",
                None,
                false,
                Some(post),
                None,
                None,
            ),
            ChannelOutboundMessage::Image { image_key } => messages::resolve_outbound_message_body(
                "feishu channel outbound send",
                "message.text",
                "message.as_card",
                "message.post",
                "message.image_key",
                "message.file_key",
                None,
                false,
                None,
                Some(image_key.as_str()),
                None,
            ),
            ChannelOutboundMessage::File { file_key } => messages::resolve_outbound_message_body(
                "feishu channel outbound send",
                "message.text",
                "message.as_card",
                "message.post",
                "message.image_key",
                "message.file_key",
                None,
                false,
                None,
                None,
                Some(file_key.as_str()),
            ),
        }
    }

    async fn send_feishu_message(
        &self,
        target: &ChannelOutboundTarget,
        body: &FeishuOutboundMessageBody,
    ) -> CliResult<()> {
        if target.platform != ChannelPlatform::Feishu {
            return Err(format!(
                "feishu adapter cannot send to {} target",
                target.platform.as_str()
            ));
        }

        let token = self.tenant_access_token()?;
        match target.kind {
            ChannelOutboundTargetKind::MessageReply => {
                messages::reply_outbound_message(
                    &self.client,
                    token,
                    target.trimmed_id()?,
                    body,
                    target.feishu_reply_in_thread().unwrap_or(false),
                    target.idempotency_key(),
                )
                .await?;
                Ok(())
            }
            ChannelOutboundTargetKind::ReceiveId => {
                messages::send_outbound_message(
                    &self.client,
                    token,
                    target
                        .feishu_receive_id_type()
                        .unwrap_or(self.receive_id_type.as_str()),
                    target.trimmed_id()?,
                    body,
                    target.idempotency_key(),
                )
                .await?;
                Ok(())
            }
            ChannelOutboundTargetKind::Conversation
            | ChannelOutboundTargetKind::Address
            | ChannelOutboundTargetKind::Endpoint => {
                Err("feishu adapter only supports message_reply or receive_id targets".to_owned())
            }
        }
    }
}

fn channel_outbound_message_from_body(body: FeishuOutboundMessageBody) -> ChannelOutboundMessage {
    match body {
        FeishuOutboundMessageBody::Text(text) => ChannelOutboundMessage::Text(text),
        FeishuOutboundMessageBody::MarkdownCard(text) => ChannelOutboundMessage::MarkdownCard(text),
        FeishuOutboundMessageBody::Post(post) => ChannelOutboundMessage::Post(post),
        FeishuOutboundMessageBody::Image(image_key) => ChannelOutboundMessage::Image { image_key },
        FeishuOutboundMessageBody::File(file_key) => ChannelOutboundMessage::File { file_key },
    }
}

pub(super) fn outbound_reply_message_from_text(text: String) -> ChannelOutboundMessage {
    let trimmed_text = text.trim();
    if trimmed_text.is_empty() {
        return ChannelOutboundMessage::Text(text);
    }

    let reply_fits_markdown_card = reply_text_fits_markdown_card(trimmed_text);
    if reply_fits_markdown_card {
        let markdown_card_text = trimmed_text.to_owned();
        return ChannelOutboundMessage::MarkdownCard(markdown_card_text);
    }

    ChannelOutboundMessage::Text(text)
}

fn reply_text_fits_markdown_card(text: &str) -> bool {
    let card = crate::feishu::resources::cards::build_markdown_card(text);
    let encoded_card = match serde_json::to_string(&card) {
        Ok(encoded_card) => encoded_card,
        Err(_) => return false,
    };
    let encoded_card_len = encoded_card.len();
    encoded_card_len <= FEISHU_CARD_MESSAGE_CONTENT_LIMIT_BYTES
}

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
        Err("feishu inbound is served via `feishu-serve` (webhook or websocket mode)".to_owned())
    }

    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        message: &ChannelOutboundMessage,
    ) -> CliResult<()> {
        let body = Self::feishu_body(message)?;
        self.send_feishu_message(target, &body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LoongClawConfig;
    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        routing::post,
    };
    use serde_json::json;
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
        config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu test account")
    }

    #[tokio::test]
    async fn feishu_adapter_send_message_supports_post_receive_id_targets() {
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
                            Json(json!({
                                "code": 0,
                                "tenant_access_token": "t-token-channel-send-post"
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
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_channel_post_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token");

        ChannelAdapter::send_message(
            &adapter,
            &ChannelOutboundTarget::feishu_receive_id("oc_demo"),
            &ChannelOutboundMessage::Post(json!({
                "zh_cn": {
                    "title": "Channel post",
                    "content": [[{
                        "tag": "text",
                        "text": "rich channel"
                    }]]
                }
            })),
        )
        .await
        .expect("send post message");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(
            requests[1]
                .query
                .as_deref()
                .is_some_and(|query| query.contains("receive_id_type=chat_id"))
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-channel-send-post")
        );
        assert!(requests[1].body.contains("\"msg_type\":\"post\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"title\\\":\\\"Channel post\\\"")
        );

        server.abort();
    }

    #[tokio::test]
    async fn feishu_adapter_send_message_honors_receive_id_overrides_and_uuid() {
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
                                "tenant_access_token": "t-token-channel-send-override"
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
                                    "message_id": "om_channel_override_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token");

        ChannelAdapter::send_message(
            &adapter,
            &ChannelOutboundTarget::feishu_receive_id("ou_demo")
                .with_feishu_receive_id_type("open_id")
                .with_idempotency_key("send-uuid-override"),
            &ChannelOutboundMessage::Text("hello override".to_owned()),
        )
        .await
        .expect("send text with override");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(
            requests[1]
                .query
                .as_deref()
                .is_some_and(|query| query.contains("receive_id_type=open_id"))
        );
        assert!(requests[1].body.contains("\"uuid\":\"send-uuid-override\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"hello override\\\"")
        );

        server.abort();
    }

    #[tokio::test]
    async fn feishu_adapter_send_message_supports_image_replies() {
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
                            Json(json!({
                                "code": 0,
                                "tenant_access_token": "t-token-channel-reply-image"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_1/reply",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_channel_reply_image_1",
                                    "root_id": "om_parent_1",
                                    "parent_id": "om_parent_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token");

        ChannelAdapter::send_message(
            &adapter,
            &ChannelOutboundTarget::feishu_message_reply("om_parent_1"),
            &ChannelOutboundMessage::Image {
                image_key: "img_v2_demo".to_owned(),
            },
        )
        .await
        .expect("send image reply");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_1/reply"
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-channel-reply-image")
        );
        assert!(requests[1].body.contains("\"msg_type\":\"image\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"image_key\\\":\\\"img_v2_demo\\\"")
        );

        server.abort();
    }

    #[tokio::test]
    async fn feishu_adapter_send_message_supports_thread_replies() {
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
                            Json(json!({
                                "code": 0,
                                "tenant_access_token": "t-token-channel-thread-reply"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_thread/reply",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_channel_reply_thread_1",
                                    "root_id": "om_parent_thread",
                                    "parent_id": "om_parent_thread"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token");

        ChannelAdapter::send_message(
            &adapter,
            &ChannelOutboundTarget::feishu_message_reply("om_parent_thread")
                .with_feishu_reply_in_thread(true),
            &ChannelOutboundMessage::Text("threaded reply".to_owned()),
        )
        .await
        .expect("send threaded reply");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_thread/reply"
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-channel-thread-reply")
        );
        assert!(requests[1].body.contains("\"reply_in_thread\":true"));
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"threaded reply\\\"")
        );

        server.abort();
    }

    #[test]
    fn outbound_reply_message_from_text_prefers_markdown_cards_within_limit() {
        let reply_message = outbound_reply_message_from_text("## done\n\n- rendered".to_owned());

        assert_eq!(
            reply_message,
            ChannelOutboundMessage::MarkdownCard("## done\n\n- rendered".to_owned())
        );
    }

    #[test]
    fn outbound_reply_message_from_text_trims_markdown_cards_before_returning() {
        let reply_message =
            outbound_reply_message_from_text("  ## done\n\n- rendered  ".to_owned());

        assert_eq!(
            reply_message,
            ChannelOutboundMessage::MarkdownCard("## done\n\n- rendered".to_owned())
        );
    }

    #[test]
    fn outbound_reply_message_from_text_respects_card_limit_boundary() {
        let fitting_reply_len = max_reply_text_len_for_markdown_card();
        let fitting_reply = "a".repeat(fitting_reply_len);
        let overflowing_reply = format!("{fitting_reply}a");
        let fitting_message = outbound_reply_message_from_text(fitting_reply.clone());
        let overflowing_message = outbound_reply_message_from_text(overflowing_reply.clone());

        assert_eq!(
            fitting_message,
            ChannelOutboundMessage::MarkdownCard(fitting_reply)
        );
        assert_eq!(
            overflowing_message,
            ChannelOutboundMessage::Text(overflowing_reply)
        );
    }

    fn max_reply_text_len_for_markdown_card() -> usize {
        let empty_card = crate::feishu::resources::cards::build_markdown_card("");
        let encoded_empty_card =
            serde_json::to_string(&empty_card).expect("encode empty markdown card");
        let empty_card_len = encoded_empty_card.len();

        FEISHU_CARD_MESSAGE_CONTENT_LIMIT_BYTES.saturating_sub(empty_card_len)
    }
}
