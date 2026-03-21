use crate::channel::traits::{ApiResult, MediaType, MessageContent, MessagingApi, Pagination};

pub(super) struct TelegramMessagingImpl;

#[async_trait::async_trait]
impl MessagingApi for TelegramMessagingImpl {
    type Receipt = TelegramSendReceipt;
    type Message = TelegramMessage;
    type MessagePage = TelegramMessagePage;
    type MediaUploadResult = TelegramMediaUploadResult;

    async fn send_message(
        &self,
        target: &str,
        receive_id_type: Option<&str>,
        content: &MessageContent,
        idempotency_key: Option<&str>,
    ) -> ApiResult<Self::Receipt> {
        todo!("Implement Telegram send_message")
    }

    async fn reply(&self, parent_id: &str, content: &MessageContent) -> ApiResult<Self::Receipt> {
        todo!("Implement Telegram reply")
    }

    async fn get_message(&self, message_id: &str) -> ApiResult<Self::Message> {
        todo!("Implement Telegram get_message")
    }

    async fn list_messages(
        &self,
        chat_id: &str,
        pagination: &Pagination,
    ) -> ApiResult<Self::MessagePage> {
        todo!("Implement Telegram list_messages")
    }

    async fn upload_media(
        &self,
        file_path: Option<&str>,
        file_key: Option<&str>,
        media_type: MediaType,
    ) -> ApiResult<Self::MediaUploadResult> {
        todo!("Implement Telegram upload_media")
    }
}

#[derive(Clone, Debug)]
pub struct TelegramSendReceipt {
    pub message_id: i64,
}

#[derive(Clone, Debug)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub chat_id: i64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct TelegramMessagePage {
    pub messages: Vec<TelegramMessage>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct TelegramMediaUploadResult {
    pub file_id: String,
}
