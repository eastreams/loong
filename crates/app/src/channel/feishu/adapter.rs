use async_trait::async_trait;
use serde_json::{json, Value};

use crate::channel::{ChannelAdapter, ChannelInboundMessage};
use crate::config::LoongClawConfig;
use crate::CliResult;

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

impl FeishuAdapter {
    pub(super) fn new(config: &LoongClawConfig) -> CliResult<Self> {
        let app_id = config
            .feishu
            .app_id()
            .ok_or_else(|| "missing Feishu app id (feishu.app_id or env)".to_owned())?;
        let app_secret = config
            .feishu
            .app_secret()
            .ok_or_else(|| "missing Feishu app secret (feishu.app_secret or env)".to_owned())?;
        Ok(Self {
            app_id,
            app_secret,
            base_url: config.feishu.base_url.clone(),
            receive_id_type: config.feishu.receive_id_type.clone(),
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

    pub(super) async fn send_card(&self, receive_id: &str, text: &str) -> CliResult<()> {
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

    pub(super) async fn send_reply(&self, message_id: &str, text: &str) -> CliResult<()> {
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
}

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
        Err("feishu inbound is served via webhook mode (`feishu-serve`)".to_owned())
    }

    async fn send_text(&self, target: &str, text: &str) -> CliResult<()> {
        self.send_message(target, "text", json!({"text": text}))
            .await
    }
}
