use async_trait::async_trait;
use chrono::Utc;

use crate::CliResult;
use crate::channel::feishu::api::FeishuClient;
use crate::channel::feishu::api::messaging_api::{
    convert_cli_result, convert_feishu_message_to_generic, convert_message_content_to_feishu,
    convert_string_error_to_api_error, extract_receive_params, generate_idempotency_key,
};
use crate::channel::feishu::api::resources::docs::{
    convert_content_to_blocks, create_document as create_feishu_document, create_nested_blocks,
    fetch_document_content, fetch_document_metadata,
};
use crate::channel::feishu::api::resources::messages::{
    self, FeishuMessageHistoryQuery, FeishuOutboundMessageBody, fetch_message_detail,
    fetch_message_history, reply_outbound_message, send_outbound_message, update_card,
};
use crate::channel::feishu::api::{
    FeishuOperatorOutboundMessageInput, resolve_operator_outbound_message_body,
};
use crate::channel::traits::documents::{Document, DocumentContent, DocumentType, DocumentsApi};
use crate::channel::traits::error::{ApiError, ApiResult};
use crate::channel::traits::messaging::{
    Message, MessageContent, MessagingApi, PaginatedResult, Pagination, SendOptions,
};
use crate::channel::{
    ChannelAdapter, ChannelInboundMessage, ChannelOutboundMessage, ChannelOutboundTarget,
    ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
};
use crate::config::{FeishuIntegrationConfig, ResolvedFeishuChannelConfig};

const FEISHU_CARD_MESSAGE_CONTENT_LIMIT_BYTES: usize = 30 * 1024;
pub(super) const FEISHU_ACK_REACTIONS: &[&str] =
    &["THUMBSUP", "OK", "APPLAUSE", "MUSCLE", "DONE", "SMILE"];

fn pick_uniform_index(len: usize) -> usize {
    debug_assert!(len > 0);
    let upper = len as u64;
    let reject_threshold = (u64::MAX / upper) * upper;

    loop {
        let value = rand::random::<u64>();
        if value < reject_threshold {
            #[allow(clippy::cast_possible_truncation)]
            return (value % upper) as usize;
        }
    }
}

pub(super) fn random_feishu_ack_reaction() -> &'static str {
    let index = pick_uniform_index(FEISHU_ACK_REACTIONS.len());
    FEISHU_ACK_REACTIONS.get(index).unwrap_or(&"THUMBSUP")
}

pub(super) async fn add_message_reaction(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
    emoji_type: &str,
) -> CliResult<()> {
    let message_id = message_id.trim();
    if message_id.is_empty() {
        return Err("feishu reaction requires non-empty message_id".to_owned());
    }
    let emoji_type = emoji_type.trim();
    if emoji_type.is_empty() {
        return Err("feishu reaction requires non-empty emoji_type".to_owned());
    }

    let path = format!("/open-apis/im/v1/messages/{message_id}/reactions");
    let body = serde_json::json!({
        "reaction_type": {
            "emoji_type": emoji_type,
        }
    });
    let _ = client
        .post_json(path.as_str(), Some(tenant_access_token), &[], &body)
        .await?;
    Ok(())
}

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

    async fn append_document_content_with_token(
        &self,
        token: &str,
        document_id: &str,
        content: &DocumentContent,
    ) -> ApiResult<()> {
        let trimmed_document_id = document_id.trim();
        if trimmed_document_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Document ID cannot be empty".to_owned(),
            ));
        }

        let content_str = match content {
            DocumentContent::Text(text) => {
                if text.trim().is_empty() {
                    return Err(ApiError::InvalidRequest(
                        "Document content cannot be empty".to_owned(),
                    ));
                }
                text.as_str()
            }
            DocumentContent::Markdown(text) => {
                if text.trim().is_empty() {
                    return Err(ApiError::InvalidRequest(
                        "Document content cannot be empty".to_owned(),
                    ));
                }
                text.as_str()
            }
            DocumentContent::Binary(_) => {
                return Err(ApiError::NotSupported(
                    "Binary content not supported by Feishu documents".to_owned(),
                ));
            }
            DocumentContent::Json(_) => {
                return Err(ApiError::NotSupported(
                    "JSON content not supported by Feishu documents".to_owned(),
                ));
            }
        };

        let blocks = convert_cli_result(
            convert_content_to_blocks(&self.client, token, "markdown", content_str).await,
        )?;

        let insert_summary = convert_cli_result(
            create_nested_blocks(&self.client, token, trimmed_document_id, &blocks).await,
        )?;

        let _ = insert_summary;
        Ok(())
    }
}

