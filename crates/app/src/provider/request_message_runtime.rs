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
    build_system_message_with_tool_runtime_config(
        config,
        include_system_prompt,
        tools::runtime_config::get_tool_runtime_config(),
    )
}

fn build_system_message_with_tool_runtime_config(
    config: &LoongClawConfig,
    include_system_prompt: bool,
    tool_runtime_config: &tools::runtime_config::ToolRuntimeConfig,
) -> Option<Value> {
    if !include_system_prompt {
        return None;
    }
    let system_prompt = config.cli.resolved_system_prompt();
    let system = system_prompt.trim();
    let snapshot = tools::capability_snapshot_with_config(tool_runtime_config);
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
    messages.extend(load_memory_window_messages(config, session_id)?);
    Ok(messages)
}

pub(super) fn load_memory_window_messages(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<Vec<Value>> {
    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let hydrated = memory::hydrate_memory_context(session_id, &mem_config)
            .map_err(|error| format!("hydrate prompt memory context failed: {error}"))?;
        let mut messages = Vec::with_capacity(hydrated.entries.len());
        append_hydrated_memory_messages(&mut messages, &hydrated);
        Ok(messages)
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, session_id);
        Ok(Vec::new())
    }
}

#[cfg(feature = "memory-sqlite")]
fn append_hydrated_memory_messages(
    messages: &mut Vec<Value>,
    hydrated: &memory::HydratedMemoryContext,
) {
    for entry in &hydrated.entries {
        match entry.kind {
            memory::MemoryContextKind::Profile | memory::MemoryContextKind::Summary => {
                messages.push(json!({
                    "role": entry.role,
                    "content": entry.content,
                }));
            }
            memory::MemoryContextKind::Turn => {
                push_history_message(messages, entry.role.as_str(), entry.content.as_str());
            }
        }
    }
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
    use crate::config::MemoryProfile;

    #[test]
    fn build_system_message_returns_none_when_disabled() {
        let config = LoongClawConfig::default();
        assert_eq!(build_system_message(&config, false), None);
    }

    #[test]
    fn build_system_message_includes_custom_prompt_and_capability_snapshot() {
        let mut config = LoongClawConfig::default();
        config.cli.prompt_pack_id = None;
        config.cli.personality = None;
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

    #[test]
    fn message_builder_uses_rendered_prompt_from_pack_metadata() {
        let mut config = LoongClawConfig::default();
        config.cli.personality = Some(crate::prompt::PromptPersonality::FriendlyCollab);
        config.cli.system_prompt = String::new();
        let session_id = format!(
            "provider-rendered-prompt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        config.memory.sqlite_path = std::env::temp_dir()
            .join(format!("{session_id}.sqlite3"))
            .display()
            .to_string();

        let messages =
            build_messages_for_session(&config, &session_id, true).expect("build messages");
        let system_content = messages[0]["content"].as_str().expect("system content");

        assert!(system_content.contains("## Personality Overlay: Friendly Collaboration"));
        assert!(system_content.contains("[available_tools]"));

        let _ = std::fs::remove_file(config.memory.sqlite_path.as_str());
    }

    #[test]
    fn message_builder_keeps_legacy_inline_prompt_when_pack_is_disabled() {
        let mut config = LoongClawConfig::default();
        config.cli.prompt_pack_id = None;
        config.cli.personality = None;
        config.cli.system_prompt = "You are a legacy inline prompt.".to_owned();

        let system = build_system_message(&config, true).expect("system message");
        let system_content = system["content"].as_str().expect("system content");

        assert!(system_content.contains("You are a legacy inline prompt."));
        assert!(!system_content.contains("## Personality Overlay: Calm Engineering"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn message_builder_includes_summary_block_for_window_plus_summary_profile() {
        let tmp =
            std::env::temp_dir().join(format!("loongclaw-provider-summary-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("provider-summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = LoongClawConfig::default();
        config.memory.sqlite_path = db_path.display().to_string();
        config.memory.profile = MemoryProfile::WindowPlusSummary;
        config.memory.sliding_window = 2;

        let memory_config =
            memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        memory::append_turn_direct("summary-session", "user", "turn 1", &memory_config)
            .expect("append turn 1 should succeed");
        memory::append_turn_direct("summary-session", "assistant", "turn 2", &memory_config)
            .expect("append turn 2 should succeed");
        memory::append_turn_direct("summary-session", "user", "turn 3", &memory_config)
            .expect("append turn 3 should succeed");
        memory::append_turn_direct("summary-session", "assistant", "turn 4", &memory_config)
            .expect("append turn 4 should succeed");

        let messages =
            build_messages_for_session(&config, "summary-session", true).expect("build messages");

        assert!(
            messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("## Memory Summary"))
            }),
            "expected a system summary block in provider messages"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }
}
