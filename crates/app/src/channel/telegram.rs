use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::{self, LoongClawConfig};
use crate::CliResult;

use super::{ChannelAdapter, ChannelInboundMessage};

pub(super) struct TelegramAdapter {
    token: String,
    base_url: String,
    timeout_s: u64,
    offset_path: PathBuf,
    next_offset: i64,
    allowlist: BTreeSet<i64>,
}

impl TelegramAdapter {
    pub(super) fn new(config: &LoongClawConfig, token: String) -> Self {
        let offset_path = config::default_loongclaw_home().join("telegram.offset");
        let next_offset = load_offset(&offset_path).unwrap_or(0);
        Self {
            token,
            base_url: config.telegram.base_url.clone(),
            timeout_s: config.telegram.polling_timeout_s.clamp(1, 50),
            offset_path,
            next_offset,
            allowlist: config.telegram.allowed_chat_ids.iter().copied().collect(),
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
            "offset": self.next_offset,
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

        let (inbox, next_offset) =
            parse_telegram_updates(&payload, &self.allowlist, self.next_offset)?;
        if let Some(next) = next_offset {
            self.next_offset = next;
            save_offset(&self.offset_path, self.next_offset)?;
        }

        Ok(inbox)
    }

    async fn send_text(&self, target: &str, text: &str) -> CliResult<()> {
        let chat_id = target
            .trim()
            .parse::<i64>()
            .map_err(|error| format!("invalid telegram chat id `{target}`: {error}"))?;

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
}

pub(super) fn parse_telegram_updates(
    payload: &Value,
    allowlist: &BTreeSet<i64>,
    current_offset: i64,
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
        let allowed = allowlist.is_empty() || allowlist.contains(&chat_id);
        if !allowed {
            continue;
        }

        inbox.push(ChannelInboundMessage {
            session_id: format!("telegram:{chat_id}"),
            reply_target: chat_id.to_string(),
            text,
        });
    }

    let next_offset = if max_update >= current_offset {
        Some(max_update.saturating_add(1))
    } else {
        None
    };
    Ok((inbox, next_offset))
}

fn load_offset(path: &Path) -> Option<i64> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse::<i64>().ok()
}

fn save_offset(path: &Path, next_offset: i64) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create telegram offset directory failed: {error}"))?;
        }
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
        let (inbox, next_offset) =
            parse_telegram_updates(&payload, &allowlist, 50).expect("parse telegram updates");

        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].session_id, "telegram:123");
        assert_eq!(inbox[0].text, "hello");
        assert_eq!(next_offset, Some(102));
    }
}
