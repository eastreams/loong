use std::time::Duration;

use crate::CliResult;
use crate::channel::core::http::{read_json_or_text_response, validate_outbound_http_target};
use crate::channel::http::ChannelOutboundHttpPolicy;
use bytes::Bytes;
use futures::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::Interval;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing;

use super::message_manager::{QqbotMsgManager, QqbotOutboundMessage};
use super::token_manager::QqbotTokenManager;

const QQBOT_API_BASE_URL: &str = "https://api.sgroup.qq.com";
const QQBOT_GATEWAY_PATH: &str = "/gateway";
const WS_RECONNECT_BASE_DELAY_MS: u64 = 1000;
const WS_RECONNECT_MAX_DELAY_MS: u64 = 30000;

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type WsStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// Response from GET /gateway
#[derive(Debug, Deserialize)]
struct QqbotGatewayResponse {
    url: String,
}

/// Manages QQBot WebSocket connection lifecycle.
pub(super) struct QqbotWebsocketManager {
    resolved: crate::config::ResolvedQqbotChannelConfig,
    token_manager: QqbotTokenManager,
    http_client: Client,
    account_id: String,
    msg_manager: QqbotMsgManager,
    outbound_rx: mpsc::Receiver<QqbotOutboundMessage>,
    wss_tx: Option<WsSink>,
    wss_rx: Option<WsStream>,
    last_seq: u64,
    heartbeat_timer: Option<Interval>,
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
            wss_tx: None,
            wss_rx: None,
            last_seq: 0,
            heartbeat_timer: None,
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
                    tracing::warn!(
                        account_id = %self.account_id,
                        error = %e,
                        "qqbot websocket session error"
                    );
                    tokio::time::sleep(Duration::from_millis(reconnect_delay_ms)).await;
                    reconnect_delay_ms = (reconnect_delay_ms * 2).min(WS_RECONNECT_MAX_DELAY_MS);
                }
            }
        }
    }

    /// Fetch the WebSocket gateway URL from the QQBot API.
    async fn fetch_gateway_url(&self, token: &str) -> CliResult<String> {
        let raw_url = format!("{QQBOT_API_BASE_URL}{QQBOT_GATEWAY_PATH}");
        let url = validate_outbound_http_target("qqbot gateway_url", &raw_url, self.policy)?;

        let response = self
            .http_client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("qqbot gateway request failed: {e}"))?;

        let (_status, _raw, payload) =
            read_json_or_text_response(response, "qqbot gateway").await?;

        let gateway: QqbotGatewayResponse = serde_json::from_value(payload)
            .map_err(|e| format!("qqbot gateway response parse failed: {e}"))?;

        if gateway.url.is_empty() {
            return Err("qqbot gateway returned empty url".to_owned());
        }

        Ok(gateway.url)
    }

    async fn connect_and_run(&mut self) -> CliResult<()> {
        let token = self
            .token_manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("qqbot token unavailable: {e}"))?;

        // Step 1: Fetch WSS gateway URL
        let gateway_url = self.fetch_gateway_url(&token).await?;

        // Step 2: Connect with token appended
        let ws_url = if gateway_url.contains('?') {
            format!("{gateway_url}&access_token={token}")
        } else {
            format!("{gateway_url}?access_token={token}")
        };

        let (stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("qqbot websocket connect failed: {e}"))?;

        let (tx, rx) = stream.split();
        self.wss_tx = Some(tx);
        self.wss_rx = Some(rx);
        tracing::info!(
            account_id = %self.account_id,
            gateway = %gateway_url,
            "qqbot channel connected"
        );

        loop {
            tokio::select! {
                biased;

                // Outbound AI replies
                Some(outbound) = self.outbound_rx.recv() => {
                    let token = match self.token_manager.get_valid_access_token().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(
                                account_id = %self.account_id,
                                error = %e,
                                "qqbot send token unavailable"
                            );
                            continue;
                        }
                    };
                    if let Err(e) = send_qqbot_message(
                        &self.http_client, &outbound.openid, &outbound.text, &token, self.policy,
                    ).await {
                        tracing::warn!(
                            account_id = %self.account_id,
                            error = %e,
                            "qqbot send failed"
                        );
                    }
                }

                // Heartbeat send
                _ = Self::tick_heartbeat(&mut self.heartbeat_timer), if self.heartbeat_timer.is_some() => {
                    self.send_hearbeat().await?;
                }

                // Inbound WebSocket messages
                msg = Self::wss_next(&mut self.wss_rx) => {
                    let msg = msg?;

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
                            let _ = Self::wss_send(&mut self.wss_tx, Message::Pong(Bytes::new())).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Handle a single WebSocket frame.
    fn handle_ws_frame(&mut self, payload: &serde_json::Value) -> CliResult<()> {
        let seq = payload.get("s").and_then(serde_json::Value::as_u64);
        if let Some(s) = seq {
            self.last_seq = s;
        }

        let op = payload.get("op").and_then(serde_json::Value::as_u64);
        match op {
            Some(0) => {
                // Dispatch event — enqueue for message manager
                self.msg_manager.enqueue(payload.clone());
            }
            Some(10) => {
                let heartbeat_intv_ms = payload
                    .get("d")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|obj| {
                        obj.get("heartbeat_interval")
                            .and_then(serde_json::Value::as_u64)
                    })
                    .ok_or_else(|| format!("qqbot has wrong format: {}", payload))?;
                if self.heartbeat_timer.is_none() {
                    self.heartbeat_timer = Some(tokio::time::interval(Duration::from_millis(
                        heartbeat_intv_ms,
                    )));
                }
            }
            Some(11) => {
                // Heartbeat ACK — 确认心跳成功，静默处理
            }
            Some(13) => {
                tracing::warn!(
                    account_id = %self.account_id,
                    error = %payload["d"],
                    "qqbot ws error event"
                );
            }
            _ => {}
        }

        Ok(())
    }

    /// Drain and process all queued messages. Called after each WS frame.
    async fn drain_message_queue(&mut self) {
        self.msg_manager.process_all().await;
    }

    async fn tick_heartbeat(heartbeat_timer: &mut Option<Interval>) {
        if let Some(ref mut intv) = *heartbeat_timer {
            intv.tick().await;
        }
    }

    async fn wss_send(wss_tx: &mut Option<WsSink>, item: Message) -> CliResult<()> {
        match wss_tx {
            Some(tx) => tx
                .send(item)
                .await
                .map_err(|e| format!("qqbot ws send failed: {e}")),
            None => Err("qqbot wss_tx is None".to_owned()),
        }
    }

    async fn wss_next(wss_rx: &mut Option<WsStream>) -> CliResult<Message> {
        match wss_rx {
            Some(rx) => match rx.next().await {
                Some(Ok(msg)) => Ok(msg),
                Some(Err(e)) => Err(format!("qqbot ws read error: {e}")),
                None => Err("qqbot ws closed by remote".to_owned()),
            },
            None => Err("qqbot wss_rx is None".to_owned()),
        }
    }

    async fn send_hearbeat(&mut self) -> CliResult<()> {
        let heartbeat = if self.last_seq > 0 {
            json!({
                "op": 1,
                "d": self.last_seq,
            })
        } else {
            json!({
                "op": 1,
                "d": null,
            })
        };
        if let Err(e) = Self::wss_send(
            &mut self.wss_tx,
            Message::Text(
                serde_json::to_string(&heartbeat)
                    .map_err(|e| format!("qqbot heartbeat serialize failed: {e}"))?
                    .into(),
            ),
        )
        .await
        {
            return Err(format!("qqbot heartbeat send failed: {e}"));
        } else {
            return Ok(());
        }
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
