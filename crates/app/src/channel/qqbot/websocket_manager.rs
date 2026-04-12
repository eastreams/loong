use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::CliResult;
use crate::channel::core::http::{read_json_or_text_response, validate_outbound_http_target};
use crate::channel::http::ChannelOutboundHttpPolicy;

use super::message_manager::{QqbotMsgManager, QqbotOutboundMessage};
use super::token_manager::QqbotTokenManager;

const QQBOT_API_BASE_URL: &str = "https://api.sgroup.qq.com";
const QQBOT_WS_URL: &str = "wss://api.sgroup.qq.com/websocket";
const WS_RECONNECT_BASE_DELAY_MS: u64 = 1000;
const WS_RECONNECT_MAX_DELAY_MS: u64 = 30000;

/// Manages QQBot WebSocket connection lifecycle.
pub(super) struct QqbotWebsocketManager {
    resolved: crate::config::ResolvedQqbotChannelConfig,
    token_manager: QqbotTokenManager,
    http_client: Client,
    account_id: String,
    msg_manager: QqbotMsgManager,
    outbound_rx: mpsc::Receiver<QqbotOutboundMessage>,
    last_seq: u64,
    policy: ChannelOutboundHttpPolicy,
}

impl QqbotWebsocketManager {
    pub(super) fn new(
        resolved: crate::config::ResolvedQqbotChannelConfig,
        token_manager: QqbotTokenManager,
        http_client: Client,
        account_id: String,
        msg_manager: QqbotMsgManager,
        outbound_rx: mpsc::Receiver<QqbotOutboundMessage>,
        policy: ChannelOutboundHttpPolicy,
    ) -> Self {
        Self {
            resolved,
            token_manager,
            http_client,
            account_id,
            msg_manager,
            outbound_rx,
            last_seq: 0,
            policy,
        }
    }

    /// Run the full WebSocket session with reconnect logic.
    pub(super) async fn run_session(&mut self) -> CliResult<()> {
        let mut reconnect_delay_ms = WS_RECONNECT_BASE_DELAY_MS;

        loop {
            match self.connect_and_run().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    eprintln!(
                        "warning: qqbot websocket session error for account {}: {e}",
                        self.account_id
                    );
                    tokio::time::sleep(Duration::from_millis(reconnect_delay_ms)).await;
                    reconnect_delay_ms = (reconnect_delay_ms * 2).min(WS_RECONNECT_MAX_DELAY_MS);
                }
            }
        }
    }

    async fn connect_and_run(&mut self) -> CliResult<()> {
        let token = self
            .token_manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("qqbot token unavailable: {e}"))?;

        let ws_url = format!("{QQBOT_WS_URL}/?access_token={token}");
        let (stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("qqbot websocket connect failed: {e}"))?;

        let (mut tx, mut rx) = stream.split();

        eprintln!("qqbot channel connected for account {}", self.account_id);

        loop {
            tokio::select! {
                biased;

                Some(outbound) = self.outbound_rx.recv() => {
                    let token = match self.token_manager.get_valid_access_token().await {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("warning: qqbot send token unavailable: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = send_qqbot_message(
                        &self.http_client, &outbound.openid, &outbound.text, &token, self.policy,
                    ).await {
                        eprintln!(
                            "warning: qqbot send failed for account {}: {e}",
                            self.account_id
                        );
                    }
                }

                maybe_msg = rx.next() => {
                    let msg = match maybe_msg {
                        Some(Ok(msg)) => msg,
                        Some(Err(e)) => return Err(format!("qqbot ws read error: {e}")),
                        None => return Err("qqbot ws closed by remote".to_owned()),
                    };

                    match msg {
                        Message::Text(text) => {
                            let payload: serde_json::Value = serde_json::from_str(&text)
                                .map_err(|e| format!("qqbot ws json parse failed: {e}"))?;
                            self.handle_ws_frame(&payload)?;
                            // Process queued messages (AI invocation) after each frame
                            self.drain_message_queue().await;
                        }
                        Message::Close(_) => return Err("qqbot ws closed by remote".to_owned()),
                        Message::Ping(_) => {
                            let _ = tx.send(Message::Pong(Bytes::new())).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn handle_ws_frame(&mut self, payload: &serde_json::Value) -> CliResult<()> {
        let seq = payload.get("s").and_then(serde_json::Value::as_u64);
        if let Some(s) = seq {
            self.last_seq = s;
        }

        let op = payload.get("op").and_then(serde_json::Value::as_u64);
        match op {
            Some(0) => {
                self.msg_manager.enqueue(payload.clone());
            }
            Some(10) => {
                // Server heartbeat interval
            }
            Some(13) => {
                eprintln!("warning: qqbot ws error event: {}", payload["d"]);
            }
            _ => {}
        }

        Ok(())
    }

    /// Drain and process all queued messages. Called after each WS frame.
    async fn drain_message_queue(&mut self) {
        self.msg_manager.process_all().await;
    }
}

/// Send a message to a QQBot user via the REST API.
pub(super) async fn send_qqbot_message(
    http_client: &Client,
    openid: &str,
    text: &str,
    access_token: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<String> {
    let raw_url = format!("{QQBOT_API_BASE_URL}/v2/users/{openid}/messages");
    let url = validate_outbound_http_target("qqbot send_url", &raw_url, policy)?;
    let body = json!({
        "content": text,
        "msg_type": 0,
    });

    let response = http_client
        .post(url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("qqbot send request failed: {e}"))?;

    let (status, _raw, payload) = read_json_or_text_response(response, "qqbot send").await?;

    if !status.is_success() {
        return Err(format!(
            "qqbot send returned status {}: {payload}",
            status.as_u16()
        ));
    }

    let msg_id = payload
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("qqbot send did not return message id: {payload}"))?
        .to_string();

    Ok(msg_id)
}
