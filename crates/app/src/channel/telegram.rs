use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::CliResult;
use crate::config::{self, ResolvedTelegramChannelConfig};

use super::{
    ChannelAdapter, ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget,
    ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
};

pub(super) struct TelegramAdapter {
    account_id: String,
    token: String,
    base_url: String,
    timeout_s: u64,
    offset_tracker: TelegramOffsetTracker,
    allowlist: BTreeSet<i64>,
}

struct TelegramOffsetTracker {
    offset_path: PathBuf,
    current_offset: i64,
    pending_batch_offset: Option<i64>,
}

impl TelegramOffsetTracker {
    fn new(offset_path: PathBuf, current_offset: i64) -> Self {
        Self {
            offset_path,
            current_offset,
            pending_batch_offset: None,
        }
    }

    fn current_offset(&self) -> i64 {
        self.current_offset
    }

    fn remember_polled_offset(&mut self, next_offset: Option<i64>) -> CliResult<()> {
        self.pending_batch_offset = match next_offset {
            Some(next) if next > self.current_offset => Some(next),
            _ => None,
        };
        Ok(())
    }

    fn ack_delivery(&mut self, ack_cursor: Option<&str>) -> CliResult<()> {
        let Some(raw_cursor) = ack_cursor.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(());
        };
        let cursor = raw_cursor
            .parse::<i64>()
            .map_err(|error| format!("invalid telegram ack cursor `{raw_cursor}`: {error}"))?;
        self.persist_if_newer(cursor)
    }

    fn complete_batch(&mut self) -> CliResult<()> {
        if let Some(next_offset) = self.pending_batch_offset.take() {
            self.persist_if_newer(next_offset)?;
        }
        Ok(())
    }

    fn persist_if_newer(&mut self, next_offset: i64) -> CliResult<()> {
        if next_offset <= self.current_offset {
            return Ok(());
        }
        save_offset(&self.offset_path, next_offset)?;
        self.current_offset = next_offset;
        Ok(())
    }
}

impl TelegramAdapter {
    pub(super) fn new(config: &ResolvedTelegramChannelConfig, token: String) -> Self {
        let offset_home = config::default_loongclaw_home();
        let offset_path =
            telegram_offset_path_for_account(offset_home.as_path(), config.account.id.as_str());
        let next_offset =
            load_offset_for_account(offset_home.as_path(), config.account.id.as_str()).unwrap_or(0);
        Self {
            account_id: config.account.id.clone(),
            token,
            base_url: config.base_url.clone(),
            timeout_s: config.polling_timeout_s.clamp(1, 50),
            offset_tracker: TelegramOffsetTracker::new(offset_path, next_offset),
            allowlist: config.allowed_chat_ids.iter().copied().collect(),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.base_url.trim_end_matches('/'),
            self.token,
            method
        )
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
        let url = self.api_url("getUpdates");
        let client = reqwest::Client::new();
        let body = json!({
            "offset": self.offset_tracker.current_offset(),
            "timeout": self.timeout_s,
            "allowed_updates": ["message"],
        });
        let payload = client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("telegram getUpdates failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("telegram getUpdates decode failed: {error}"))?;

        let (inbox, next_offset) = parse_telegram_updates(
            &payload,
            &self.allowlist,
            self.offset_tracker.current_offset(),
            self.account_id.as_str(),
        )?;
        self.offset_tracker.remember_polled_offset(next_offset)?;

        Ok(inbox)
    }

    async fn send_text(&self, target: &ChannelOutboundTarget, text: &str) -> CliResult<()> {
        if target.platform != ChannelPlatform::Telegram {
            return Err(format!(
                "telegram adapter cannot send to {} target",
                target.platform.as_str()
            ));
        }
        if target.kind != ChannelOutboundTargetKind::Conversation {
            return Err(format!(
                "telegram adapter requires conversation target, got {}",
                target.kind.as_str()
            ));
        }

        let chat_id = target
            .trimmed_id()?
            .parse::<i64>()
            .map_err(|error| format!("invalid telegram chat id `{}`: {error}", target.id))?;

        let url = self.api_url("sendMessage");
        let client = reqwest::Client::new();
        let body = json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true,
        });

        let payload = client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("telegram sendMessage failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("telegram sendMessage decode failed: {error}"))?;

        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(format!("telegram sendMessage not ok: {payload}"));
        }
        Ok(())
    }

    async fn ack_inbound(&mut self, message: &ChannelInboundMessage) -> CliResult<()> {
        self.offset_tracker
            .ack_delivery(message.delivery.ack_cursor.as_deref())
    }

    async fn complete_batch(&mut self) -> CliResult<()> {
        self.offset_tracker.complete_batch()
    }
}

pub(super) fn parse_telegram_updates(
    payload: &Value,
    allowlist: &BTreeSet<i64>,
    current_offset: i64,
    account_id: &str,
) -> CliResult<(Vec<ChannelInboundMessage>, Option<i64>)> {
    if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(format!("telegram getUpdates not ok: {payload}"));
    }

    let updates = payload
        .get("result")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut inbox = Vec::new();
    let mut max_update = current_offset.saturating_sub(1);

    for update in updates {
        let update_id = update.get("update_id").and_then(Value::as_i64).unwrap_or(0);
        if update_id > max_update {
            max_update = update_id;
        }

        let message = update.get("message").cloned().unwrap_or(Value::Null);
        let text = message
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let Some(text) = text else {
            continue;
        };

        let chat_id = message
            .get("chat")
            .and_then(|chat| chat.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let allowed = allowlist.contains(&chat_id);
        if !allowed {
            continue;
        }

        inbox.push(ChannelInboundMessage {
            session: ChannelSession::with_account(
                ChannelPlatform::Telegram,
                account_id,
                chat_id.to_string(),
            ),
            reply_target: ChannelOutboundTarget::telegram_chat(chat_id),
            text,
            delivery: ChannelDelivery {
                ack_cursor: Some(update_id.saturating_add(1).to_string()),
                source_message_id: message
                    .get("message_id")
                    .and_then(Value::as_i64)
                    .map(|value| value.to_string()),
            },
        });
    }

    let next_offset = if max_update >= current_offset {
        Some(max_update.saturating_add(1))
    } else {
        None
    };
    Ok((inbox, next_offset))
}

