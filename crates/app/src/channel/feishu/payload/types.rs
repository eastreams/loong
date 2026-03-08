#[derive(Debug, Clone)]
pub(in crate::channel::feishu) struct FeishuInboundEvent {
    pub(in crate::channel::feishu) event_id: String,
    pub(in crate::channel::feishu) session_id: String,
    pub(in crate::channel::feishu) message_id: String,
    pub(in crate::channel::feishu) text: String,
}

#[derive(Debug)]
pub(in crate::channel::feishu) enum FeishuWebhookAction {
    UrlVerification { challenge: String },
    Ignore,
    Inbound(FeishuInboundEvent),
}
