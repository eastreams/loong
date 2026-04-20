use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::super::{ChannelOutboundTarget, ChannelSession};
use super::error::ApiResult;

const MIN_PAGINATION_LIMIT: usize = 1;
const MAX_PAGINATION_LIMIT: usize = 1000;

/// Content types for messages
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// Plain text message
    Text { text: String },
    /// Markdown formatted message (maps to interactive card)
    Markdown { text: String },
    /// Rich content (post format with structured content)
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
    /// Audio message
    Audio { url: String, duration: Option<u64> },
    /// Media message (video with cover)
    Media {
        url: String,
        cover_url: Option<String>,
        duration: Option<u64>,
    },
    /// Share chat (group card)
    ShareChat { chat_id: String },
    /// Share user (contact card)
    ShareUser { user_id: String },
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
///
/// Uses a unified cursor-based approach. For offset-based pagination,
/// use `with_page()` or `with_offset()` which encode the value in the cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pagination {
    /// Maximum number of items to return
    pub limit: Option<usize>,
    /// Pagination cursor (opaque string)
    ///
    /// For cursor-based platforms: platform-specific token
    /// For offset-based: encoded as "page:N" or "offset:N"
    pub cursor: Option<String>,
}

impl Pagination {
    /// Create pagination with limit only
    pub fn with_limit(limit: usize) -> Self {
        let normalized_limit = limit.clamp(MIN_PAGINATION_LIMIT, MAX_PAGINATION_LIMIT);
        Self {
            limit: Some(normalized_limit),
            cursor: None,
        }
    }

    /// Create pagination with cursor
    pub fn with_cursor(cursor: impl Into<String>) -> Self {
        Self {
            limit: None,
            cursor: Some(cursor.into()),
        }
    }

    /// Create pagination from page number (for offset-based platforms)
    /// Encoded as "page:{n}"
    pub fn with_page(page: u32) -> Self {
        Self {
            limit: None,
            cursor: Some(format!("page:{}", page)),
        }
    }

    /// Create pagination from offset (for offset-based platforms)
    /// Encoded as "offset:{n}"
    pub fn with_offset(offset: usize) -> Self {
        Self {
            limit: None,
            cursor: Some(format!("offset:{}", offset)),
        }
    }

    /// Parse cursor as page number
    pub fn as_page(&self) -> Option<u32> {
        self.cursor
            .as_ref()
            .and_then(|c| c.strip_prefix("page:").and_then(|n| n.parse().ok()))
    }

    /// Parse cursor as offset
    pub fn as_offset(&self) -> Option<usize> {
        self.cursor
            .as_ref()
            .and_then(|c| c.strip_prefix("offset:").and_then(|n| n.parse().ok()))
    }
}

/// Options for sending messages
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendOptions {
    /// Whether to send as a silent message (no notification)
    pub silent: bool,
    /// Whether this message should be formatted as a card/update
    pub as_card: bool,
    /// Whether to reply in thread (for reply operations)
    /// When true, the reply appears in the thread of the parent message
    pub reply_in_thread: bool,
    /// Additional metadata for the platform
    pub metadata: Option<serde_json::Value>,
}

/// Message metadata returned by API operations
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Paginated result for list operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginatedResult<T> {
    /// Items in this page
    pub items: Vec<T>,
    /// Whether there are more items available
    pub has_more: bool,
    /// Cursor for fetching the next page (if has_more is true)
    pub next_cursor: Option<String>,
}

impl<T> PaginatedResult<T> {
    /// Create a new paginated result
    pub fn new(items: Vec<T>, has_more: bool, next_cursor: Option<String>) -> Self {
        Self {
            items,
            has_more,
            next_cursor,
        }
    }

    /// Create a result with no more items
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            has_more: false,
            next_cursor: None,
        }
    }

    /// Create a result with items but no more pages
    pub fn complete(items: Vec<T>) -> Self {
        Self {
            items,
            has_more: false,
            next_cursor: None,
        }
    }
}

/// Trait for message sending capabilities.
#[async_trait]
pub trait MessageSendApi: Send + Sync {
    /// Send a message to a normalized channel target.
    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        content: &MessageContent,
        options: Option<SendOptions>,
    ) -> ApiResult<Message>;

    /// Reply to an existing message.
    async fn reply(
        &self,
        target: &ChannelOutboundTarget,
        content: &MessageContent,
        options: Option<SendOptions>,
    ) -> ApiResult<Message>;
}

/// Trait for message query capabilities.
#[async_trait]
pub trait MessageQueryApi: Send + Sync {
    /// Get a message by ID.
    async fn get_message(&self, id: &str) -> ApiResult<Option<Message>>;

