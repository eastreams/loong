use crate::channel::{ChannelOutboundTarget, ChannelSession};

#[derive(Debug, Clone)]
pub(in crate::channel::feishu) struct FeishuInboundEvent {
    pub(in crate::channel::feishu) event_id: String,
    pub(in crate::channel::feishu) session: ChannelSession,
    pub(in crate::channel::feishu) reply_target: ChannelOutboundTarget,
    pub(in crate::channel::feishu) text: String,
}

#[derive(Debug)]
pub(in crate::channel::feishu) enum FeishuWebhookAction {
    UrlVerification { challenge: String },
    Ignore,
    Inbound(FeishuInboundEvent),
}
