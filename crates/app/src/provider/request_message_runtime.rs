use serde_json::{Value, json};

use crate::CliResult;
use crate::config::LoongClawConfig;
use crate::runtime_identity;
use crate::runtime_self;
use crate::tools::{self, ToolView};

#[cfg(feature = "memory-sqlite")]
use crate::memory;

pub(super) fn build_system_message(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Option<Value> {
    build_system_message_for_view(config, include_system_prompt, &tools::runtime_tool_view())
}

pub(super) fn build_system_message_for_view(
    config: &LoongClawConfig,
    include_system_prompt: bool,
    tool_view: &ToolView,
) -> Option<Value> {
    build_system_message_with_tool_runtime_config(
        config,
        include_system_prompt,
        tool_view,
        &tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None),
    )
}

fn build_system_message_with_tool_runtime_config(
    config: &LoongClawConfig,
    include_system_prompt: bool,
    tool_view: &ToolView,
    tool_runtime_config: &tools::runtime_config::ToolRuntimeConfig,
) -> Option<Value> {
    if !include_system_prompt {
        return None;
    }

    let system_prompt = config.cli.resolved_system_prompt();
    let system = system_prompt.trim();
    let snapshot = tools::capability_snapshot_for_view_with_config(tool_view, tool_runtime_config);
    let workspace_root = tool_runtime_config.file_root.as_deref();
    let runtime_self_model = workspace_root.map(runtime_self::load_runtime_self_model);
    let runtime_self_section = runtime_self_model
        .as_ref()
        .and_then(runtime_self::render_runtime_self_section);
    let trimmed_profile_note = config.memory.trimmed_profile_note();
    let resolved_runtime_identity = runtime_identity::resolve_runtime_identity(
        runtime_self_model.as_ref(),
        trimmed_profile_note.as_deref(),
    );
    let runtime_identity_section = resolved_runtime_identity
        .as_ref()
        .map(runtime_identity::render_runtime_identity_section);

    let mut sections = Vec::new();
    if !system.is_empty() {
        sections.push(system.to_owned());
    }
    if let Some(section) = runtime_self_section {
        sections.push(section);
    }
    if let Some(section) = runtime_identity_section {
        sections.push(section);
    }
    sections.push(snapshot);

    let content = sections.join("\n\n");
    Some(json!({
        "role": "system",
        "content": content,
    }))
}

pub(super) fn build_base_messages(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Vec<Value> {
    build_base_messages_for_view(config, include_system_prompt, &tools::runtime_tool_view())
}

pub(super) fn build_base_messages_for_view(
    config: &LoongClawConfig,
    include_system_prompt: bool,
    tool_view: &ToolView,
) -> Vec<Value> {
    build_system_message_for_view(config, include_system_prompt, tool_view)
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
    build_messages_for_session_in_view(
        config,
        session_id,
        include_system_prompt,
        &tools::runtime_tool_view(),
    )
}

pub(super) fn build_messages_for_session_in_view(
    config: &LoongClawConfig,
    session_id: &str,
    include_system_prompt: bool,
    tool_view: &ToolView,
) -> CliResult<Vec<Value>> {
    let mut messages = build_base_messages_for_view(config, include_system_prompt, tool_view);
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
    use tempfile::tempdir;

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
        assert!(content.contains("[tool_discovery_runtime]"));
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
        assert!(system_content.contains("[tool_discovery_runtime]"));

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

    #[test]
    fn build_system_message_includes_normalized_runtime_self_sections_from_workspace_root() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();

        let agents_path = workspace_root.join("AGENTS.md");
        let soul_path = workspace_root.join("SOUL.md");
        let identity_path = workspace_root.join("IDENTITY.md");
        let user_path = workspace_root.join("USER.md");

        let agents_text = "Always keep workspace instructions explicit.";
        let soul_text = "Prefer calm, rigorous, low-drama execution.";
        let identity_text = "You are the migration-shaped helper identity.";
        let user_text = "The operator prefers concise technical summaries.";

        std::fs::write(&agents_path, agents_text).expect("write AGENTS");
        std::fs::write(&soul_path, soul_text).expect("write SOUL");
        std::fs::write(&identity_path, identity_text).expect("write IDENTITY");
        std::fs::write(&user_path, user_text).expect("write USER");

        let config = LoongClawConfig::default();
        let tool_view = tools::runtime_tool_view();

        let tool_runtime_config = tools::runtime_config::ToolRuntimeConfig {
            file_root: Some(workspace_root.to_path_buf()),
            ..tools::runtime_config::ToolRuntimeConfig::default()
        };

        let system_message = build_system_message_with_tool_runtime_config(
            &config,
            true,
            &tool_view,
            &tool_runtime_config,
        )
        .expect("system message");
        let system_content = system_message["content"].as_str().expect("system content");

        assert!(system_content.contains("## Runtime Self Context"));
        assert!(system_content.contains("### Standing Instructions"));
        assert!(system_content.contains(agents_text));
        assert!(system_content.contains("### Soul Guidance"));
        assert!(system_content.contains(soul_text));
        assert!(system_content.contains("### User Context"));
        assert!(system_content.contains(user_text));
        assert!(system_content.contains("## Resolved Runtime Identity"));
        assert!(system_content.contains(identity_text));
        assert!(!system_content.contains("### Identity Context"));
    }

    #[test]
    fn build_system_message_promotes_legacy_imported_identity_when_workspace_identity_is_absent() {
        let mut config = LoongClawConfig::default();
        let legacy_profile_note =
            "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot";
        config.memory.profile_note = Some(legacy_profile_note.to_owned());

        let tool_view = tools::runtime_tool_view();
        let tool_runtime_config = tools::runtime_config::ToolRuntimeConfig::default();

        let system_message = build_system_message_with_tool_runtime_config(
            &config,
            true,
            &tool_view,
            &tool_runtime_config,
        )
        .expect("system message");
        let system_content = system_message["content"].as_str().expect("system content");

        assert!(system_content.contains("## Resolved Runtime Identity"));
        assert!(system_content.contains("Legacy build copilot"));
    }

    #[test]
    fn build_system_message_prefers_workspace_identity_over_legacy_profile_note_identity() {
        let temp_dir = tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let identity_path = workspace_root.join("IDENTITY.md");
        let workspace_identity = "# Identity\n\n- Name: Workspace build copilot";
        std::fs::write(&identity_path, workspace_identity).expect("write IDENTITY");

        let mut config = LoongClawConfig::default();
        let legacy_profile_note =
            "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot";
        config.memory.profile_note = Some(legacy_profile_note.to_owned());

        let tool_view = tools::runtime_tool_view();
        let tool_runtime_config = tools::runtime_config::ToolRuntimeConfig {
            file_root: Some(workspace_root.to_path_buf()),
            ..tools::runtime_config::ToolRuntimeConfig::default()
        };

        let system_message = build_system_message_with_tool_runtime_config(
            &config,
            true,
            &tool_view,
            &tool_runtime_config,
        )
        .expect("system message");
        let system_content = system_message["content"].as_str().expect("system content");

        assert!(system_content.contains("## Resolved Runtime Identity"));
        assert!(system_content.contains("Workspace build copilot"));
        assert!(!system_content.contains("Legacy build copilot"));
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