    /// List messages for a normalized channel session.
    async fn list_messages(
        &self,
        session: &ChannelSession,
        pagination: Option<Pagination>,
    ) -> ApiResult<PaginatedResult<Message>>;

    /// Search messages.
    async fn search_messages(
        &self,
        query: &str,
        pagination: Option<Pagination>,
    ) -> ApiResult<PaginatedResult<Message>>;
}

/// Trait for message edit capabilities.
#[async_trait]
pub trait MessageEditApi: Send + Sync {
    /// Edit/update an existing message.
    async fn edit_message(&self, id: &str, content: &MessageContent) -> ApiResult<Message>;
}

/// Trait for message deletion capabilities.
#[async_trait]
pub trait MessageDeleteApi: Send + Sync {
    /// Delete a message.
    async fn delete_message(&self, id: &str) -> ApiResult<()>;
}

/// Trait for messaging capabilities
///
/// Implement this trait for channels that support sending and receiving messages.
/// This is a core capability expected to be implemented by most channel platforms.
///
/// **Note**: This is the preferred abstraction for new code. For legacy protocol-level
/// operations, see `ChannelAdapter`.
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
    ///
    /// Returns a paginated result containing messages and pagination metadata.
    /// Use `next_cursor` from the result to fetch the next page.
    async fn list_messages(
        &self,
        session: &ChannelSession,
        pagination: Option<Pagination>,
    ) -> ApiResult<PaginatedResult<Message>>;

    /// Search messages
    ///
    /// Returns a paginated result containing messages matching the query.
    /// Use `next_cursor` from the result to fetch the next page.
    async fn search_messages(
        &self,
        query: &str,
        pagination: Option<Pagination>,
    ) -> ApiResult<PaginatedResult<Message>>;

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

#[cfg(test)]
mod tests {
    use super::Pagination;

    #[test]
    fn pagination_with_limit_clamps_to_supported_minimum() {
        let pagination = Pagination::with_limit(0);

        assert_eq!(pagination.limit, Some(1));
    }

    #[test]
    fn pagination_with_limit_clamps_to_supported_maximum() {
        let pagination = Pagination::with_limit(1001);

        assert_eq!(pagination.limit, Some(1000));
    }

    fn assert_message_send_api<T: super::MessageSendApi>() {}
    fn assert_message_query_api<T: super::MessageQueryApi>() {}
    fn assert_message_edit_api<T: super::MessageEditApi>() {}
    fn assert_message_delete_api<T: super::MessageDeleteApi>() {}

    #[test]
    fn narrow_trait_assertions_compile() {
        struct TestMessagingApi;

        #[async_trait::async_trait]
        impl super::MessageSendApi for TestMessagingApi {
            async fn send_message(
                &self,
                _target: &crate::channel::ChannelOutboundTarget,
                _content: &super::MessageContent,
                _options: Option<super::SendOptions>,
            ) -> super::ApiResult<super::Message> {
                panic!("compile-time assertion only")
            }

            async fn reply(
                &self,
                _target: &crate::channel::ChannelOutboundTarget,
                _content: &super::MessageContent,
                _options: Option<super::SendOptions>,
            ) -> super::ApiResult<super::Message> {
                panic!("compile-time assertion only")
            }
        }

        #[async_trait::async_trait]
        impl super::MessageQueryApi for TestMessagingApi {
            async fn get_message(&self, _id: &str) -> super::ApiResult<Option<super::Message>> {
                panic!("compile-time assertion only")
            }

            async fn list_messages(
                &self,
                _session: &crate::channel::ChannelSession,
                _pagination: Option<super::Pagination>,
            ) -> super::ApiResult<super::PaginatedResult<super::Message>> {
                panic!("compile-time assertion only")
            }

            async fn search_messages(
                &self,
                _query: &str,
                _pagination: Option<super::Pagination>,
            ) -> super::ApiResult<super::PaginatedResult<super::Message>> {
                panic!("compile-time assertion only")
            }
        }

        #[async_trait::async_trait]
        impl super::MessageEditApi for TestMessagingApi {
            async fn edit_message(
                &self,
                _id: &str,
                _content: &super::MessageContent,
            ) -> super::ApiResult<super::Message> {
                panic!("compile-time assertion only")
            }
        }

        #[async_trait::async_trait]
        impl super::MessageDeleteApi for TestMessagingApi {
            async fn delete_message(&self, _id: &str) -> super::ApiResult<()> {
                panic!("compile-time assertion only")
            }
        }

        assert_message_send_api::<TestMessagingApi>();
        assert_message_query_api::<TestMessagingApi>();
        assert_message_edit_api::<TestMessagingApi>();
        assert_message_delete_api::<TestMessagingApi>();
    }
}
