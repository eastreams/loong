use crate::CliResult;
use crate::channel::feishu::api::resources::messages::{self, FeishuOutboundMessageBody};
use crate::channel::feishu::api::{FeishuClient, FeishuMessageWriteReceipt};
use crate::channel::traits::error::{ApiError, ApiResult};
use crate::channel::traits::messaging::{MessageContent, MessageSendApi};
use crate::channel::{
    ChannelOutboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind, ChannelPlatform,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeishuMessageDispatchRoute<'a> {
    Reply {
        message_id: &'a str,
        reply_in_thread: bool,
    },
    Send {
        receive_id: &'a str,
        receive_id_type: &'a str,
    },
}

fn resolve_feishu_message_dispatch_route<'a>(
    target: &'a ChannelOutboundTarget,
    default_receive_id_type: Option<&'a str>,
) -> ApiResult<FeishuMessageDispatchRoute<'a>> {
    if target.platform != ChannelPlatform::Feishu {
        return Err(ApiError::InvalidRequest(format!(
            "feishu messaging helper cannot route {} targets",
            target.platform.as_str()
        )));
    }

    match target.kind {
        ChannelOutboundTargetKind::MessageReply => Ok(FeishuMessageDispatchRoute::Reply {
            message_id: target.trimmed_id().map_err(ApiError::Other)?,
            reply_in_thread: target.feishu_reply_in_thread().unwrap_or(false),
        }),
        ChannelOutboundTargetKind::ReceiveId => Ok(FeishuMessageDispatchRoute::Send {
            receive_id: target.trimmed_id().map_err(ApiError::Other)?,
            receive_id_type: target
                .feishu_receive_id_type()
                .or(default_receive_id_type)
                .unwrap_or("chat_id"),
        }),
        ChannelOutboundTargetKind::Conversation
        | ChannelOutboundTargetKind::Address
        | ChannelOutboundTargetKind::Endpoint => Err(ApiError::InvalidRequest(format!(
            "feishu messaging helper does not support target kind `{}`",
            target.kind
        ))),
    }
}

fn message_content_from_channel_outbound(message: ChannelOutboundMessage) -> MessageContent {
    match message {
        ChannelOutboundMessage::Text(text) => MessageContent::Text { text },
        ChannelOutboundMessage::MarkdownCard(text) => MessageContent::Markdown { text },
        ChannelOutboundMessage::Post(content) => MessageContent::Rich { content },
        ChannelOutboundMessage::Image { image_key } => MessageContent::Image {
            url: image_key,
            width: None,
            height: None,
        },
        ChannelOutboundMessage::File { file_key } => MessageContent::File {
            name: String::new(),
            url: file_key,
            size: None,
        },
    }
}

pub(crate) async fn deliver_feishu_message_body(
    client: &FeishuClient,
    tenant_access_token: &str,
    default_receive_id_type: &str,
    target: &ChannelOutboundTarget,
    body: &FeishuOutboundMessageBody,
) -> CliResult<FeishuMessageWriteReceipt> {
    match resolve_feishu_message_dispatch_route(target, Some(default_receive_id_type))
        .map_err(|error| error.to_string())?
    {
        FeishuMessageDispatchRoute::Reply {
            message_id,
            reply_in_thread,
        } => {
            messages::reply_outbound_message(
                client,
                tenant_access_token,
                message_id,
                body,
                reply_in_thread,
                target.idempotency_key(),
            )
            .await
        }
        FeishuMessageDispatchRoute::Send {
            receive_id,
            receive_id_type,
        } => {
            messages::send_outbound_message(
                client,
                tenant_access_token,
                receive_id_type,
                receive_id,
                body,
                target.idempotency_key(),
            )
            .await
        }
    }
}

