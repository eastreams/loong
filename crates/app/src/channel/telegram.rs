use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::CliResult;
use crate::config::{self, ResolvedTelegramChannelConfig};

use super::{
    ChannelAdapter, ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget,
    ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
};

const TELEGRAM_GENERAL_TOPIC_ID: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TelegramTypingTarget {
    chat_id: i64,
    thread_id: Option<i64>,
}

pub(super) struct TelegramAdapter {
    account_id: String,
    token: String,
    base_url: String,
    timeout_s: u64,
    offset_tracker: TelegramOffsetTracker,
    allowlist: BTreeSet<i64>,
    typing_handles: Mutex<HashMap<TelegramTypingTarget, tokio::task::JoinHandle<()>>>,
}

const TELEGRAM_TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

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
            typing_handles: Mutex::new(HashMap::new()),
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

    async fn send_text_to_target(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
    ) -> CliResult<()> {
        let url = self.api_url("sendMessage");
        let client = reqwest::Client::new();
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        insert_telegram_thread_id(&mut body, thread_id);

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

fn telegram_chat_thread_target(chat_id: i64, thread_id: i64) -> ChannelOutboundTarget {
    ChannelOutboundTarget::new(
        ChannelPlatform::Telegram,
        ChannelOutboundTargetKind::Conversation,
        format!("{chat_id}:topic:{thread_id}"),
    )
}

fn parse_telegram_conversation_target(
    target: &ChannelOutboundTarget,
) -> CliResult<(i64, Option<i64>)> {
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

    let trimmed = target.trimmed_id()?;
    if let Some((chat_id, thread_id)) = trimmed.split_once(":topic:") {
        let chat_id = chat_id
            .trim()
            .parse::<i64>()
            .map_err(|error| format!("invalid telegram chat id `{chat_id}`: {error}"))?;
        let thread_id = thread_id
            .trim()
            .parse::<i64>()
            .map_err(|error| format!("invalid telegram thread id `{thread_id}`: {error}"))?;
        return Ok((chat_id, Some(thread_id)));
    }

    let chat_id = trimmed
        .parse::<i64>()
        .map_err(|error| format!("invalid telegram chat id `{}`: {error}", target.id))?;
    Ok((chat_id, None))
}

fn telegram_forum_topic_id(thread_id: Option<i64>) -> Option<i64> {
    thread_id
        .filter(|thread_id| *thread_id > 0)
        .filter(|thread_id| *thread_id != TELEGRAM_GENERAL_TOPIC_ID)
}

fn telegram_inbound_thread_id(message: &Value) -> Option<i64> {
    let is_forum = message
        .get("chat")
        .and_then(|chat| chat.get("is_forum"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !is_forum {
        return None;
    }
    telegram_forum_topic_id(message.get("message_thread_id").and_then(Value::as_i64))
}

fn insert_telegram_thread_id(body: &mut Value, thread_id: Option<i64>) {
    let Some(thread_id) = telegram_forum_topic_id(thread_id) else {
        return;
    };
    let Some(object) = body.as_object_mut() else {
        return;
    };
    object.insert("message_thread_id".to_owned(), json!(thread_id));
}

fn telegram_typing_target(chat_id: i64, thread_id: Option<i64>) -> TelegramTypingTarget {
    TelegramTypingTarget {
        chat_id,
        thread_id: telegram_forum_topic_id(thread_id),
    }
}

impl TelegramAdapter {
    fn stop_typing_handle(&self, target: TelegramTypingTarget) {
        let handle = self.typing_handles_guard().remove(&target);
        if let Some(handle) = handle {
            handle.abort();
        }
    }

    fn abort_all_typing_handles(&self) {
        let mut handles = self.typing_handles_guard();
        for (_, handle) in handles.drain() {
            handle.abort();
        }
    }

    fn typing_handles_guard(
        &self,
    ) -> MutexGuard<'_, HashMap<TelegramTypingTarget, tokio::task::JoinHandle<()>>> {
        match self.typing_handles.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(super) async fn run_telegram_send(
    config: &ResolvedTelegramChannelConfig,
    token: String,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    let adapter = TelegramAdapter::new(config, token);
    let target = build_telegram_send_target(target_kind, target_id)?;
    adapter.send_text(&target, text).await
}

fn build_telegram_send_target(
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
) -> CliResult<ChannelOutboundTarget> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "telegram send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    let trimmed_target_id = target_id.trim();
    if trimmed_target_id.is_empty() {
        return Err("telegram outbound target id is empty".to_owned());
    }

    let parsed_target = ChannelOutboundTarget::new(
        ChannelPlatform::Telegram,
        ChannelOutboundTargetKind::Conversation,
        trimmed_target_id,
    );
    let (chat_id, thread_id) = parse_telegram_conversation_target(&parsed_target)?;
    Ok(match telegram_forum_topic_id(thread_id) {
        Some(thread_id) => ChannelOutboundTarget::telegram_chat_thread(chat_id, thread_id),
        None => ChannelOutboundTarget::telegram_chat(chat_id),
    })
}

impl Drop for TelegramAdapter {
    fn drop(&mut self) {
        self.abort_all_typing_handles();
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
        let (chat_id, thread_id) = parse_telegram_conversation_target(target)?;
        self.send_text_to_target(chat_id, thread_id, text).await
    }

    async fn start_typing(&self, target: &ChannelOutboundTarget) -> CliResult<()> {
        let (chat_id, thread_id) = parse_telegram_conversation_target(target)?;
        let typing_target = telegram_typing_target(chat_id, thread_id);
        self.stop_typing_handle(typing_target);

        let client = reqwest::Client::new();
        let url = self.api_url("sendChatAction");
        let handle = tokio::spawn(async move {
            loop {
                let mut body = json!({
                    "chat_id": chat_id,
                    "action": "typing",
                });
                insert_telegram_thread_id(&mut body, thread_id);
                let _ = client.post(&url).json(&body).send().await;
                tokio::time::sleep(TELEGRAM_TYPING_REFRESH_INTERVAL).await;
            }
        });

        self.typing_handles_guard().insert(typing_target, handle);
        Ok(())
    }

    async fn stop_typing(&self, target: &ChannelOutboundTarget) -> CliResult<()> {
        let (chat_id, thread_id) = parse_telegram_conversation_target(target)?;
        self.stop_typing_handle(telegram_typing_target(chat_id, thread_id));
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

        let thread_id = telegram_inbound_thread_id(&message);
        let session = match thread_id {
            Some(thread_id) => ChannelSession::with_account_and_thread(
                ChannelPlatform::Telegram,
                account_id,
                chat_id.to_string(),
                thread_id.to_string(),
            ),
            None => ChannelSession::with_account(
                ChannelPlatform::Telegram,
                account_id,
                chat_id.to_string(),
            ),
        };
        let reply_target = match thread_id {
            Some(thread_id) => telegram_chat_thread_target(chat_id, thread_id),
            None => ChannelOutboundTarget::telegram_chat(chat_id),
        };

        inbox.push(ChannelInboundMessage {
            session,
            reply_target,
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
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn test_adapter() -> TelegramAdapter {
        let unique = format!(
            "loongclaw-telegram-typing-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique).join("offset.txt");
        TelegramAdapter {
            account_id: "bot_123456".to_owned(),
            token: "fake-token".to_owned(),
            base_url: "http://127.0.0.1:9".to_owned(),
            timeout_s: 1,
            offset_tracker: TelegramOffsetTracker::new(path, 0),
            allowlist: BTreeSet::from([123_i64]),
            typing_handles: Mutex::new(HashMap::new()),
        }
    }

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
    fn telegram_parser_uses_forum_topic_context_for_sessions_and_reply_targets() {
        let payload = json!({
            "ok": true,
            "result": [
                {
                    "update_id": 100,
                    "message": {
                        "message_id": 42,
                        "message_thread_id": 7,
                        "text": "topic hello",
                        "chat": {"id": 123, "is_forum": true}
                    }
                }
            ]
        });

        let allowlist = BTreeSet::from([123_i64]);
        let (inbox, next_offset) = parse_telegram_updates(&payload, &allowlist, 50, "bot_123456")
            .expect("parse telegram forum topic updates");

        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].session.session_key(), "telegram:bot_123456:123:7");
        assert_eq!(inbox[0].reply_target, telegram_chat_thread_target(123, 7));
        assert_eq!(inbox[0].text, "topic hello");
        assert_eq!(inbox[0].delivery.ack_cursor.as_deref(), Some("101"));
        assert_eq!(next_offset, Some(101));
    }

    #[test]
    fn parse_telegram_conversation_target_supports_chat_and_topic_ids() {
        let chat_only = ChannelOutboundTarget::telegram_chat(123);
        let topic = ChannelOutboundTarget::telegram_chat_thread(123, 7);

        assert_eq!(
            parse_telegram_conversation_target(&chat_only).expect("chat target"),
            (123, None)
        );
        assert_eq!(
            parse_telegram_conversation_target(&topic).expect("topic target"),
            (123, Some(7))
        );
    }

    #[test]
    fn build_telegram_send_target_supports_chat_and_topic_ids() {
        let chat_only =
            build_telegram_send_target(ChannelOutboundTargetKind::Conversation, " 123 ")
                .expect("chat target");
        let topic =
            build_telegram_send_target(ChannelOutboundTargetKind::Conversation, " 123:topic:7 ")
                .expect("topic target");

        assert_eq!(chat_only, ChannelOutboundTarget::telegram_chat(123));
        assert_eq!(topic, ChannelOutboundTarget::telegram_chat_thread(123, 7));
    }

    #[test]
    fn build_telegram_send_target_rejects_non_conversation_target_kinds() {
        assert_eq!(
            build_telegram_send_target(ChannelOutboundTargetKind::MessageReply, "om_123")
                .expect_err("message reply targets should be rejected"),
            "telegram send requires conversation target kind, got message_reply"
        );
    }

    #[test]
    fn telegram_thread_helpers_drop_general_topic_and_non_forum_context() {
        assert_eq!(
            telegram_forum_topic_id(Some(TELEGRAM_GENERAL_TOPIC_ID)),
            None
        );
        assert_eq!(telegram_forum_topic_id(Some(7)), Some(7));

        let forum_message = json!({
            "chat": {"id": 123, "is_forum": true},
            "message_thread_id": TELEGRAM_GENERAL_TOPIC_ID,
        });
        let non_forum_message = json!({
            "chat": {"id": 123, "is_forum": false},
            "message_thread_id": 7,
        });

        assert_eq!(telegram_inbound_thread_id(&forum_message), None);
        assert_eq!(telegram_inbound_thread_id(&non_forum_message), None);
    }

    #[test]
    fn insert_telegram_thread_id_omits_general_topic_targets() {
        let mut general_topic_body = json!({
            "chat_id": 123,
            "action": "typing",
        });
        let mut forum_topic_body = general_topic_body.clone();

        insert_telegram_thread_id(&mut general_topic_body, Some(TELEGRAM_GENERAL_TOPIC_ID));
        insert_telegram_thread_id(&mut forum_topic_body, Some(7));

        assert_eq!(general_topic_body.get("message_thread_id"), None);
        assert_eq!(forum_topic_body.get("message_thread_id"), Some(&json!(7)));
    }

    #[tokio::test]
    async fn telegram_send_text_omits_message_thread_id_for_general_topic_targets() {
        let (base_url, requests, server) =
            spawn_request_sequence_server(vec![(200, r#"{"ok":true}"#.to_owned())]);
        let adapter = TelegramAdapter {
            account_id: "bot_123456".to_owned(),
            token: "telegram-token".to_owned(),
            base_url,
            timeout_s: 5,
            offset_tracker: TelegramOffsetTracker::new(temp_offset_path("general-topic"), 0),
            allowlist: BTreeSet::new(),
            typing_handles: Mutex::new(HashMap::new()),
        };

        adapter
            .send_text(
                &telegram_chat_thread_target(123, TELEGRAM_GENERAL_TOPIC_ID),
                "hello telegram general",
            )
            .await
            .expect("telegram general topic send should succeed");

        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("capture telegram send request");

        assert!(request.contains(r#""chat_id":123"#));
        assert!(!request.contains(r#""message_thread_id":"#));

        server.join().expect("join telegram general topic server");
    }

    #[tokio::test]
    async fn telegram_send_text_includes_message_thread_id_for_forum_topics() {
        let (base_url, requests, server) =
            spawn_request_sequence_server(vec![(200, r#"{"ok":true}"#.to_owned())]);
        let adapter = TelegramAdapter {
            account_id: "bot_123456".to_owned(),
            token: "telegram-token".to_owned(),
            base_url,
            timeout_s: 5,
            offset_tracker: TelegramOffsetTracker::new(temp_offset_path("forum-topic"), 0),
            allowlist: BTreeSet::new(),
            typing_handles: Mutex::new(HashMap::new()),
        };

        adapter
            .send_text(&telegram_chat_thread_target(123, 7), "hello telegram topic")
            .await
            .expect("telegram topic send should succeed");

        let request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("capture telegram topic send request");

        assert!(request.contains(r#""chat_id":123"#));
        assert!(request.contains(r#""message_thread_id":7"#));

        server.join().expect("join telegram topic server");
    }

    #[tokio::test]
    async fn telegram_start_typing_omits_message_thread_id_for_general_topic_targets() {
        let (base_url, requests, server) =
            spawn_request_sequence_server(vec![(200, r#"{"ok":true}"#.to_owned())]);
        let adapter = TelegramAdapter {
            account_id: "bot_123456".to_owned(),
            token: "telegram-token".to_owned(),
            base_url,
            timeout_s: 5,
            offset_tracker: TelegramOffsetTracker::new(temp_offset_path("typing-general-topic"), 0),
            allowlist: BTreeSet::new(),
            typing_handles: Mutex::new(HashMap::new()),
        };
        let target = telegram_chat_thread_target(123, TELEGRAM_GENERAL_TOPIC_ID);

        adapter
            .start_typing(&target)
            .await
            .expect("start typing for telegram general topic should succeed");

        let request =
            tokio::task::spawn_blocking(move || requests.recv_timeout(Duration::from_secs(2)))
                .await
                .expect("join telegram typing capture task")
                .expect("capture telegram typing request");

        assert!(request.contains(r#""chat_id":123"#));
        assert!(!request.contains(r#""message_thread_id":"#));

        adapter
            .stop_typing(&target)
            .await
            .expect("stop typing for telegram general topic should succeed");

        server
            .join()
            .expect("join telegram general topic typing server");
    }

    #[tokio::test]
    async fn telegram_start_typing_includes_message_thread_id_for_forum_topics() {
        let (base_url, requests, server) =
            spawn_request_sequence_server(vec![(200, r#"{"ok":true}"#.to_owned())]);
        let adapter = TelegramAdapter {
            account_id: "bot_123456".to_owned(),
            token: "telegram-token".to_owned(),
            base_url,
            timeout_s: 5,
            offset_tracker: TelegramOffsetTracker::new(temp_offset_path("typing-topic"), 0),
            allowlist: BTreeSet::new(),
            typing_handles: Mutex::new(HashMap::new()),
        };
        let target = telegram_chat_thread_target(123, 7);

        adapter
            .start_typing(&target)
            .await
            .expect("start typing for telegram topic should succeed");

        let request =
            tokio::task::spawn_blocking(move || requests.recv_timeout(Duration::from_secs(2)))
                .await
                .expect("join telegram typing capture task")
                .expect("capture telegram typing request");

        assert!(request.contains(r#""chat_id":123"#));
        assert!(request.contains(r#""message_thread_id":7"#));

        adapter
            .stop_typing(&target)
            .await
            .expect("stop typing for telegram topic should succeed");

        server.join().expect("join telegram topic typing server");
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

    #[tokio::test]
    async fn telegram_stop_typing_is_idempotent_and_clears_handle() {
        let adapter = test_adapter();
        let target = ChannelOutboundTarget::telegram_chat(123);

        adapter
            .start_typing(&target)
            .await
            .expect("start typing should succeed");
        adapter
            .stop_typing(&target)
            .await
            .expect("first stop typing should succeed");
        adapter
            .stop_typing(&target)
            .await
            .expect("second stop typing should succeed");

        assert!(
            adapter
                .typing_handles
                .lock()
                .expect("typing handles lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn telegram_start_typing_replaces_existing_handle_for_same_target() {
        let adapter = test_adapter();
        let target = ChannelOutboundTarget::telegram_chat(123);
        let dropped = Arc::new(AtomicBool::new(false));
        let dropped_for_task = Arc::clone(&dropped);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _flag = DropFlag(dropped_for_task);
            let _ = started_tx.send(());
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        adapter
            .typing_handles
            .lock()
            .expect("typing handles lock")
            .insert(telegram_typing_target(123, None), handle);

        started_rx.await.expect("typing task should start");

        adapter
            .start_typing(&target)
            .await
            .expect("start typing should succeed");

        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(
            adapter
                .typing_handles
                .lock()
                .expect("typing handles lock")
                .len(),
            1
        );

        adapter
            .stop_typing(&target)
            .await
            .expect("stop typing should succeed");
    }

    #[tokio::test]
    async fn telegram_start_typing_keeps_distinct_handles_for_different_topics() {
        let adapter = test_adapter();
        let topic_a = telegram_chat_thread_target(123, 7);
        let topic_b = telegram_chat_thread_target(123, 8);

        adapter
            .start_typing(&topic_a)
            .await
            .expect("start typing for first topic should succeed");
        adapter
            .start_typing(&topic_b)
            .await
            .expect("start typing for second topic should succeed");

        assert_eq!(
            adapter
                .typing_handles
                .lock()
                .expect("typing handles lock")
                .len(),
            2
        );

        adapter
            .stop_typing(&topic_a)
            .await
            .expect("stop typing for first topic should succeed");
        adapter
            .stop_typing(&topic_b)
            .await
            .expect("stop typing for second topic should succeed");
    }

    #[tokio::test]
    async fn telegram_adapter_drop_aborts_active_typing_handles() {
        let dropped = Arc::new(AtomicBool::new(false));
        let dropped_for_task = Arc::clone(&dropped);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _flag = DropFlag(dropped_for_task);
            let _ = started_tx.send(());
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        let adapter = test_adapter();
        adapter
            .typing_handles
            .lock()
            .expect("typing handles lock")
            .insert(telegram_typing_target(123, None), handle);

        started_rx.await.expect("typing task should start");

        drop(adapter);
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(
            dropped.load(Ordering::SeqCst),
            "dropping the adapter should abort active typing handles"
        );
    }

    #[tokio::test]
    async fn telegram_typing_rejects_non_telegram_target() {
        let adapter = test_adapter();
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::Conversation,
            "oc_123",
        );

        let error = adapter
            .start_typing(&target)
            .await
            .expect_err("non telegram target should fail");

        assert_eq!(error, "telegram adapter cannot send to feishu target");
    }

    #[tokio::test]
    async fn telegram_typing_rejects_non_conversation_target() {
        let adapter = test_adapter();
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Telegram,
            ChannelOutboundTargetKind::MessageReply,
            "123",
        );

        let error = adapter
            .start_typing(&target)
            .await
            .expect_err("non conversation target should fail");

        assert_eq!(
            error,
            "telegram adapter requires conversation target, got message_reply"
        );
    }

    fn temp_offset_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "loongclaw-telegram-send-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join("offset.txt")
    }

    fn spawn_request_sequence_server(
        responses: Vec<(u16, String)>,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind telegram test server");
        let address = listener.local_addr().expect("telegram test server address");
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            for (status_code, response_body) in responses {
                let (mut stream, _) = listener.accept().expect("accept telegram request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("set telegram read timeout");

                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            request.extend_from_slice(&buffer[..read]);
                            if request_complete(request.as_slice()) {
                                break;
                            }
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            break;
                        }
                        Err(error) => panic!("read telegram test request failed: {error}"),
                    }
                }

                let body = extract_request_body(request.as_slice());
                tx.send(body).expect("capture telegram request body");

                let response = format!(
                    "HTTP/1.1 {} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    status_code,
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write telegram test response");
            }
        });

        (format!("http://{address}"), rx, handle)
    }

    fn request_complete(buffer: &[u8]) -> bool {
        let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let header_end = header_end + 4;
        let content_length = String::from_utf8_lossy(&buffer[..header_end])
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if !name.eq_ignore_ascii_case("content-length") {
                    return None;
                }
                value.trim().parse::<usize>().ok()
            })
            .unwrap_or(0);
        buffer.len() >= header_end + content_length
    }

    fn extract_request_body(request: &[u8]) -> String {
        let request = String::from_utf8_lossy(request);
        request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default()
    }
}
