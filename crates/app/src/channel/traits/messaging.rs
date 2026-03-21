use serde_json::Value;

#[derive(Clone, Debug)]
pub struct MessageContent {
    pub text: Option<String>,
    pub html: Option<String>,
    pub image_key: Option<String>,
    pub file_key: Option<String>,
    pub file_type: Option<String>,
    pub card: Option<Value>,
}

impl Default for MessageContent {
    fn default() -> Self {
        Self {
            text: None,
            html: None,
            image_key: None,
            file_key: None,
            file_type: None,
            card: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Pagination {
    pub page_size: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaType {
    Image,
    File,
    Audio,
    Video,
}

impl Default for MediaType {
    fn default() -> Self {
        Self::File
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub enum ApiError {
    Network(String),
    Auth(String),
    RateLimited(u64),
    NotFound(String),
    PermissionDenied(String),
    InvalidRequest(String),
    Platform { code: i32, message: String },
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(s) => write!(f, "network error: {s}"),
            Self::Auth(s) => write!(f, "authentication failed: {s}"),
            Self::RateLimited(s) => write!(f, "rate limited, retry after {s}s"),
            Self::NotFound(s) => write!(f, "not found: {s}"),
            Self::PermissionDenied(s) => write!(f, "permission denied: {s}"),
            Self::InvalidRequest(s) => write!(f, "invalid request: {s}"),
            Self::Platform { code, message } => {
                write!(f, "platform error: code={code}, message={message}")
            }
        }
    }
}

impl std::error::Error for ApiError {}

#[async_trait::async_trait]
pub trait MessagingApi: Send + Sync {
    type Receipt: Send + Sync + Clone + std::fmt::Debug;
    type Message: Send + Sync + Clone + std::fmt::Debug;
    type MessagePage: Send + Sync + Clone + std::fmt::Debug;
    type MediaUploadResult: Send + Sync + Clone + std::fmt::Debug;

    async fn send_message(
        &self,
        target: &str,
        receive_id_type: Option<&str>,
        content: &MessageContent,
        idempotency_key: Option<&str>,
    ) -> ApiResult<Self::Receipt>;

    async fn reply(&self, parent_id: &str, content: &MessageContent) -> ApiResult<Self::Receipt>;

    async fn get_message(&self, message_id: &str) -> ApiResult<Self::Message>;

    async fn list_messages(
        &self,
        chat_id: &str,
        pagination: &Pagination,
    ) -> ApiResult<Self::MessagePage>;

    async fn upload_media(
        &self,
        file_path: Option<&str>,
        file_key: Option<&str>,
        media_type: MediaType,
    ) -> ApiResult<Self::MediaUploadResult>;
}