pub(crate) async fn send_channel_message_via_message_send_api<T: MessageSendApi>(
    sender: &T,
    target: &ChannelOutboundTarget,
    message: ChannelOutboundMessage,
) -> ApiResult<()> {
    let content = message_content_from_channel_outbound(message);
    match resolve_feishu_message_dispatch_route(target, None)? {
        FeishuMessageDispatchRoute::Reply { .. } => {
            MessageSendApi::reply(sender, target, &content, None).await?;
        }
        FeishuMessageDispatchRoute::Send { .. } => {
            MessageSendApi::send_message(sender, target, &content, None).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::traits::messaging::{Message, SendOptions};
    use crate::channel::{ChannelPlatform, ChannelSession};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq)]
    enum RecordedCall {
        Send {
            target: ChannelOutboundTarget,
            content: MessageContent,
        },
        Reply {
            target: ChannelOutboundTarget,
            content: MessageContent,
        },
    }

    #[derive(Clone, Default)]
    struct RecordingSender {
        calls: Arc<Mutex<Vec<RecordedCall>>>,
    }

    impl RecordingSender {
        fn recorded_calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().expect("recorded calls lock").clone()
        }
    }

    #[async_trait]
    impl MessageSendApi for RecordingSender {
        async fn send_message(
            &self,
            target: &ChannelOutboundTarget,
            content: &MessageContent,
            _options: Option<SendOptions>,
        ) -> ApiResult<Message> {
            self.calls
                .lock()
                .expect("record send call")
                .push(RecordedCall::Send {
                    target: target.clone(),
                    content: content.clone(),
                });
            Ok(Message {
                id: "om_send".to_owned(),
                session: ChannelSession::new(ChannelPlatform::Feishu, "oc_demo"),
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
            _options: Option<SendOptions>,
        ) -> ApiResult<Message> {
            self.calls
                .lock()
                .expect("record reply call")
                .push(RecordedCall::Reply {
                    target: target.clone(),
                    content: content.clone(),
                });
            Ok(Message {
                id: "om_reply".to_owned(),
                session: ChannelSession::new(ChannelPlatform::Feishu, "oc_demo"),
                sender_id: String::new(),
                content: content.clone(),
                timestamp: Utc::now(),
                parent_id: Some(target.id.clone()),
                raw: None,
            })
        }
    }

    #[tokio::test]
    async fn helper_routes_receive_id_targets_to_send_message() {
        let sender = RecordingSender::default();
        let target = ChannelOutboundTarget::feishu_receive_id("oc_demo")
            .with_feishu_receive_id_type("chat_id");

        send_channel_message_via_message_send_api(
            &sender,
            &target,
            ChannelOutboundMessage::Text("hello send".to_owned()),
        )
        .await
        .expect("receive_id send should succeed");

        assert_eq!(
            sender.recorded_calls(),
            vec![RecordedCall::Send {
                target,
                content: MessageContent::Text {
                    text: "hello send".to_owned(),
                },
            }]
        );
    }

    #[tokio::test]
    async fn helper_routes_message_reply_targets_to_reply() {
        let sender = RecordingSender::default();
        let target = ChannelOutboundTarget::feishu_message_reply("om_parent")
            .with_feishu_reply_in_thread(true);

        send_channel_message_via_message_send_api(
            &sender,
            &target,
            ChannelOutboundMessage::MarkdownCard("reply body".to_owned()),
        )
        .await
        .expect("reply send should succeed");

        assert_eq!(
            sender.recorded_calls(),
            vec![RecordedCall::Reply {
                target,
                content: MessageContent::Markdown {
                    text: "reply body".to_owned(),
                },
            }]
        );
    }

    #[tokio::test]
    async fn helper_preserves_rich_and_media_key_payloads() {
        let sender = RecordingSender::default();

        send_channel_message_via_message_send_api(
            &sender,
            &ChannelOutboundTarget::feishu_receive_id("oc_demo"),
            ChannelOutboundMessage::Post(serde_json::json!({"zh_cn": {"title": "hello"}})),
        )
        .await
        .expect("post send should succeed");
        send_channel_message_via_message_send_api(
            &sender,
            &ChannelOutboundTarget::feishu_receive_id("oc_demo"),
            ChannelOutboundMessage::Image {
                image_key: "img_v2_demo".to_owned(),
            },
        )
        .await
        .expect("image send should succeed");
        send_channel_message_via_message_send_api(
            &sender,
            &ChannelOutboundTarget::feishu_receive_id("oc_demo"),
            ChannelOutboundMessage::File {
                file_key: "file_v2_demo".to_owned(),
            },
        )
        .await
        .expect("file send should succeed");

        let calls = sender.recorded_calls();
        assert_eq!(calls.len(), 3);
        assert!(matches!(
            &calls[0],
            RecordedCall::Send {
                content: MessageContent::Rich { .. },
                ..
            }
        ));
        assert_eq!(
            calls[1],
            RecordedCall::Send {
                target: ChannelOutboundTarget::feishu_receive_id("oc_demo"),
                content: MessageContent::Image {
                    url: "img_v2_demo".to_owned(),
                    width: None,
                    height: None,
                },
            }
        );
        assert_eq!(
            calls[2],
            RecordedCall::Send {
                target: ChannelOutboundTarget::feishu_receive_id("oc_demo"),
                content: MessageContent::File {
                    name: String::new(),
                    url: "file_v2_demo".to_owned(),
                    size: None,
                },
            }
        );
    }

    #[tokio::test]
    async fn helper_rejects_non_feishu_messaging_target_kinds() {
        let sender = RecordingSender::default();
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::Conversation,
            "oc_demo",
        );

        let error = send_channel_message_via_message_send_api(
            &sender,
            &target,
            ChannelOutboundMessage::Text("hello".to_owned()),
        )
        .await
        .expect_err("unsupported target kind should fail");

        match error {
            ApiError::InvalidRequest(message) => {
                assert!(message.contains("target kind `conversation`"));
            }
            ApiError::Auth(_)
            | ApiError::NotFound(_)
            | ApiError::RateLimited { .. }
            | ApiError::Network(_)
            | ApiError::Server(_)
            | ApiError::NotSupported(_)
            | ApiError::Platform { .. }
            | ApiError::Other(_) => panic!("expected invalid request, got {error:?}"),
        }
        assert!(sender.recorded_calls().is_empty());
    }

    #[tokio::test]
    async fn helper_rejects_non_feishu_platform_targets() {
        let sender = RecordingSender::default();
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Telegram,
            ChannelOutboundTargetKind::ReceiveId,
            "oc_demo",
        );

        let error = send_channel_message_via_message_send_api(
            &sender,
            &target,
            ChannelOutboundMessage::Text("hello".to_owned()),
        )
        .await
        .expect_err("non-feishu platform should fail");

        match error {
            ApiError::InvalidRequest(message) => {
                assert!(message.contains("cannot route telegram targets"));
            }
            ApiError::Auth(_)
            | ApiError::NotFound(_)
            | ApiError::RateLimited { .. }
            | ApiError::Network(_)
            | ApiError::Server(_)
            | ApiError::NotSupported(_)
            | ApiError::Platform { .. }
            | ApiError::Other(_) => panic!("expected invalid request, got {error:?}"),
        }
        assert!(sender.recorded_calls().is_empty());
    }
}
