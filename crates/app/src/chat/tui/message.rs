use std::time::Instant;

use serde_json::Value;

pub(super) fn format_tool_args_preview(tool_name: &str, raw_args: &str) -> String {
    let trimmed_args = raw_args.trim();

    if trimmed_args.is_empty() {
        return String::new();
    }

    let parsed_args = serde_json::from_str::<Value>(trimmed_args);

    let Ok(parsed_args) = parsed_args else {
        return trimmed_args.to_owned();
    };

    let formatted_args =
        crate::conversation::turn_engine::format_tool_arguments_preview(tool_name, &parsed_args);

    match formatted_args {
        Some(formatted_args) => formatted_args,
        None => trimmed_args.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    User,
    Assistant,
    System,
    Surface,
}

#[derive(Debug, Clone)]
pub(super) enum ToolStatus {
    Running {
        started: Instant,
    },
    Done {
        success: bool,
        output: String,
        duration_ms: u32,
    },
}

#[derive(Debug, Clone)]
pub(super) enum MessagePart {
    Text(String),
    ThinkBlock(String),
    ToolCall {
        tool_id: String,
        tool_name: String,
        args_preview: String,
        status: ToolStatus,
    },
}

#[derive(Debug, Clone)]
pub(super) struct Message {
    pub(super) role: Role,
    pub(super) parts: Vec<MessagePart>,
}

impl Message {
    pub(super) fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(text.into())],
        }
    }

    pub(super) fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            parts: vec![MessagePart::Text(text.into())],
        }
    }

    pub(super) fn surface(text: impl Into<String>) -> Self {
        Self {
            role: Role::Surface,
            parts: vec![MessagePart::Text(text.into())],
        }
    }

    pub(super) fn assistant() -> Self {
        Self {
            role: Role::Assistant,
            parts: Vec::new(),
        }
    }
}

impl ToolStatus {
    /// Truncates output to the first 80 characters for preview display.
    pub(super) fn preview_output(&self) -> Option<&str> {
        match self {
            Self::Running { .. } => None,
            Self::Done { output, .. } => {
                let end = output
                    .char_indices()
                    .nth(80)
                    .map_or(output.len(), |(i, _)| i);
                Some(output.get(..end).unwrap_or(output))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)]
mod tests {
    use super::*;

    #[test]
    fn user_message_creation() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.parts.len(), 1);
        match &msg.parts[0] {
            MessagePart::Text(t) => assert_eq!(t, "hello"),
            _ => panic!("expected Text part"),
        }
    }

    #[test]
    fn system_message_creation() {
        let msg = Message::system("system prompt");
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.parts.len(), 1);
    }

    #[test]
    fn surface_message_creation() {
        let msg = Message::surface("surface");
        assert_eq!(msg.role, Role::Surface);
        assert_eq!(msg.parts.len(), 1);
    }

    #[test]
    fn assistant_message_starts_empty() {
        let msg = Message::assistant();
        assert_eq!(msg.role, Role::Assistant);
        assert!(msg.parts.is_empty());
    }

    #[test]
    fn tool_call_lifecycle() {
        let status = ToolStatus::Running {
            started: Instant::now(),
        };
        assert!(status.preview_output().is_none());

        let status = ToolStatus::Done {
            success: true,
            output: "short output".to_string(),
            duration_ms: 42,
        };
        assert_eq!(status.preview_output(), Some("short output"));
    }

    #[test]
    fn tool_status_truncates_long_output() {
        let long = "a".repeat(200);
        let status = ToolStatus::Done {
            success: true,
            output: long,
            duration_ms: 10,
        };
        let preview = status.preview_output().unwrap_or("");
        assert_eq!(preview.len(), 80);
    }

    #[test]
    fn streaming_append_and_flush_via_parts() {
        let mut msg = Message::assistant();
        msg.parts.push(MessagePart::Text("streamed".to_string()));
        msg.parts
            .push(MessagePart::ThinkBlock("thought".to_string()));
        assert_eq!(msg.parts.len(), 2);
        match &msg.parts[0] {
            MessagePart::Text(t) => assert_eq!(t, "streamed"),
            _ => panic!("expected Text"),
        }
        match &msg.parts[1] {
            MessagePart::ThinkBlock(t) => assert_eq!(t, "thought"),
            _ => panic!("expected ThinkBlock"),
        }
    }

    #[test]
    fn tool_args_preview_summarizes_file_write_payload() {
        let raw_args = "{\"path\":\"src/main.rs\",\"content\":\"fn main() {}\\n\"}";
        let preview = format_tool_args_preview("file.write", raw_args);

        assert!(preview.contains("src/main.rs"));
        assert!(preview.contains("12 chars"));
    }

    #[test]
    fn tool_args_preview_summarizes_shell_exec_payload() {
        let raw_args = "{\"command\":\"git\",\"args\":[\"status\",\"--short\"]}";
        let preview = format_tool_args_preview("shell.exec", raw_args);

        assert_eq!(preview, "git status --short");
    }
}
