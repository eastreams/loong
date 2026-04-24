use std::collections::VecDeque;
use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::mpsc;
use tracing;

use crate::CliResult;
use crate::KernelContext;
use crate::channel::access_policy::ChannelInboundAccessPolicy;
use crate::channel::core::types::{
    ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform, ChannelSession,
};
use crate::channel::process_inbound_with_provider;
use crate::channel::runtime::turn_feedback::ChannelTurnFeedbackPolicy;
use crate::config::LoongConfig;
use crate::config::ResolvedQqbotChannelConfig;

/// Outbound message to be sent via the WebSocket manager.
#[derive(Debug, Clone)]
pub(super) struct QqbotOutboundMessage {
    pub(super) openid: String,
    pub(super) text: String,
}

/// Maximum messages queued before dropping new ones.
const MAX_QUEUE_CAPACITY: usize = 3;

/// Manages inbound message queue and serial processing.
pub(super) struct QqbotMsgManager {
    config: LoongConfig,
    resolved_path: PathBuf,
    kernel_ctx: KernelContext,
    account_id: String,
    configured_account_id: String,
    access_policy: ChannelInboundAccessPolicy<String>,
    outbound_tx: mpsc::Sender<QqbotOutboundMessage>,
    queue: VecDeque<Value>,
    ready_session_id: Option<String>,
}

impl QqbotMsgManager {
    pub(super) fn new(
        config: LoongConfig,
        resolved_path: PathBuf,
        resolved: ResolvedQqbotChannelConfig,
        kernel_ctx: KernelContext,
        account_id: String,
        outbound_tx: mpsc::Sender<QqbotOutboundMessage>,
    ) -> Self {
        let access_policy = build_qqbot_access_policy(&resolved);
        Self {
            config,
            resolved_path,
            kernel_ctx,
            account_id,
            configured_account_id: resolved.configured_account_id,
            access_policy,
            outbound_tx,
            queue: VecDeque::with_capacity(MAX_QUEUE_CAPACITY + 1),
            ready_session_id: None,
        }
    }

    /// Enqueue a raw WebSocket payload. Drops if at capacity.
    pub(super) fn enqueue(&mut self, payload: Value) {
        if self.queue.len() >= MAX_QUEUE_CAPACITY {
            tracing::warn!(
                account_id = %self.account_id,
                "qqbot message queue full, dropping message"
            );
            return;
        }
        self.queue.push_back(payload);
    }

    /// Process all queued messages serially.
    pub(super) async fn process_all(&mut self) {
        while let Some(payload) = self.queue.pop_front() {
            if let Err(e) = self.process_single(&payload).await {
                tracing::warn!(
                    account_id = %self.account_id,
                    error = %e,
                    "qqbot message processing failed"
                );
            }
        }
    }

    async fn process_single(&mut self, payload: &Value) -> CliResult<()> {
        let event = parse_qqbot_ws_frame(payload)?;
        match event {
            QqBotWsEvent::C2cMessage(data) => self.process_c2c_message(data).await,
            QqBotWsEvent::ReadyEvent(session_id) => {
                self.process_ready_event(session_id);
                Ok(())
            }
            QqBotWsEvent::HeartbeatInterval(_) | QqBotWsEvent::Error(_) | QqBotWsEvent::Other => {
                Ok(())
            }
        }
    }

    /// Process a C2C (user-to-bot) message: build inbound, get AI reply, send outbound.
    async fn process_c2c_message(&mut self, data: Value) -> CliResult<()> {
        let inbound =
            build_qqbot_inbound_message(&data, &self.account_id, &self.configured_account_id)?;
        let peer_id = inbound.session.conversation_id.as_str();
        if !qqbot_peer_allowed(&self.access_policy, peer_id) {
            tracing::info!(
                account_id = %self.account_id,
                peer_id,
                "qqbot inbound message ignored by allowed_peer_ids"
            );
            return Ok(());
        }
        let reply = process_inbound_with_provider(
            &self.config,
            Some(&self.resolved_path),
            &inbound,
            &self.kernel_ctx,
            ChannelTurnFeedbackPolicy::final_trace_significant(),
        )
        .await?;

        let openid = inbound.session.conversation_id.clone();
        self.send_outbound(openid, reply).await;

        Ok(())
    }

    /// Store the session ID from a Ready event.
    fn process_ready_event(&mut self, session_id: String) {
        self.ready_session_id = Some(session_id);
        tracing::info!(
            account_id = %self.account_id,
            "qqbot ready event processed"
        );
    }

    /// Send an outbound message to the WebSocket manager, logging on channel closure.
    async fn send_outbound(&self, openid: String, text: String) {
        if self
            .outbound_tx
            .send(QqbotOutboundMessage { openid, text })
            .await
            .is_err()
        {
            tracing::warn!(
                account_id = %self.account_id,
                "qqbot outbound channel closed"
            );
        }
    }
}

fn build_qqbot_access_policy(
    resolved: &ResolvedQqbotChannelConfig,
) -> ChannelInboundAccessPolicy<String> {
    ChannelInboundAccessPolicy::from_string_lists(resolved.allowed_peer_ids.as_slice(), &[], false)
}

