use std::collections::VecDeque;
use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::mpsc;
use tracing;

use crate::CliResult;
use crate::KernelContext;
use crate::channel::core::types::{
    ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform, ChannelSession,
};
use crate::channel::process_inbound_with_provider;
use crate::channel::runtime::turn_feedback::ChannelTurnFeedbackPolicy;
use crate::config::LoongClawConfig;
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
    config: LoongClawConfig,
    resolved_path: PathBuf,
    resolved: ResolvedQqbotChannelConfig,
    kernel_ctx: KernelContext,
    account_id: String,
    outbound_tx: mpsc::Sender<QqbotOutboundMessage>,
    queue: VecDeque<Value>,
}

impl QqbotMsgManager {
    pub(super) fn new(
        config: LoongClawConfig,
        resolved_path: PathBuf,
        resolved: ResolvedQqbotChannelConfig,
        kernel_ctx: KernelContext,
        account_id: String,
        outbound_tx: mpsc::Sender<QqbotOutboundMessage>,
    ) -> Self {
        Self {
            config,
            resolved_path,
            resolved,
            kernel_ctx,
            account_id,
            outbound_tx,
            queue: VecDeque::with_capacity(MAX_QUEUE_CAPACITY + 1),
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
        if !matches!(event, QqBotWsEvent::C2cMessage(_)) {
            return Ok(());
        }

        let QqBotWsEvent::C2cMessage(data) = event else {
            return Ok(());
        };

        let inbound = build_qqbot_inbound_message(&data, &self.account_id)?;
        let reply = process_inbound_with_provider(
            &self.config,
            Some(&self.resolved_path),
            &inbound,
            &self.kernel_ctx,
            ChannelTurnFeedbackPolicy::final_trace_significant(),
        )
        .await?;

        let openid = inbound.session.conversation_id.clone();
        if self
            .outbound_tx
            .send(QqbotOutboundMessage {
                openid,
                text: reply,
            })
            .await
            .is_err()
        {
            tracing::warn!(
                account_id = %self.account_id,
                "qqbot outbound channel closed"
            );
        }

        Ok(())
    }
}

/// Parsed QQBot WebSocket event.
enum QqBotWsEvent {
    C2cMessage(Value),
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

fn build_qqbot_inbound_message(data: &Value, account_id: &str) -> CliResult<ChannelInboundMessage> {
    let openid = data["author"]["user_openid"]
        .as_str()
        .ok_or("missing user_openid in qqbot message")?;
    let content = data["content"].as_str().unwrap_or("").to_string();

    let session =
        ChannelSession::with_account(ChannelPlatform::Qqbot, account_id, openid.to_string());

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
            sender_principal_key: None,
            thread_root_id: None,
            parent_message_id: None,
            resources: Vec::new(),
            feishu_callback: None,
        },
    })
}
