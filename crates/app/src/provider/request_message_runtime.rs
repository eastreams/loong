use serde_json::{Value, json};

use crate::CliResult;
use crate::config::LoongClawConfig;
use crate::tools;

#[cfg(feature = "memory-sqlite")]
use crate::memory;

pub(super) fn build_system_message(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Option<Value> {
    if !include_system_prompt {
        return None;
    }
    let system = config.cli.system_prompt.trim();
    let snapshot = tools::capability_snapshot();
    let content = if system.is_empty() {
        snapshot
    } else {
        format!("{system}\n\n{snapshot}")
    };
    Some(json!({
        "role": "system",
        "content": content,
    }))
}

pub(super) fn build_base_messages(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Vec<Value> {
    build_system_message(config, include_system_prompt)
        .into_iter()
        .collect()
}

pub(super) fn push_history_message(messages: &mut Vec<Value>, role: &str, content: &str) {
    if !is_supported_chat_role(role) {
        return;
    }
    if should_skip_history_turn(role, content) {
        return;
    }
    messages.push(json!({
        "role": role,
        "content": content,
    }));
}

pub(super) fn build_messages_for_session(
    config: &LoongClawConfig,
    session_id: &str,
    include_system_prompt: bool,
) -> CliResult<Vec<Value>> {
    let mut messages = build_base_messages(config, include_system_prompt);

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let turns = memory::window_direct(session_id, config.memory.sliding_window, &mem_config)
            .map_err(|error| format!("load memory window failed: {error}"))?;
        for turn in turns {
            push_history_message(&mut messages, turn.role.as_str(), turn.content.as_str());
        }
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = session_id;
    }
    Ok(messages)
}

fn is_supported_chat_role(role: &str) -> bool {
    matches!(role, "system" | "user" | "assistant" | "tool")
}

fn should_skip_history_turn(role: &str, content: &str) -> bool {
    if role != "assistant" {
        return false;
    }
    let parsed = match serde_json::from_str::<Value>(content) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let event_type = parsed
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    matches!(
        event_type,
        "conversation_event" | "tool_decision" | "tool_outcome"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_system_message_returns_none_when_disabled() {
        let config = LoongClawConfig::default();
        assert_eq!(build_system_message(&config, false), None);
    }

    #[test]
    fn build_system_message_includes_custom_prompt_and_capability_snapshot() {
        let mut config = LoongClawConfig::default();
        config.cli.system_prompt = "Stay concise and technical.".to_owned();

        let system = build_system_message(&config, true).expect("system message");
        let content = system["content"].as_str().expect("system content");
        assert!(content.starts_with("Stay concise and technical."));
        assert!(content.contains("[available_tools]"));
    }

    #[test]
    fn push_history_message_skips_unsupported_roles() {
        let mut messages = Vec::new();
        push_history_message(&mut messages, "planner", "hello");
        assert!(messages.is_empty());
    }

    #[test]
    fn push_history_message_skips_internal_assistant_events() {
        let mut messages = Vec::new();
        let payload = serde_json::to_string(&json!({
            "type": "tool_outcome",
            "ok": true
        }))
        .expect("serialize");
        push_history_message(&mut messages, "assistant", payload.as_str());
        assert!(messages.is_empty());
    }

    #[test]
    fn push_history_message_keeps_normal_assistant_replies() {
        let mut messages = Vec::new();
        push_history_message(&mut messages, "assistant", "plain assistant reply");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"], "plain assistant reply");
    }
}