fn qqbot_peer_allowed(access_policy: &ChannelInboundAccessPolicy<String>, peer_id: &str) -> bool {
    access_policy.allows_str(peer_id, None)
}

/// Parsed QQBot WebSocket event.
enum QqBotWsEvent {
    C2cMessage(Value),
    ReadyEvent(String),
    #[allow(dead_code)]
    HeartbeatInterval(u64),
    #[allow(dead_code)]
    Error(Value),
    Other,
}

fn parse_qqbot_ws_frame(raw: &Value) -> CliResult<QqBotWsEvent> {
    let op = raw
        .get("op")
        .and_then(Value::as_u64)
        .ok_or("qqbot ws frame missing op field")?;

    match op {
        0 => {
            let event_type = raw.get("t").and_then(Value::as_str).unwrap_or("");
            let data = raw.get("d").cloned().unwrap_or(Value::Null);
            match event_type {
                "C2C_MESSAGE_CREATE" => Ok(QqBotWsEvent::C2cMessage(data)),
                "READY" => {
                    let session_id = data
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string())
                        .ok_or("qqbot ready event missing session_id")?;
                    Ok(QqBotWsEvent::ReadyEvent(session_id))
                }
                _ => Ok(QqBotWsEvent::Other),
            }
        }
        10 => {
            let interval = raw
                .get("d")
                .and_then(|d| d.get("heartbeat_interval"))
                .and_then(Value::as_u64)
                .unwrap_or(30_000);
            Ok(QqBotWsEvent::HeartbeatInterval(interval))
        }
        13 => Ok(QqBotWsEvent::Error(
            raw.get("d").cloned().unwrap_or(Value::Null),
        )),
        _ => Ok(QqBotWsEvent::Other),
    }
}

fn build_qqbot_inbound_message(
    data: &Value,
    account_id: &str,
    configured_account_id: &str,
) -> CliResult<ChannelInboundMessage> {
    let openid = data
        .get("author")
        .ok_or("missing author in qqbot message")?
        .get("user_openid")
        .ok_or("missing user_openid in qqbot message")?
        .as_str()
        .ok_or("")?;
    let content = data
        .get("content")
        .ok_or("missing content in qqbot message")?
        .as_str()
        .ok_or("")?
        .to_string();

    let session =
        ChannelSession::with_account(ChannelPlatform::Qqbot, account_id, openid.to_string())
            .with_configured_account_id(configured_account_id);

    let reply_target = ChannelOutboundTarget::new(
        ChannelPlatform::Qqbot,
        ChannelOutboundTargetKind::Conversation,
        openid.to_string(),
    );

    Ok(ChannelInboundMessage {
        session,
        reply_target,
        text: content,
        delivery: ChannelDelivery {
            ack_cursor: None,
            source_message_id: data["id"].as_str().map(|s| s.to_string()),
            sender_principal_key: Some(format!("qqbot:user:{openid}")),
            thread_root_id: None,
            parent_message_id: None,
            resources: Vec::new(),
            feishu_callback: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ChannelAccountIdentity, ChannelAccountIdentitySource, ResolvedQqbotChannelConfig,
    };
    use serde_json::json;

    fn resolved_with_allowed_peers(allowed_peer_ids: Vec<String>) -> ResolvedQqbotChannelConfig {
        ResolvedQqbotChannelConfig {
            configured_account_id: "primary".to_owned(),
            configured_account_label: "primary".to_owned(),
            account: ChannelAccountIdentity {
                id: "qqbot_account".to_owned(),
                label: "qqbot account".to_owned(),
                source: ChannelAccountIdentitySource::Configured,
            },
            enabled: true,
            app_id: None,
            app_id_env: None,
            client_secret: None,
            client_secret_env: None,
            allowed_peer_ids,
        }
    }

    #[test]
    fn qqbot_access_policy_requires_allowed_peer() {
        let resolved = resolved_with_allowed_peers(vec!["peer_allowed".to_owned()]);
        let access_policy = build_qqbot_access_policy(&resolved);

        assert!(qqbot_peer_allowed(&access_policy, "peer_allowed"));
        assert!(!qqbot_peer_allowed(&access_policy, "peer_other"));
        assert!(!qqbot_peer_allowed(&access_policy, ""));
    }

    #[test]
    fn qqbot_inbound_records_configured_account_and_sender() {
        let payload = json!({
            "id": "msg-1",
            "author": {"user_openid": "peer_allowed"},
            "content": "hello"
        });

        let inbound = build_qqbot_inbound_message(&payload, "runtime_account", "primary")
            .expect("valid qqbot inbound message");

        assert_eq!(
            inbound.session.account_id.as_deref(),
            Some("runtime_account")
        );
        assert_eq!(
            inbound.session.configured_account_id.as_deref(),
            Some("primary")
        );
        assert_eq!(inbound.session.conversation_id, "peer_allowed");
        assert_eq!(
            inbound.delivery.sender_principal_key.as_deref(),
            Some("qqbot:user:peer_allowed")
        );
    }
}
