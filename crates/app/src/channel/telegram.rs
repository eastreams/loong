use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::CliResult;
use crate::config::{self, ResolvedTelegramChannelConfig, TelegramStreamingMode};

use super::{
    ChannelAdapter, ChannelDelivery, ChannelInboundMessage, ChannelOutboundMessage,
    ChannelOutboundTarget, ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
    ChannelStreamingMode,
};

const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
const TELEGRAM_CONTINUATION_OVERHEAD: usize = 30;
const TELEGRAM_ACK_REACTIONS: &[&str] =
    &["⚡️", "👌", "👀", "🔥", "👍", "💪", "🤩", "😎", "🤘", "🙌"];

pub(super) struct TelegramAdapter {
    account_id: String,
    token: String,
    base_url: String,
    timeout_s: u64,
    offset_tracker: TelegramOffsetTracker,
    allowlist: BTreeSet<i64>,
    http_client: reqwest::Client,
    ack_reactions: bool,
    streaming_mode: TelegramStreamingMode,
    draft_update_interval_ms: u64,
    last_draft_edit: HashMap<String, Instant>,
    pending_reactions: Vec<(i64, i64)>,
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
            http_client: reqwest::Client::new(),
            ack_reactions: config.ack_reactions,
            streaming_mode: config.streaming_mode,
            draft_update_interval_ms: 500,
            last_draft_edit: HashMap::new(),
            pending_reactions: Vec::new(),
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

fn split_message_for_telegram(message: &str) -> Vec<String> {
    if message.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return vec![message.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = message;

    while !remaining.is_empty() {
        let is_first = chunks.is_empty();
        let limit = if is_first {
            TELEGRAM_MAX_MESSAGE_LENGTH
        } else {
            TELEGRAM_MAX_MESSAGE_LENGTH - TELEGRAM_CONTINUATION_OVERHEAD
        };

        if remaining.chars().count() <= limit {
            chunks.push(remaining.to_string());
            break;
        }

        let hard_split = remaining
            .char_indices()
            .nth(limit)
            .map_or(remaining.len(), |(idx, _)| idx);

        let chunk_end = if hard_split == remaining.len() {
            hard_split
        } else {
            let search_area = &remaining[..hard_split];

            let candidate = search_area
                .rfind('\n')
                .map(|pos| pos + 1)
                .or_else(|| search_area.rfind(' ').map(|pos| pos + 1));

            match candidate {
                Some(pos) if pos <= hard_split => pos,
                _ => hard_split,
            }
        };

        chunks.push(remaining[..chunk_end].to_string());
        remaining = &remaining[chunk_end..];
    }

    chunks
}

fn pick_uniform_index(len: usize) -> usize {
    debug_assert!(len > 0);
    let upper = len as u64;
    let reject_threshold = (u64::MAX / upper) * upper;

    loop {
        let value = rand::random::<u64>();
        if value < reject_threshold {
            #[allow(clippy::cast_possible_truncation)]
            return (value % upper) as usize;
        }
    }
}

fn random_ack_reaction() -> &'static str {
    let index = pick_uniform_index(TELEGRAM_ACK_REACTIONS.len());
    TELEGRAM_ACK_REACTIONS.get(index).unwrap_or(&"⚡️")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[allow(clippy::collapsible_if)]
fn markdown_to_telegram_html(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result_lines: Vec<String> = Vec::new();
    let mut in_fenced_block = false;

    for line in &lines {
        let trimmed_line = line.trim_start();
        if trimmed_line.starts_with("```") {
            in_fenced_block = !in_fenced_block;
            result_lines.push(trimmed_line.to_string());
            continue;
        }

        if in_fenced_block {
            result_lines.push(line.to_string());
            continue;
        }

        let mut line_out = String::new();

        let stripped = line.trim_start_matches('#');
        let header_level = line.len() - stripped.len();
        if header_level > 0 && line.starts_with('#') && stripped.starts_with(' ') {
            let title = escape_html(stripped.trim());
            result_lines.push(format!("<b>{title}</b>"));
            continue;
        }

        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            if i + 1 < len
                && bytes.get(i) == Some(&b'*')
                && bytes.get(i + 1) == Some(&b'*')
                && let Some(end) = line[i + 2..].find("**")
            {
                let inner = escape_html(&line[i + 2..i + 2 + end]);
                let _ = write!(line_out, "<b>{inner}</b>");
                i += 4 + end;
                continue;
            }
            if i + 1 < len
                && bytes.get(i) == Some(&b'_')
                && bytes.get(i + 1) == Some(&b'_')
                && let Some(end) = line[i + 2..].find("__")
            {
                let inner = escape_html(&line[i + 2..i + 2 + end]);
                let _ = write!(line_out, "<b>{inner}</b>");
                i += 4 + end;
                continue;
            }
            if bytes.get(i) == Some(&b'*')
                && (i == 0 || bytes.get(i - 1) != Some(&b'*'))
                && let Some(end) = line[i + 1..].find('*')
                && end > 0
            {
                let inner = escape_html(&line[i + 1..i + 1 + end]);
                let _ = write!(line_out, "<i>{inner}</i>");
                i += 2 + end;
                continue;
            }
            if bytes.get(i) == Some(&b'`')
                && (i == 0 || bytes.get(i - 1) != Some(&b'`'))
                && let Some(end) = line[i + 1..].find('`')
            {
                let inner = escape_html(&line[i + 1..i + 1 + end]);
                let _ = write!(line_out, "<code>{inner}</code>");
                i += 2 + end;
                continue;
            }
            if bytes.get(i) == Some(&b'[') {
                if let Some(bracket_end) = line[i + 1..].find(']') {
                    let text_part = &line[i + 1..i + 1 + bracket_end];
                    let after_bracket = i + 1 + bracket_end + 1;
                    if after_bracket < len && bytes.get(after_bracket) == Some(&b'(') {
                        if let Some(paren_end) = line[after_bracket + 1..].find(')') {
                            let url = &line[after_bracket + 1..after_bracket + 1 + paren_end];
                            if url.starts_with("http://") || url.starts_with("https://") {
                                let text_html = escape_html(text_part);
                                let url_html = escape_html(url);
                                let _ = write!(line_out, "<a href=\"{url_html}\">{text_html}</a>");
                                i = after_bracket + 1 + paren_end + 1;
                                continue;
                            }
                        }
                    }
                }
            }
            if i + 1 < len
                && bytes.get(i) == Some(&b'~')
                && bytes.get(i + 1) == Some(&b'~')
                && let Some(end) = line[i + 2..].find("~~")
            {
                let inner = escape_html(&line[i + 2..i + 2 + end]);
                let _ = write!(line_out, "<s>{inner}</s>");
                i += 4 + end;
                continue;
            }
            #[allow(clippy::unwrap_used)]
            let ch = line[i..].chars().next().unwrap();
            match ch {
                '<' => line_out.push_str("&lt;"),
                '>' => line_out.push_str("&gt;"),
                '&' => line_out.push_str("&amp;"),
                '"' => line_out.push_str("&quot;"),
                '\'' => line_out.push_str("&#39;"),
                _ => line_out.push(ch),
            }
            i += ch.len_utf8();
        }
        result_lines.push(line_out);
    }

    let joined = result_lines.join("\n");
    let mut final_out = String::with_capacity(joined.len());
    let mut in_code_block = false;
    let mut code_buf = String::new();

    for line in joined.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code_block {
                in_code_block = false;
                let escaped = code_buf.trim_end_matches('\n');
                let _ = writeln!(final_out, "<pre><code>{escaped}</code></pre>");
                code_buf.clear();
            } else {
                in_code_block = true;
                code_buf.clear();
            }
        } else if in_code_block {
            code_buf.push_str(line);
            code_buf.push('\n');
        } else {
            final_out.push_str(line);
            final_out.push('\n');
        }
    }
    if in_code_block && !code_buf.is_empty() {
        let _ = writeln!(final_out, "<pre><code>{}</code></pre>", code_buf.trim_end());
    }

    final_out.trim_end_matches('\n').to_string()
}

