use loong_contracts::ToolCoreOutcome;
use serde_json::{Value, json};

use super::payload::required_payload_string;

use crate::config::{LoongConfig, ToolConfig};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository,
};
use crate::session::store::SessionStoreConfig;

const SESSION_MESSAGE_SENT_EVENT_KIND: &str = "session_message_sent";

pub(crate) async fn execute_sessions_send_with_config(
    payload: Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    app_config: &LoongConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (
            payload,
            current_session_id,
            memory_config,
            tool_config,
            app_config,
        );
        return Err(
            "session tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.messages.enabled {
            return Err("app_tool_disabled: messaging tools are disabled by config".to_owned());
        }

        let session_id = required_payload_string(&payload, "session_id", "sessions_send")?;
        let text = required_payload_text(&payload)?;
        let text_length = text.chars().count();

        let repo = SessionRepository::new(memory_config)?;
        let target_summary = repo
            .load_session_summary_with_legacy_fallback(&session_id)?
            .ok_or_else(|| format!("session_not_found: `{session_id}`"))?;
        if target_summary.kind != SessionKind::Root {
            return Err(format!(
                "sessions_send_not_supported: session `{session_id}` is not a root session"
            ));
        }
        if repo.load_session(&session_id)?.is_none() {
            let _ = repo.ensure_session(NewSessionRecord {
                session_id: target_summary.session_id.clone(),
                kind: target_summary.kind,
                parent_session_id: target_summary.parent_session_id.clone(),
                label: target_summary.label.clone(),
                state: target_summary.state,
            })?;
        }

        let receipt =
            crate::channel::send_text_to_known_session(app_config, &session_id, &text).await?;
        repo.append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_kind: SESSION_MESSAGE_SENT_EVENT_KIND.to_owned(),
            actor_session_id: Some(current_session_id.to_owned()),
            payload_json: json!({
                "channel": receipt.channel,
                "target": receipt.target,
                "text_length": text_length,
                "delivery": "sent",
            }),
        })?;

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tool": "sessions_send",
                "session_id": session_id,
                "channel": receipt.channel,
                "target": receipt.target,
                "text_length": text_length,
                "delivery": "sent",
            }),
        })
    }
}

fn required_payload_text(payload: &Value) -> Result<String, String> {
    payload
        .get("text")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "sessions_send requires payload.text".to_owned())
}
