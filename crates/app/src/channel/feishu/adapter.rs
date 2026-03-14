use async_trait::async_trait;
use serde_json::{Value, json};

use crate::CliResult;
use crate::channel::{
    ChannelAdapter, ChannelInboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform,
};
use crate::config::ResolvedFeishuChannelConfig;

use super::payload::{
    build_feishu_reply_payload, build_feishu_send_payload, ensure_feishu_response_ok,
};

pub(super) struct FeishuAdapter {
    app_id: String,
    app_secret: String,
    base_url: String,
    receive_id_type: String,
    tenant_access_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeishuSendTarget<'a> {
    MessageReply(&'a str),
    ReceiveId(&'a str),
}

fn resolve_feishu_send_target<'a>(
    target: &'a ChannelOutboundTarget,
) -> CliResult<FeishuSendTarget<'a>> {
    if target.platform != ChannelPlatform::Feishu {
        return Err(format!(
            "feishu adapter cannot send to {} target",
            target.platform.as_str()
        ));
    }

    match target.kind {
        ChannelOutboundTargetKind::MessageReply => {
            Ok(FeishuSendTarget::MessageReply(target.trimmed_id()?))
        }
        ChannelOutboundTargetKind::ReceiveId => {
            Ok(FeishuSendTarget::ReceiveId(target.trimmed_id()?))
        }
        ChannelOutboundTargetKind::Conversation => Err(
            "feishu adapter does not support conversation targets for outbound sends".to_owned(),
        ),
    }
}

impl FeishuAdapter {
    pub(super) fn new(config: &ResolvedFeishuChannelConfig) -> CliResult<Self> {
        let app_id = config
            .app_id()
            .ok_or_else(|| "missing Feishu app id (feishu.app_id or env)".to_owned())?;
        let app_secret = config
            .app_secret()
            .ok_or_else(|| "missing Feishu app secret (feishu.app_secret or env)".to_owned())?;
        Ok(Self {
            app_id,
            app_secret,
            base_url: config.resolved_base_url(),
            receive_id_type: config.receive_id_type.clone(),
            tenant_access_token: None,
        })
    }

    pub(super) async fn refresh_tenant_token(&mut self) -> CliResult<()> {
        let url = format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            self.base_url.trim_end_matches('/')
        );
        let client = reqwest::Client::new();
        let payload = client
            .post(url)
            .json(&json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await
            .map_err(|error| format!("feishu auth request failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("feishu auth decode failed: {error}"))?;

        let code = payload.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code != 0 {
            return Err(format!("feishu auth returned code {code}: {payload}"));
        }

        let token = payload
            .get("tenant_access_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("feishu auth missing tenant_access_token: {payload}"))?;
        self.tenant_access_token = Some(token.to_owned());
        Ok(())
    }

    pub(super) async fn send_card(
        &self,
        target: &ChannelOutboundTarget,
        text: &str,
    ) -> CliResult<()> {
        let receive_id = self.resolve_receive_id_target(target)?;
        let card = json!({
            "config": {
                "wide_screen_mode": true
            },
            "elements": [
                {
                    "tag": "markdown",
                    "content": text
                }
            ]
        });
        self.send_message(receive_id, "interactive", json!({ "card": card }))
            .await
    }

    async fn send_reply(&self, message_id: &str, text: &str) -> CliResult<()> {
        let token = self.tenant_access_token.as_deref().ok_or_else(|| {
            "feishu tenant token is missing, call refresh_tenant_token first".to_owned()
        })?;
        let message_id = message_id.trim();
        if message_id.is_empty() {
            return Err("feishu reply message id is empty".to_owned());
        }

        let url = format!(
            "{}/open-apis/im/v1/messages/{}/reply",
            self.base_url.trim_end_matches('/'),
            message_id
        );
        let payload = build_feishu_reply_payload("text", json!({"text": text}))?;
        let client = reqwest::Client::new();
        let response = client
            .post(url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .await
            .map_err(|error| format!("feishu reply request failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("feishu reply decode failed: {error}"))?;
        ensure_feishu_response_ok("feishu reply", &response)
    }

    async fn send_message(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: Value,
    ) -> CliResult<()> {
        let token = self.tenant_access_token.as_deref().ok_or_else(|| {
            "feishu tenant token is missing, call refresh_tenant_token first".to_owned()
        })?;
        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type={}",
            self.base_url.trim_end_matches('/'),
            self.receive_id_type
        );

        let body = build_feishu_send_payload(receive_id, msg_type, content)?;
        let client = reqwest::Client::new();
        let payload = client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("feishu send message failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("feishu send decode failed: {error}"))?;
        ensure_feishu_response_ok("feishu send", &payload)
    }

    fn resolve_receive_id_target<'a>(
        &self,
        target: &'a ChannelOutboundTarget,
    ) -> CliResult<&'a str> {
        match resolve_feishu_send_target(target)? {
            FeishuSendTarget::ReceiveId(receive_id) => Ok(receive_id),
            FeishuSendTarget::MessageReply(_) => {
                Err("feishu card send requires receive_id target, got message_reply".to_owned())
            }
        }
    }
}

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
        Err("feishu inbound is served via webhook mode (`feishu-serve`)".to_owned())
    }

    async fn send_text(&self, target: &ChannelOutboundTarget, text: &str) -> CliResult<()> {
        match resolve_feishu_send_target(target)? {
            FeishuSendTarget::MessageReply(message_id) => self.send_reply(message_id, text).await,
            FeishuSendTarget::ReceiveId(receive_id) => {
                self.send_message(receive_id, "text", json!({"text": text}))
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_feishu_send_target_supports_reply_and_receive_id_targets() {
        let reply = ChannelOutboundTarget::feishu_message_reply(" om_1 ");
        let receive_id = ChannelOutboundTarget::feishu_receive_id(" ou_1 ");

        assert_eq!(
            resolve_feishu_send_target(&reply).expect("message reply target"),
            FeishuSendTarget::MessageReply("om_1")
        );
        assert_eq!(
            resolve_feishu_send_target(&receive_id).expect("receive id target"),
            FeishuSendTarget::ReceiveId("ou_1")
        );
    }

    #[test]
    fn resolve_feishu_send_target_rejects_conversation_targets() {
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::Conversation,
            "oc_1",
        );

        assert_eq!(
            resolve_feishu_send_target(&target)
                .expect_err("conversation target should be rejected"),
            "feishu adapter does not support conversation targets for outbound sends"
        );
    }

    #[test]
    fn resolve_receive_id_target_rejects_reply_targets_for_cards() {
        let adapter = FeishuAdapter {
            app_id: "cli_a".to_owned(),
            app_secret: "secret".to_owned(),
            base_url: "https://open.feishu.cn".to_owned(),
            receive_id_type: "open_id".to_owned(),
            tenant_access_token: None,
        };
        let target = ChannelOutboundTarget::feishu_message_reply("om_1");

        assert_eq!(
            adapter
                .resolve_receive_id_target(&target)
                .expect_err("reply target should be rejected for card send"),
            "feishu card send requires receive_id target, got message_reply"
        );
    }
}
