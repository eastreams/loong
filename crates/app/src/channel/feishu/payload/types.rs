use crate::channel::feishu::api::FeishuUserPrincipal;
use crate::channel::{ChannelDeliveryResource, ChannelOutboundTarget, ChannelSession};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub(in crate::channel::feishu) struct FeishuInboundEvent {
    pub(in crate::channel::feishu) event_id: String,
    pub(in crate::channel::feishu) message_id: String,
    pub(in crate::channel::feishu) root_id: Option<String>,
    pub(in crate::channel::feishu) parent_id: Option<String>,
    pub(in crate::channel::feishu) session: ChannelSession,
    pub(in crate::channel::feishu) principal: Option<FeishuUserPrincipal>,
    pub(in crate::channel::feishu) reply_target: ChannelOutboundTarget,
    pub(in crate::channel::feishu) text: String,
    pub(in crate::channel::feishu) resources: Vec<ChannelDeliveryResource>,
}

impl FeishuInboundEvent {
    pub(in crate::channel::feishu) fn delivery_dedupe_key(&self) -> &str {
        self.message_id.as_str()
    }
}

#[derive(Debug)]
pub(in crate::channel::feishu) enum FeishuWebhookAction {
    UrlVerification { challenge: String },
    Ignore,
    Inbound(FeishuInboundEvent),
    CardCallback(FeishuCardCallbackEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::channel::feishu) enum FeishuCardCallbackVersion {
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::channel::feishu) struct FeishuCardCallbackAction {
    pub(in crate::channel::feishu) tag: String,
    pub(in crate::channel::feishu) name: Option<String>,
    pub(in crate::channel::feishu) value: Option<Value>,
    pub(in crate::channel::feishu) form_value: Option<Value>,
    pub(in crate::channel::feishu) timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::channel::feishu) struct FeishuCardCallbackContext {
    pub(in crate::channel::feishu) open_message_id: Option<String>,
    pub(in crate::channel::feishu) open_chat_id: Option<String>,
    pub(in crate::channel::feishu) url: Option<String>,
    pub(in crate::channel::feishu) preview_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::channel::feishu) struct FeishuCardCallbackEvent {
    pub(in crate::channel::feishu) event_id: String,
    pub(in crate::channel::feishu) version: FeishuCardCallbackVersion,
    pub(in crate::channel::feishu) session: ChannelSession,
    pub(in crate::channel::feishu) principal: Option<FeishuUserPrincipal>,
    pub(in crate::channel::feishu) callback_token: Option<String>,
    pub(in crate::channel::feishu) action: FeishuCardCallbackAction,
    pub(in crate::channel::feishu) context: FeishuCardCallbackContext,
    pub(in crate::channel::feishu) text: String,
}

impl FeishuCardCallbackEvent {
    pub(in crate::channel::feishu) fn delivery_dedupe_key(&self) -> &str {
        self.event_id.as_str()
    }
}

pub(in crate::channel::feishu) fn feishu_message_reply_idempotency_key(
    account_id: &str,
    message_id: &str,
) -> String {
    feishu_stable_idempotency_key("reply", [Some(account_id), Some(message_id)])
}

fn feishu_stable_idempotency_key<'a>(
    namespace: &str,
    parts: impl IntoIterator<Item = Option<&'a str>>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"loong:feishu:");
    hasher.update(namespace.as_bytes());

    for part in parts {
        hasher.update([0x1f]);
        if let Some(part) = part {
            hasher.update(part.trim().as_bytes());
        }
    }

    let digest = hex::encode(hasher.finalize());
    format!("feishu-{namespace}-{}", &digest[..24])
}
