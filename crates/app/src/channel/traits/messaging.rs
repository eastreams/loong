use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::super::{ChannelOutboundTarget, ChannelSession};
use super::error::ApiResult;

/// Content types for messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// Plain text message
    Text { text: String },
    /// Markdown formatted message
    Markdown { text: String },
    /// Rich content (cards, attachments, etc.)
    Rich { content: serde_json::Value },
    /// File attachment
    File {
        name: String,
        url: String,
        size: Option<u64>,
    },
    /// Image attachment
    Image {
        url: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

impl MessageContent {
    /// Create a simple text message
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create a markdown message
    pub fn markdown(text: impl Into<String>) -> Self {
        Self::Markdown { text: text.into() }
    }
}

/// Pagination parameters for list operations
#[derive(Debug, Clone, Default)]
pub struct Pagination {
    /// Maximum number of items to return
    pub limit: Option<usize>,
    /// Offset or cursor for pagination
    pub cursor: Option<String>,
    /// Page number (for offset-based pagination)
    pub page: Option<u32>,
}

impl Pagination {
    /// Create pagination with limit
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            ..Default::default()
        }
    }
}

/// Options for sending messages
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    /// Whether to send as a silent message (no notification)
    pub silent: bool,
    /// Whether this message should be formatted as a card/update
    pub as_card: bool,
    /// Additional metadata for the platform
    pub metadata: Option<serde_json::Value>,
}

/// Message metadata returned by API operations
#[derive(Debug, Clone)]
pub struct Message {
    /// Platform-specific message ID
    pub id: String,
    /// Normalized channel session describing where this message belongs
    pub session: ChannelSession,
    /// Sender identifier
    pub sender_id: String,
    /// Message content
    pub content: MessageContent,
    /// Timestamp when the message was sent
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Optional parent message ID (for replies)
    pub parent_id: Option<String>,
    /// Platform-specific raw data
    pub raw: Option<serde_json::Value>,
}

/// Trait for messaging capabilities
///
/// Implement this trait for channels that support sending and receiving messages.
/// This is a core capability expected to be implemented by most channel platforms.
#[async_trait]
pub trait MessagingApi: Send + Sync {
    /// Send a message to a normalized channel target
    ///
    /// # Arguments
    /// * `target` - Normalized delivery target with target kind and routing options
    /// * `content` - Message content
    /// * `options` - Optional send options
    ///
    /// # Returns
    /// The sent message with platform-assigned ID
    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        content: &MessageContent,
        options: Option<SendOptions>,
    ) -> ApiResult<Message>;

    /// Reply to an existing message
    ///
    /// # Arguments
    /// * `target` - Normalized reply target, typically using `message_reply`
    /// * `content` - Reply content
    /// * `options` - Optional send options
    async fn reply(
        &self,
        target: &ChannelOutboundTarget,
        content: &MessageContent,
        options: Option<SendOptions>,
    ) -> ApiResult<Message>;

    /// Get a message by ID
    async fn get_message(&self, id: &str) -> ApiResult<Option<Message>>;

    /// List messages for a normalized channel session
    async fn list_messages(
        &self,
        session: &ChannelSession,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Message>>;

    /// Search messages
    async fn search_messages(
        &self,
        query: &str,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Message>>;

    /// Edit/update an existing message
    async fn edit_message(&self, id: &str, content: &MessageContent) -> ApiResult<Message>;

    /// Delete a message
    async fn delete_message(&self, id: &str) -> ApiResult<()>;
}

/// Extension trait for rich messaging features
#[async_trait]
pub trait RichMessagingApi: MessagingApi {
    /// Send a card/interactive message
    async fn send_card(
        &self,
        target: &ChannelOutboundTarget,
        card: serde_json::Value,
        options: Option<SendOptions>,
    ) -> ApiResult<Message>;

    /// Update an existing card message
    async fn update_card(&self, message_id: &str, card: serde_json::Value) -> ApiResult<Message>;
}