fn channel_outbound_message_from_body(body: FeishuOutboundMessageBody) -> ChannelOutboundMessage {
    match body {
        FeishuOutboundMessageBody::Text(text) => ChannelOutboundMessage::Text(text),
        FeishuOutboundMessageBody::MarkdownCard(text) => ChannelOutboundMessage::MarkdownCard(text),
        FeishuOutboundMessageBody::Post(post) => ChannelOutboundMessage::Post(post),
        FeishuOutboundMessageBody::Image(image_key) => ChannelOutboundMessage::Image { image_key },
        FeishuOutboundMessageBody::File(file_key) => ChannelOutboundMessage::File { file_key },
        FeishuOutboundMessageBody::Audio(_)
        | FeishuOutboundMessageBody::Media { .. }
        | FeishuOutboundMessageBody::ShareChat(_)
        | FeishuOutboundMessageBody::ShareUser(_) => {
            // These types don't have corresponding ChannelOutboundMessage variants
            // Convert to text representation for now
            ChannelOutboundMessage::Text("[Unsupported message type]".to_owned())
        }
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

#[async_trait]
impl MessagingApi for FeishuAdapter {
    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        content: &MessageContent,
        _options: Option<SendOptions>,
    ) -> ApiResult<Message> {
        // Validate platform
        if target.platform != ChannelPlatform::Feishu {
            return Err(ApiError::InvalidRequest(
                "Target platform must be Feishu".to_owned(),
            ));
        }

        // Convert content to Feishu format
        let body = convert_message_content_to_feishu(content)?;

        // Get tenant access token
        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        // Extract receive parameters
        let (receive_id, receive_id_type) = extract_receive_params(target)?;

        // Use caller-provided idempotency key or generate one
        let uuid = target
            .idempotency_key()
            .filter(|key| !key.is_empty())
            .map(|key| key.to_owned())
            .or_else(|| Some(generate_idempotency_key()));

        // Send the message
        let receipt = convert_cli_result(
            send_outbound_message(
                &self.client,
                &token,
                &receive_id_type,
                &receive_id,
                &body,
                uuid.as_deref(),
            )
            .await,
        )?;

        // Build message from receipt data directly
        Ok(Message {
            id: receipt.message_id,
            session: ChannelSession::new(ChannelPlatform::Feishu, receive_id),
            sender_id: String::new(),
            content: content.clone(),
            timestamp: Utc::now(),
            parent_id: None,
            raw: None,
        })
    }

    async fn reply(
        &self,
        target: &ChannelOutboundTarget,
        content: &MessageContent,
        options: Option<SendOptions>,
    ) -> ApiResult<Message> {
        // Validate platform
        if target.platform != ChannelPlatform::Feishu {
            return Err(ApiError::InvalidRequest(
                "Target platform must be Feishu".to_owned(),
            ));
        }

        // For replies, the target ID should be the message_id
        let message_id = target.id.trim();
        if message_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Message ID is required for reply".to_owned(),
            ));
        }

        // Get tenant access token
        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        // Fetch parent message to get session info
        let parent_detail =
            convert_cli_result(fetch_message_detail(&self.client, &token, message_id).await)?;
        let parent_session = ChannelSession::new(
            ChannelPlatform::Feishu,
            parent_detail.chat_id.unwrap_or_default(),
        );

        // Convert content to Feishu format
        let body = convert_message_content_to_feishu(content)?;

        // Determine if we should reply in thread
        // Priority: SendOptions.reply_in_thread > target.feishu_reply_in_thread()
        let reply_in_thread = options
            .as_ref()
            .map(|o| o.reply_in_thread)
            .unwrap_or_else(|| target.feishu_reply_in_thread().unwrap_or(false));

        // Use caller-provided idempotency key or generate one
        let uuid = target
            .idempotency_key()
            .filter(|key| !key.is_empty())
            .map(|key| key.to_owned())
            .or_else(|| Some(generate_idempotency_key()));

        // Send the reply
        let receipt = convert_cli_result(
            reply_outbound_message(
                &self.client,
                &token,
                message_id,
                &body,
                reply_in_thread,
                uuid.as_deref(),
            )
            .await,
        )?;

        // Build the reply message
        Ok(Message {
            id: receipt.message_id,
            session: parent_session,
            sender_id: String::new(),
            content: content.clone(),
            timestamp: Utc::now(),
            parent_id: Some(message_id.to_owned()),
            raw: None,
        })
    }

    async fn get_message(&self, id: &str) -> ApiResult<Option<Message>> {
        let message_id = id.trim();
        if message_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Message ID cannot be empty".to_owned(),
            ));
        }

        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        match fetch_message_detail(&self.client, &token, message_id).await {
            Ok(detail) => {
                let message = convert_feishu_message_to_generic(detail)?;
                Ok(Some(message))
            }
            Err(err) => {
                let api_err = convert_string_error_to_api_error(&err);
                match api_err {
                    ApiError::NotFound(_) => Ok(None),
                    ApiError::Auth(_)
                    | ApiError::RateLimited { .. }
                    | ApiError::InvalidRequest(_)
                    | ApiError::Network(_)
                    | ApiError::Server(_)
                    | ApiError::NotSupported(_)
                    | ApiError::Platform { .. }
                    | ApiError::Other(_) => Err(api_err),
                }
            }
        }
    }

    async fn list_messages(
        &self,
        session: &ChannelSession,
        pagination: Option<Pagination>,
    ) -> ApiResult<PaginatedResult<Message>> {
        // Validate platform
        if session.platform != ChannelPlatform::Feishu {
            return Err(ApiError::InvalidRequest(
                "Session platform must be Feishu".to_owned(),
            ));
        }

        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        // Build the query
        let page_size = pagination
            .as_ref()
            .and_then(|p| p.limit)
            .map(|l| l.min(50))
            .unwrap_or(20);

        let query = FeishuMessageHistoryQuery {
            container_id_type: "chat".to_owned(),
            container_id: session.conversation_id.clone(),
            start_time: None,
            end_time: None,
            sort_type: Some("ByCreateTimeDesc".to_owned()),
            page_size: Some(page_size),
            page_token: pagination.and_then(|p| p.cursor),
        };

        let page = convert_cli_result(fetch_message_history(&self.client, &token, &query).await)?;

        // Convert directly from history items — no individual detail fetches needed
        let messages: Vec<Message> = page
            .items
            .into_iter()
            .filter_map(|detail| convert_feishu_message_to_generic(detail).ok())
            .collect();

        Ok(PaginatedResult {
            items: messages,
            has_more: page.has_more,
            next_cursor: page.page_token,
        })
    }

    async fn search_messages(
        &self,
        query: &str,
        _pagination: Option<Pagination>,
    ) -> ApiResult<PaginatedResult<Message>> {
        let search_query = query.trim();
        if search_query.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Search query cannot be empty".to_owned(),
            ));
        }

        // Note: Feishu message search requires user access token
        // This is a limitation - we need a user grant to search messages
        Err(ApiError::NotSupported(
            "Message search requires user access token (not yet implemented)".to_owned(),
        ))
    }

    async fn edit_message(&self, id: &str, content: &MessageContent) -> ApiResult<Message> {
        let message_id = id.trim();
        if message_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Message ID cannot be empty".to_owned(),
            ));
        }

        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        match content {
            MessageContent::Text { text } => {
                // Edit text message using PUT API
                let body = serde_json::json!({ "text": text });
                convert_cli_result(
                    messages::edit_message(&self.client, &token, message_id, "text", body).await,
                )?;
            }
            MessageContent::Markdown { text } => {
                // Update interactive card using PATCH API
                let card_content = serde_json::json!({
                    "config": { "wide_screen_mode": true },
                    "elements": [
                        {
                            "tag": "div",
                            "text": {
                                "tag": "lark_md",
                                "content": text
                            }
                        }
                    ]
                });
                convert_cli_result(
                    update_card(&self.client, &token, message_id, &card_content).await,
                )?;
            }
            MessageContent::Rich { .. }
            | MessageContent::File { .. }
            | MessageContent::Image { .. }
            | MessageContent::Audio { .. }
            | MessageContent::Media { .. }
            | MessageContent::ShareChat { .. }
            | MessageContent::ShareUser { .. } => {
                return Err(ApiError::NotSupported(
                    "Only text and markdown messages can be edited".to_owned(),
                ));
            }
        }

        // Return message with updated content
        Ok(Message {
            id: message_id.to_owned(),
            session: ChannelSession::new(ChannelPlatform::Feishu, String::new()),
            sender_id: String::new(),
            content: content.clone(),
            timestamp: Utc::now(),
            parent_id: None,
            raw: None,
        })
    }

    async fn delete_message(&self, id: &str) -> ApiResult<()> {
        let message_id = id.trim();
        if message_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Message ID cannot be empty".to_owned(),
            ));
        }

        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        convert_cli_result(messages::delete_message(&self.client, &token, message_id).await)
    }
}

