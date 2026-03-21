use crate::channel::feishu::FeishuUserPrincipal;
use crate::channel::{ChannelDeliveryResource, ChannelOutboundTarget, ChannelSession};
use serde_json::Value;

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