fn telegram_offset_path_for_account(loongclaw_home: &Path, account_id: &str) -> PathBuf {
    loongclaw_home
        .join("telegram-offsets")
        .join(format!("{}.offset", account_id.trim()))
}

fn load_offset_for_account(loongclaw_home: &Path, account_id: &str) -> Option<i64> {
    let account_path = telegram_offset_path_for_account(loongclaw_home, account_id);
    load_offset(&account_path).or_else(|| load_offset(&loongclaw_home.join("telegram.offset")))
}

fn load_offset(path: &Path) -> Option<i64> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse::<i64>().ok()
}

fn save_offset(path: &Path, next_offset: i64) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create telegram offset directory failed: {error}"))?;
    }
    fs::write(path, next_offset.to_string())
        .map_err(|error| format!("write telegram offset file failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_file_roundtrip() {
        let unique = format!(
            "loongclaw-telegram-offset-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique).join("offset.txt");
        save_offset(&path, 42).expect("save offset");
        assert_eq!(load_offset(&path), Some(42));
    }

    #[test]
    fn telegram_parser_filters_by_allowlist_and_updates_offset() {
        let payload = json!({
            "ok": true,
            "result": [
                {
                    "update_id": 100,
                    "message": {
                        "text": "hello",
                        "chat": {"id": 123}
                    }
                },
                {
                    "update_id": 101,
                    "message": {
                        "text": "blocked",
                        "chat": {"id": 456}
                    }
                }
            ]
        });

        let allowlist = BTreeSet::from([123_i64]);
        let (inbox, next_offset) = parse_telegram_updates(&payload, &allowlist, 50, "bot_123456")
            .expect("parse telegram updates");

        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].session.session_key(), "telegram:bot_123456:123");
        assert_eq!(
            inbox[0].reply_target,
            ChannelOutboundTarget::telegram_chat(123)
        );
        assert_eq!(inbox[0].text, "hello");
        assert_eq!(inbox[0].delivery.ack_cursor.as_deref(), Some("101"));
        assert_eq!(next_offset, Some(102));
    }

    #[test]
    fn telegram_parser_rejects_all_when_allowlist_is_empty() {
        let payload = json!({
            "ok": true,
            "result": [
                {
                    "update_id": 8,
                    "message": {
                        "text": "hello",
                        "chat": {"id": 42}
                    }
                }
            ]
        });

        let allowlist = BTreeSet::new();
        let (inbox, next_offset) = parse_telegram_updates(&payload, &allowlist, 0, "bot_123456")
            .expect("parse telegram updates");

        assert!(inbox.is_empty());
        assert_eq!(next_offset, Some(9));
    }

    #[test]
    fn telegram_batch_offset_is_not_persisted_until_ack() {
        let unique = format!(
            "loongclaw-telegram-batch-offset-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique).join("offset.txt");
        let mut tracker = TelegramOffsetTracker::new(path.clone(), 0);

        tracker
            .remember_polled_offset(Some(7))
            .expect("remember polled offset");

        assert_eq!(load_offset(&path), None);
        assert_eq!(tracker.current_offset(), 0);

        tracker.complete_batch().expect("complete batch");

        assert_eq!(load_offset(&path), Some(7));
        assert_eq!(tracker.current_offset(), 7);
    }

    #[test]
    fn telegram_batch_acknowledges_messages_incrementally_and_flushes_trailing_offset() {
        let unique = format!(
            "loongclaw-telegram-ack-offset-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique).join("offset.txt");
        let mut tracker = TelegramOffsetTracker::new(path.clone(), 0);

        tracker
            .remember_polled_offset(Some(12))
            .expect("remember polled offset");
        tracker
            .ack_delivery(Some("10"))
            .expect("ack successful message");

        assert_eq!(load_offset(&path), Some(10));
        assert_eq!(tracker.current_offset(), 10);

        tracker.complete_batch().expect("complete batch");

        assert_eq!(load_offset(&path), Some(12));
        assert_eq!(tracker.current_offset(), 12);
    }

    #[test]
    fn telegram_offset_path_is_account_scoped() {
        let home = std::env::temp_dir().join("loongclaw-telegram-account-offset");
        let path = telegram_offset_path_for_account(home.as_path(), "bot_123456");

        assert!(path.ends_with("telegram-offsets/bot_123456.offset"));
    }

    #[test]
    fn telegram_offset_loader_falls_back_to_legacy_single_file() {
        let unique = format!(
            "loongclaw-telegram-legacy-offset-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let home = std::env::temp_dir().join(unique);
        let legacy_path = home.join("telegram.offset");
        save_offset(&legacy_path, 77).expect("save legacy offset");

        let offset = load_offset_for_account(home.as_path(), "bot_123456");
        assert_eq!(offset, Some(77));
    }
}