fn parse_telegram_target(target_id: &str) -> CliResult<(i64, Option<i64>)> {
    let id = target_id.trim();
    if id.is_empty() {
        return Err("telegram target id is empty".to_owned());
    }

    if let Some((chat_id_str, thread_id_str)) = id.split_once(':') {
        let chat_id = chat_id_str
            .parse::<i64>()
            .map_err(|e| format!("invalid telegram chat id `{}`: {}", chat_id_str, e))?;
        let thread_id = thread_id_str
            .parse::<i64>()
            .map_err(|e| format!("invalid telegram thread id `{}`: {}", thread_id_str, e))?;
        Ok((chat_id, Some(thread_id)))
    } else {
        let chat_id = id
            .parse::<i64>()
            .map_err(|e| format!("invalid telegram chat id `{}`: {}", id, e))?;
        Ok((chat_id, None))
    }
}

impl TelegramAdapter {
    fn send_typing_action_nonblocking(&self, chat_id: i64) {
        let client = self.http_client.clone();
        let url = self.api_url("sendChatAction");
        let body = json!({
            "chat_id": chat_id,
            "action": "typing"
        });

        tokio::spawn(async move {
            let _ = client.post(&url).json(&body).send().await;
        });
    }

    fn send_ack_reaction_nonblocking(&self, chat_id: i64, message_id: i64) {
        if !self.ack_reactions {
            return;
        }

        let client = self.http_client.clone();
        let url = self.api_url("setMessageReaction");
        let emoji = random_ack_reaction().to_string();
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{
                "type": "emoji",
                "emoji": emoji
            }]
        });

        tokio::spawn(async move {
            let _ = client.post(&url).json(&body).send().await;
        });
    }

    async fn send_draft(
        &self,
        chat_id: i64,
        thread_id: Option<&str>,
        text: &str,
    ) -> CliResult<String> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
        });
        if let Some(tid) = thread_id
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert(
                "message_thread_id".to_string(),
                serde_json::Value::String(tid.to_string()),
            );
        }

        let response = self
            .http_client
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("telegram sendMessage failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("telegram sendMessage decode failed: {error}"))?;

        if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(format!("telegram sendMessage not ok: {response}"));
        }

        let message_id = response
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(Value::as_i64)
            .map(|id| id.to_string())
            .ok_or_else(|| "telegram sendMessage response missing message_id".to_string())?;

        Ok(message_id)
    }

    async fn update_draft(&mut self, chat_id: i64, message_id: &str, text: &str) -> CliResult<()> {
        let key = format!("{}:{}", chat_id, message_id);
        let now = Instant::now();

        if let Some(last_edit) = self.last_draft_edit.get(&key) {
            let elapsed = now.duration_since(*last_edit);
            let min_interval = std::time::Duration::from_millis(self.draft_update_interval_ms);
            if elapsed < min_interval {
                return Ok(());
            }
        }

        let html = markdown_to_telegram_html(text);
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": html,
            "parse_mode": "HTML",
        });

        let response = self
            .http_client
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("telegram editMessageText failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("telegram editMessageText decode failed: {error}"))?;

        if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(format!("telegram editMessageText not ok: {response}"));
        }

        self.last_draft_edit.insert(key, now);
        Ok(())
    }

    async fn cancel_draft(&self, chat_id: i64, message_id: &str) -> CliResult<()> {
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });

        let response = self
            .http_client
            .post(self.api_url("deleteMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("telegram deleteMessage failed: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("telegram deleteMessage decode failed: {error}"))?;

        if !response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(format!("telegram deleteMessage not ok: {response}"));
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn streaming_mode(&self) -> ChannelStreamingMode {
        match self.streaming_mode {
            TelegramStreamingMode::Off => ChannelStreamingMode::Off,
            TelegramStreamingMode::Draft => ChannelStreamingMode::Draft,
        }
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
        let url = self.api_url("getUpdates");
        let body = json!({
            "offset": self.offset_tracker.current_offset(),
            "timeout": self.timeout_s,
            "allowed_updates": ["message"],
        });
        let payload = self
            .http_client
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

        self.pending_reactions.clear();
        for message in &inbox {
            let chat_id = match message.session.conversation_id.parse::<i64>() {
                Ok(id) => id,
                Err(_) => continue,
            };
            let message_id = match message
                .delivery
                .source_message_id
                .as_ref()
                .and_then(|s| s.parse::<i64>().ok())
            {
                Some(id) => id,
                None => continue,
            };

            self.send_typing_action_nonblocking(chat_id);
            self.pending_reactions.push((chat_id, message_id));
        }

        Ok(inbox)
    }

    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        message: &ChannelOutboundMessage,
    ) -> CliResult<()> {
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

        let text = match message {
            ChannelOutboundMessage::Text(text) => text.clone(),
            ChannelOutboundMessage::MarkdownCard(text) => text.clone(),
            other @ ChannelOutboundMessage::Post(_)
            | other @ ChannelOutboundMessage::Image { .. }
            | other @ ChannelOutboundMessage::File { .. } => {
                let kind_name = if matches!(other, ChannelOutboundMessage::Post(_)) {
                    "Post"
                } else if matches!(other, ChannelOutboundMessage::Image { .. }) {
                    "Image"
                } else {
                    "File"
                };
                return Err(format!(
                    "telegram adapter does not support {} outbound messages",
                    kind_name
                ));
            }
        };

        let (chat_id, thread_id) = parse_telegram_target(&target.id)?;

        let chunks = split_message_for_telegram(&text);
        for (index, chunk) in chunks.iter().enumerate() {
            let text_to_send = if chunks.len() > 1 {
                if index == 0 {
                    format!("{chunk}\n\n(continues...)")
                } else if index == chunks.len() - 1 {
                    format!("(continued)\n\n{chunk}")
                } else {
                    format!("(continued)\n\n{chunk}\n\n(continues...)")
                }
            } else {
                chunk.clone()
            };

            let html = markdown_to_telegram_html(&text_to_send);
            let mut body = json!({
                "chat_id": chat_id,
                "text": html,
                "parse_mode": "HTML",
                "disable_web_page_preview": true,
            });
            if let Some(tid) = thread_id
                && let Some(obj) = body.as_object_mut()
            {
                obj.insert(
                    "message_thread_id".to_string(),
                    serde_json::Value::Number(tid.into()),
                );
            }

            let payload = self
                .http_client
                .post(self.api_url("sendMessage"))
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

            if index < chunks.len() - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        Ok(())
    }

    async fn send_message_streaming(
        &mut self,
        target: &ChannelOutboundTarget,
        message: &ChannelOutboundMessage,
        streaming_mode: ChannelStreamingMode,
    ) -> CliResult<()> {
        if streaming_mode == ChannelStreamingMode::Off {
            return self.send_message(target, message).await;
        }

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

        // MarkdownCard is treated as markdown and converted to HTML by update_draft.
        // This is consistent with send_message, where MarkdownCard is passed through
        // markdown_to_telegram_html before sending.
        let text = match message {
            ChannelOutboundMessage::Text(text) => text,
            ChannelOutboundMessage::MarkdownCard(text) => text,
            ChannelOutboundMessage::Post(_)
            | ChannelOutboundMessage::Image { .. }
            | ChannelOutboundMessage::File { .. } => {
                return Err("telegram streaming does not support this message type".to_string());
            }
        };

        let (chat_id, thread_id) = parse_telegram_target(&target.id)?;

        let placeholder = "Thinking...";
        let thread_id_str = thread_id.map(|tid| tid.to_string());
        let draft_id = self
            .send_draft(chat_id, thread_id_str.as_deref(), placeholder)
            .await?;

        if self.update_draft(chat_id, &draft_id, text).await.is_err() {
            let _ = self.cancel_draft(chat_id, &draft_id).await;
            return self.send_message(target, message).await;
        }

        Ok(())
    }

    async fn ack_inbound(&mut self, message: &ChannelInboundMessage) -> CliResult<()> {
        self.offset_tracker
            .ack_delivery(message.delivery.ack_cursor.as_deref())
    }

    async fn complete_batch(&mut self) -> CliResult<()> {
        self.offset_tracker.complete_batch()?;
        let reactions: Vec<_> = std::mem::take(&mut self.pending_reactions);
        for (chat_id, message_id) in reactions {
            self.send_ack_reaction_nonblocking(chat_id, message_id);
        }
        Ok(())
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

    Ok(ChannelOutboundTarget::new(
        ChannelPlatform::Telegram,
        target_kind,
        trimmed_target_id.to_owned(),
    ))
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

        let thread_id = message
            .get("message_thread_id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string());

        let reply_target = if let Some(ref tid) = thread_id {
            ChannelOutboundTarget::new(
                ChannelPlatform::Telegram,
                ChannelOutboundTargetKind::Conversation,
                format!("{}:{}", chat_id, tid),
            )
        } else {
            ChannelOutboundTarget::telegram_chat(chat_id)
        };

        let mut session = ChannelSession::with_account(
            ChannelPlatform::Telegram,
            account_id,
            chat_id.to_string(),
        );
        if let Some(ref tid) = thread_id {
            session.thread_id = Some(tid.clone());
        }

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
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
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

    #[test]
    fn split_message_for_telegram_short_message() {
        let short = "Hello, world!";
        let chunks = split_message_for_telegram(short);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], short);
    }

    #[test]
    fn split_message_for_telegram_exactly_limit() {
        let exactly: String = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH);
        let chunks = split_message_for_telegram(&exactly);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], exactly);
    }

    #[test]
    fn split_message_for_telegram_over_limit() {
        let over: String = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 100);
        let chunks = split_message_for_telegram(&over);
        assert!(chunks.len() > 1);
        // All chars should be preserved
        let reconstructed: String = chunks.iter().map(|s| s.as_str()).collect();
        assert_eq!(reconstructed.len(), over.len());
    }

    #[test]
    fn split_message_for_telegram_preserves_content() {
        let text = "Hello\n\nWorld this is a test message that is longer than the limit";
        let chunks = split_message_for_telegram(text);
        let reconstructed: String = chunks.iter().map(|s| s.as_str()).collect();
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn markdown_to_telegram_html_bold() {
        let input = "This is **bold** text";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<b>bold</b>"));
    }

    #[test]
    fn markdown_to_telegram_html_italic() {
        let input = "This is *italic* text";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<i>italic</i>"));
    }

    #[test]
    fn markdown_to_telegram_html_code() {
        let input = "Use `console.log()` for debugging";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<code>console.log()</code>"));
    }

    #[test]
    fn markdown_to_telegram_html_link() {
        let input = "Visit [our site](https://example.com) please";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<a href=\"https://example.com\">our site</a>"));
    }

    #[test]
    fn markdown_to_telegram_html_escape_html() {
        let input = "3 < 5 & 7 > 2";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("&lt;"));
        assert!(output.contains("&gt;"));
        assert!(output.contains("&amp;"));
    }

    #[test]
    fn markdown_to_telegram_html_header() {
        let input = "## Title";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<b>Title</b>"));
    }

    #[test]
    fn parse_telegram_target_chat_id_only() {
        let (chat_id, thread_id) = parse_telegram_target("123456789").unwrap();
        assert_eq!(chat_id, 123456789);
        assert!(thread_id.is_none());
    }

    #[test]
    fn parse_telegram_target_with_thread_id() {
        let (chat_id, thread_id) = parse_telegram_target("123456789:42").unwrap();
        assert_eq!(chat_id, 123456789);
        assert_eq!(thread_id, Some(42));
    }

    #[test]
    fn parse_telegram_target_negative_chat_id() {
        let (chat_id, thread_id) = parse_telegram_target("-1001234567890:42").unwrap();
        assert_eq!(chat_id, -1001234567890);
        assert_eq!(thread_id, Some(42));
    }

    #[test]
    fn parse_telegram_target_empty() {
        let result = parse_telegram_target("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_telegram_updates_with_thread_id() {
        let payload = json!({
            "ok": true,
            "result": [
                {
                    "update_id": 100,
                    "message": {
                        "text": "hello from thread",
                        "chat": {"id": 123},
                        "message_thread_id": 42
                    }
                }
            ]
        });

        let allowlist = BTreeSet::from([123_i64]);
        let (inbox, _next_offset) = parse_telegram_updates(&payload, &allowlist, 0, "bot_123456")
            .expect("parse telegram updates");

        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].session.thread_id, Some("42".to_string()));
        assert_eq!(inbox[0].reply_target.id, "123:42");
    }

    #[test]
    fn random_ack_reaction_is_valid() {
        for _ in 0..100 {
            let emoji = random_ack_reaction();
            assert!(
                TELEGRAM_ACK_REACTIONS.contains(&emoji),
                "Unexpected emoji: {}",
                emoji
            );
        }
    }
}
