use std::sync::Arc;
use std::time::Duration;

use super::message_manager::{QqbotMsgManager, QqbotOutboundMessage};
use super::token_manager::QqbotTokenManager;
use crate::CliResult;
use crate::channel::core::http::{read_json_or_text_response, validate_outbound_http_target};
use crate::channel::http::ChannelOutboundHttpPolicy;
use bytes::Bytes;
use futures::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio::time::Interval;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing;

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
    msg_manager: Arc<Mutex<QqbotMsgManager>>,
    outbound_rx: mpsc::Receiver<QqbotOutboundMessage>,
    wss_tx: Option<WsSink>,
    wss_rx: Option<WsStream>,
    last_seq: u64,
    heartbeat_timer: Option<Interval>,
    policy: ChannelOutboundHttpPolicy,
    /// Session ID from ReadyEvent; used for resume on reconnect.
    session_id: Option<String>,
    /// Whether the current connection has completed the handshake.
    handshake_complete: bool,
}

impl QqbotWebsocketManager {
    pub(super) fn new(
        resolved: crate::config::ResolvedQqbotChannelConfig,
        token_manager: QqbotTokenManager,
        http_client: Client,
        account_id: String,
        msg_manager: Arc<Mutex<QqbotMsgManager>>,
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
            session_id: None,
            handshake_complete: false,
        }
    }

    /// Run the full WebSocket session with reconnect logic.
    pub(super) async fn run_session(&mut self) -> CliResult<()> {
        let mut reconnect_delay_ms = WS_RECONNECT_BASE_DELAY_MS;
        loop {
            match self.connect_and_run().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::error!(
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
        // Reset per-connection state
        self.handshake_complete = false;
        self.heartbeat_timer = None;

        let token = self
            .token_manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("qqbot token unavailable: {e}"))?;

        // Step 1: Fetch WSS gateway URL
        let gateway_url = self.fetch_gateway_url(&token).await?;

        let (stream, _) = connect_async(&gateway_url)
            .await
            .map_err(|e| format!("qqbot websocket connect failed: {e}"))?;
        let (tx, rx) = stream.split();
        self.wss_tx = Some(tx);
        self.wss_rx = Some(rx);

        // Perform WebSocket handshake (identify or resume).
        self.perform_handshake(&token).await?;
        loop {
            tokio::select! {
                biased;

                // Outbound AI replies
                Some(outbound) = self.outbound_rx.recv() => {
                    if let Err(e) = self.send_outbound_message(outbound).await {
                        tracing::warn!(
                            account_id = %self.account_id,
                            error = %e,
                            "qqbot outbound message send failed"
                        );
                    }
                }

                // Heartbeat send
                _ = Self::tick_heartbeat(&mut self.heartbeat_timer), if self.handshake_complete && self.heartbeat_timer.is_some() => {
                    self.send_heartbeat().await?;
                }

                // Inbound WebSocket messages
                msg = Self::wss_next(&mut self.wss_rx) => {
                    let msg = msg?;
                    match msg {
                        Message::Text(text) => {
                            let payload: serde_json::Value = serde_json::from_str(&text)
                                .map_err(|e| format!("qqbot ws json parse failed: {e}"))?;
                            self.handle_ws_frame(&payload)?;
                            self.msg_manager.lock().await.enqueue(payload);
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

    /// Perform the WebSocket handshake after connecting.
    /// Sends identify (op=2) on first connect, or resume (op=6) on reconnect.
    async fn perform_handshake(&mut self, token: &str) -> CliResult<()> {
        if self.session_id.is_some() {
            // Resume existing session
            self.send_resume(token).await?;
        } else {
            // First-time identify
            self.send_identify(token).await?;
        }

        // Wait for Ready Event or Invalid Session
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let timeout = tokio::time::sleep_until(deadline);
            tokio::pin!(timeout);

            tokio::select! {
                _ = &mut timeout => {
                    return Err("qqbot handshake timed out waiting for Ready".to_owned());
                }
                msg = Self::wss_next(&mut self.wss_rx) => {
                    let msg = msg?;
                    match msg {
                        Message::Text(text) => {
                            let payload: serde_json::Value = serde_json::from_str(&text)
                                .map_err(|e| format!("qqbot ws json parse failed: {e}"))?;

                            let op = payload.get("op").and_then(serde_json::Value::as_u64);
                            match op {
                                Some(0) => {
                                    // Dispatch event — check for READY
                                    let event_type = payload.get("t").and_then(serde_json::Value::as_str).unwrap_or("");
                                    if event_type == "READY" {
                                        let data = payload.get("d").cloned().unwrap_or(serde_json::Value::Null);
                                        if let Some(sid) = data.get("session_id").and_then(serde_json::Value::as_str) {
                                            self.session_id = Some(sid.to_string());
                                            self.handshake_complete = true;
                                            // Also update last_seq if present
                                            if let Some(s) = payload.get("s").and_then(serde_json::Value::as_u64) {
                                                self.last_seq = s;
                                            }
                                            tracing::info!(
                                                account_id = %self.account_id,
                                                session_id = %sid,
                                                "qqbot handshake complete (Ready)"
                                            );
                                            return Ok(());
                                        }
                                    }
                                    // Not READY yet, enqueue for message manager to process later
                                    self.msg_manager.lock().await.enqueue(payload);
                                }
                                Some(10) => {
                                    // Hello — set heartbeat interval (but don't start until handshake done)
                                    let heartbeat_intv_ms = payload
                                        .get("d")
                                        .and_then(serde_json::Value::as_object)
                                        .and_then(|obj| obj.get("heartbeat_interval").and_then(serde_json::Value::as_u64))
                                        .ok_or_else(|| format!("qqbot hello missing heartbeat_interval: {}", payload))?;
                                    if self.heartbeat_timer.is_none() {
                                        self.heartbeat_timer = Some(tokio::time::interval(Duration::from_millis(heartbeat_intv_ms)));
                                    }
                                }
                                Some(9) => {
                                    // Invalid Session — must re-identify
                                    tracing::warn!(
                                        account_id = %self.account_id,
                                        "qqbot invalid session, clearing session_id and retrying identify"
                                    );
                                    self.session_id = None;
                                    self.send_identify(token).await?;
                                }
                                _ => {
                                    // Ignore other opcodes during handshake
                                }
                            }
                        }
                        Message::Close(_) => return Err("qqbot ws closed during handshake".to_owned()),
                        Message::Ping(_) => {
                            let _ = Self::wss_send(&mut self.wss_tx, Message::Pong(Bytes::new())).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn send_identify(&mut self, token: &str) -> CliResult<()> {
        let identify = json!({
            "op": 2,
            "d": {
                "token": format!("QQBot {}", token),
                "intents": (1u64 << 25) | (1u64 << 30),
                "properties": {
                    "$os": "linux",
                    "$browser": "loongclaw",
                    "$device": "loongclaw"
                }
            }
        });
        Self::wss_send(
            &mut self.wss_tx,
            Message::Text(
                serde_json::to_string(&identify)
                    .map_err(|e| format!("qqbot identify serialize failed: {e}"))?
                    .into(),
            ),
        )
        .await?;
        tracing::info!(account_id = %self.account_id, "qqbot identify sent");
        Ok(())
    }

    async fn send_resume(&mut self, token: &str) -> CliResult<()> {
        let session_id = self
            .session_id
            .as_ref()
            .ok_or("qqbot resume missing session_id")?;
        let resume = json!({
            "op": 6,
            "d": {
                "token": format!("QQBot {}", token),
                "session_id": session_id,
                "seq": self.last_seq
            }
        });
        Self::wss_send(
            &mut self.wss_tx,
            Message::Text(
                serde_json::to_string(&resume)
                    .map_err(|e| format!("qqbot resume serialize failed: {e}"))?
                    .into(),
            ),
        )
        .await?;
        tracing::info!(account_id = %self.account_id, session_id = %session_id, "qqbot resume sent");
        Ok(())
    }

    /// Handle a single WebSocket frame after handshake is complete.
    fn handle_ws_frame(&mut self, payload: &serde_json::Value) -> CliResult<()> {
        let seq = payload.get("s").and_then(serde_json::Value::as_u64);
        if let Some(s) = seq {
            self.last_seq = s;
        }

        let op = payload.get("op").and_then(serde_json::Value::as_u64);
        match op {
            Some(0) => {
                // Dispatch event — enqueue for message manager
                // Note: enqueue is called synchronously; the background processing task
                // will pick it up via process_all().
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
            Some(9) => {
                // Invalid Session — clear session_id so next reconnect will re-identify
                tracing::warn!(
                    account_id = %self.account_id,
                    "qqbot invalid session received, will re-identify on next reconnect"
                );
                self.session_id = None;
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

    async fn send_heartbeat(&mut self) -> CliResult<()> {
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

    /// Send an outbound AI reply via the QQBot REST API.
    async fn send_outbound_message(&mut self, outbound: QqbotOutboundMessage) -> CliResult<()> {
        let token = self
            .token_manager
            .get_valid_access_token()
            .await
            .map_err(|e| format!("qqbot token unavailable for outbound: {e}"))?;
        let msg_id = send_qqbot_message(
            &self.http_client,
            &outbound.openid,
            &outbound.text,
            &token,
            self.policy,
        )
        .await?;
        tracing::info!(
            account_id = %self.account_id,
            openid = %outbound.openid,
            msg_id = %msg_id,
            "qqbot outbound message sent"
        );
        Ok(())
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
        "markdown": {
            "content": text,
        },
        "msg_type": 2,
    });
    let response = http_client
        .post(url)
        .header("Authorization", format!("QQBot {access_token}"))
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
