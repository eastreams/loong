use crate::channel::traits::{ApiResult, MediaType, MessageContent, MessagingApi, Pagination};

pub(super) struct MatrixMessagingImpl;

#[async_trait::async_trait]
impl MessagingApi for MatrixMessagingImpl {
    type Receipt = MatrixSendReceipt;
    type Message = MatrixMessage;
    type MessagePage = MatrixMessagePage;
    type MediaUploadResult = MatrixMediaUploadResult;

    async fn send_message(
        &self,
        target: &str,
        receive_id_type: Option<&str>,
        content: &MessageContent,
        idempotency_key: Option<&str>,
    ) -> ApiResult<Self::Receipt> {
        todo!("Implement Matrix send_message")
    }

    async fn reply(&self, parent_id: &str, content: &MessageContent) -> ApiResult<Self::Receipt> {
        todo!("Implement Matrix reply")
    }

    async fn get_message(&self, message_id: &str) -> ApiResult<Self::Message> {
        todo!("Implement Matrix get_message")
    }

    async fn list_messages(
        &self,
        chat_id: &str,
        pagination: &Pagination,
    ) -> ApiResult<Self::MessagePage> {
        todo!("Implement Matrix list_messages")
    }

    async fn upload_media(
        &self,
        file_path: Option<&str>,
        file_key: Option<&str>,
        media_type: MediaType,
    ) -> ApiResult<Self::MediaUploadResult> {
        todo!("Implement Matrix upload_media")
    }
}

#[derive(Clone, Debug)]
pub struct MatrixSendReceipt {
    pub event_id: String,
}

#[derive(Clone, Debug)]
pub struct MatrixMessage {
    pub event_id: String,
    pub room_id: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct MatrixMessagePage {
    pub messages: Vec<MatrixMessage>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct MatrixMediaUploadResult {
    pub content_uri: String,
}