#[async_trait]
impl DocumentsApi for FeishuAdapter {
    /// Creates a new Feishu document.
    ///
    /// If content is provided, it will be appended to the document after creation.
    async fn create_document(
        &self,
        title: &str,
        content: Option<&DocumentContent>,
        parent_id: Option<&str>,
    ) -> ApiResult<Document> {
        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        // Create empty document
        let metadata = convert_cli_result(
            create_feishu_document(&self.client, &token, Some(title), parent_id).await,
        )?;

        let mut document = convert_feishu_document_metadata_to_document(metadata);

        // If content is provided, append it to the document
        if let Some(content) = content {
            let document_id = document.id.clone();
            let initial_content = content.clone();

            self.append_document_content_with_token(&token, &document_id, content)
                .await?;

            document.content = Some(initial_content);
        }

        Ok(document)
    }

    /// Gets a Feishu document by ID.
    ///
    /// Returns the document metadata together with plain text content.
    /// Some fields may be None if not returned by Feishu API.
    async fn get_document(&self, id: &str) -> ApiResult<Option<Document>> {
        let document_id = id.trim();
        if document_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Document ID cannot be empty".to_owned(),
            ));
        }

        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        let metadata = match fetch_document_metadata(&self.client, &token, document_id).await {
            Ok(metadata) => metadata,
            Err(err) => {
                let api_err = convert_string_error_to_api_error(&err);
                match api_err {
                    ApiError::NotFound(_) => return Ok(None),
                    ApiError::Auth(_)
                    | ApiError::RateLimited { .. }
                    | ApiError::InvalidRequest(_)
                    | ApiError::Network(_)
                    | ApiError::Server(_)
                    | ApiError::NotSupported(_)
                    | ApiError::Platform { .. }
                    | ApiError::Other(_) => return Err(api_err),
                }
            }
        };

        let content = match fetch_document_content(&self.client, &token, document_id, None).await {
            Ok(content) => content,
            Err(err) => {
                let api_err = convert_string_error_to_api_error(&err);
                match api_err {
                    ApiError::NotFound(_) => return Ok(None),
                    ApiError::Auth(_)
                    | ApiError::RateLimited { .. }
                    | ApiError::InvalidRequest(_)
                    | ApiError::Network(_)
                    | ApiError::Server(_)
                    | ApiError::NotSupported(_)
                    | ApiError::Platform { .. }
                    | ApiError::Other(_) => return Err(api_err),
                }
            }
        };

        let document = convert_feishu_document_snapshot_to_document(metadata, content);
        Ok(Some(document))
    }

    /// Gets the content of a Feishu document.
    async fn get_document_content(&self, id: &str) -> ApiResult<Option<DocumentContent>> {
        let document_id = id.trim();
        if document_id.is_empty() {
            return Err(ApiError::InvalidRequest(
                "Document ID cannot be empty".to_owned(),
            ));
        }

        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;

        match fetch_document_content(&self.client, &token, document_id, None).await {
            Ok(content) => Ok(Some(DocumentContent::Text(content.content))),
            Err(err) => {
                let api_err = convert_string_error_to_api_error(&err);
                match api_err {
                    ApiError::NotFound(_) => Ok(None),
                    ApiError::Auth(_)
                    | ApiError::RateLimited { .. }
                    | ApiError::InvalidRequest(_)
                    | ApiError::Network(_)
                    | ApiError::Server(_)
                    | ApiError::NotSupported(_)
                    | ApiError::Platform { .. }
                    | ApiError::Other(_) => Err(api_err),
                }
            }
        }
    }

    /// Updates document content.
    ///
    /// Not supported by Feishu; use append_to_document instead.
    async fn update_document(&self, _id: &str, _content: &DocumentContent) -> ApiResult<()> {
        // Feishu doesn't support updating entire document content
        // Recommend using append_to_document instead
        Err(ApiError::NotSupported(
            "update_document not supported by Feishu; use append_to_document instead".to_owned(),
        ))
    }

    /// Appends content blocks to a Feishu document.
    ///
    /// Converts the content to Feishu blocks and inserts them.
    /// Supports Text and Markdown content types.
    async fn append_to_document(&self, id: &str, content: &DocumentContent) -> ApiResult<()> {
        let token = convert_cli_result(self.client.get_tenant_access_token().await)?;
        self.append_document_content_with_token(&token, id, content)
            .await
    }

    /// Lists documents in a container.
    ///
    /// Not supported by Feishu.
    async fn list_documents(
        &self,
        _parent_id: Option<&str>,
        _pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Document>> {
        Err(ApiError::NotSupported(
            "list_documents not supported by Feishu".to_owned(),
        ))
    }

    /// Searches documents.
    ///
    /// Not supported by Feishu.
    async fn search_documents(
        &self,
        _query: &str,
        _pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Document>> {
        Err(ApiError::NotSupported(
            "search_documents not supported by Feishu".to_owned(),
        ))
    }

    /// Deletes a document.
    ///
    /// Not supported by Feishu.
    async fn delete_document(&self, _id: &str) -> ApiResult<()> {
        Err(ApiError::NotSupported(
            "delete_document not supported by Feishu".to_owned(),
        ))
    }

    /// Moves a document to a different parent.
    ///
    /// Not supported by Feishu.
    async fn move_document(&self, _id: &str, _new_parent_id: &str) -> ApiResult<Document> {
        Err(ApiError::NotSupported(
            "move_document not supported by Feishu".to_owned(),
        ))
    }
}

/// Convert FeishuDocumentMetadata to generic Document
///
/// Note: Feishu's create API returns limited metadata (id, title, revision_id, url).
/// owner_id, created_at, updated_at are not available from Feishu API.
fn convert_feishu_document_metadata_to_document(
    metadata: crate::channel::feishu::api::resources::types::FeishuDocumentMetadata,
) -> Document {
    Document {
        id: metadata.document_id,
        title: metadata.title,
        owner_id: None,   // Feishu doesn't return owner
        created_at: None, // Feishu doesn't return create time
        updated_at: None, // Feishu doesn't return update time
        content: None,
        doc_type: DocumentType::Docx,
        metadata: Some(serde_json::json!({
            "revision_id": metadata.revision_id,
            "url": metadata.url,
        })),
    }
}

/// Convert FeishuDocumentContent to generic Document
///
/// Note: Feishu's get and raw_content APIs expose different parts of the snapshot.
fn convert_feishu_document_snapshot_to_document(
    metadata: crate::channel::feishu::api::resources::types::FeishuDocumentMetadata,
    content: crate::channel::feishu::api::resources::types::FeishuDocumentContent,
) -> Document {
    let document_url = metadata.url.or(content.url);
    let document_metadata = serde_json::json!({
        "revision_id": metadata.revision_id,
        "url": document_url,
    });

    Document {
        id: metadata.document_id,
        title: metadata.title,
        owner_id: None,
        created_at: None,
        updated_at: None,
        content: Some(DocumentContent::Text(content.content)),
        doc_type: DocumentType::Docx,
        metadata: Some(document_metadata),
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
        routing::{get, post},
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

    #[test]
    fn feishu_ack_reaction_picker_only_returns_valid_candidates() {
        for _ in 0..128 {
            let emoji = random_feishu_ack_reaction();
            assert!(
                FEISHU_ACK_REACTIONS.contains(&emoji),
                "unexpected Feishu ack reaction: {emoji}"
            );
        }
    }

    #[tokio::test]
    async fn feishu_ack_reaction_helper_posts_expected_request_shape() {
        let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let state = MockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/im/v1/messages/om_inbound_ack_1/reactions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "reaction_id": "reaction_ack_1"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let client = FeishuClient::new(
            base_url,
            "cli_a1b2c3".to_owned(),
            "secret-123".to_owned(),
            30,
        )
        .expect("build feishu client");

        add_message_reaction(&client, "t-token-ack", "om_inbound_ack_1", "THUMBSUP")
            .await
            .expect("add ack reaction");

        let recorded = requests.lock().await.clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].path,
            "/open-apis/im/v1/messages/om_inbound_ack_1/reactions"
        );
        assert_eq!(
            recorded[0].authorization.as_deref(),
            Some("Bearer t-token-ack")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&recorded[0].body)
                .expect("parse reaction body"),
            json!({
                "reaction_type": {
                    "emoji_type": "THUMBSUP"
                }
            })
        );

        server.abort();
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

    #[tokio::test]
    async fn feishu_adapter_create_document_supports_initial_markdown_content() {
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
                                "tenant_access_token": "t-token-doc-create"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "document": {
                                        "document_id": "doxcnCreated",
                                        "revision_id": 7,
                                        "title": "Release Plan"
                                    }
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-heading"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-heading",
                                            "block_type": 3,
                                            "heading1": {
                                                "elements": [
                                                    {
                                                        "text_run": {
                                                            "content": "Release Plan"
                                                        }
                                                    }
                                                ]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
                post({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "blk_real_heading",
                                            "temporary_block_id": "tmp-heading"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");

        let document = DocumentsApi::create_document(
            &adapter,
            "Release Plan",
            Some(&DocumentContent::Markdown("# Release Plan".to_owned())),
            None,
        )
        .await
        .expect("create document");

        assert_eq!(document.id, "doxcnCreated");
        assert_eq!(document.title.as_deref(), Some("Release Plan"));
        assert_eq!(document.doc_type, DocumentType::Docx);
        assert_eq!(
            document.content,
            Some(DocumentContent::Markdown("# Release Plan".to_owned()))
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[1].path, "/open-apis/docx/v1/documents");
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-doc-create")
        );
        assert!(requests[1].body.contains("\"title\":\"Release Plan\""));
        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/blocks/convert"
        );
        assert!(requests[2].body.contains("\"content_type\":\"markdown\""));
        assert!(requests[2].body.contains("# Release Plan"));
        assert_eq!(
            requests[3].path,
            "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant"
        );
        assert!(
            requests[3]
                .query
                .as_deref()
                .is_some_and(|query| query.contains("document_revision_id=-1"))
        );

        server.abort();
    }

    #[tokio::test]
    async fn feishu_adapter_get_document_reads_metadata_and_raw_content() {
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
                                "tenant_access_token": "t-token-doc-get"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnCreated",
                get({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "document": {
                                        "document_id": "doxcnCreated",
                                        "revision_id": 9,
                                        "title": "Release Plan"
                                    }
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnCreated/raw_content",
                get({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 0,
                                "data": {
                                    "content": "hello from docs"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");

        let document = DocumentsApi::get_document(&adapter, "doxcnCreated")
            .await
            .expect("get document should succeed")
            .expect("document should exist");

        assert_eq!(document.id, "doxcnCreated");
        assert_eq!(document.title.as_deref(), Some("Release Plan"));
        assert_eq!(
            document.content,
            Some(DocumentContent::Text("hello from docs".to_owned()))
        );
        assert_eq!(document.doc_type, DocumentType::Docx);
        assert_eq!(
            document
                .metadata
                .as_ref()
                .and_then(|value| value.get("revision_id"))
                .and_then(|value| value.as_i64()),
            Some(9)
        );
        assert_eq!(
            document
                .metadata
                .as_ref()
                .and_then(|value| value.get("url"))
                .and_then(|value| value.as_str()),
            Some("https://open.feishu.cn/docx/doxcnCreated")
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnCreated"
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-doc-get")
        );
        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnCreated/raw_content"
        );
        assert_eq!(
            requests[2].authorization.as_deref(),
            Some("Bearer t-token-doc-get")
        );

        server.abort();
    }

    #[tokio::test]
    async fn feishu_adapter_get_document_returns_none_when_metadata_is_missing() {
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
                                "tenant_access_token": "t-token-doc-missing"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnMissing",
                get({
                    let state = state.clone();
                    move |request| {
                        let state = state.clone();
                        async move {
                            record_request(State(state), request).await;
                            Json(json!({
                                "code": 1770002,
                                "msg": "not found"
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let adapter = FeishuAdapter::new(&resolved_config(&base_url)).expect("build adapter");

        let document = DocumentsApi::get_document(&adapter, "doxcnMissing")
            .await
            .expect("get document should not fail");

        assert_eq!(document, None);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnMissing"
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
