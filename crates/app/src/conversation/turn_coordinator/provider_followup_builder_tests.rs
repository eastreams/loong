use super::*;

fn system_messages() -> [Value; 1] {
    [serde_json::json!({
        "role": "system",
        "content": "sys"
    })]
}

fn build_test_followup_messages(
    payload: ToolDrivenFollowupPayload,
    original_request: &str,
) -> Vec<Value> {
    let system_messages = system_messages();
    build_turn_reply_followup_messages(&system_messages, "preface", payload, original_request)
}

fn user_followup_prompt(messages: &[Value]) -> &str {
    messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist")
}

fn has_role_content(messages: &[Value], role: &str, content_needle: &str) -> bool {
    let expected_role = Value::String(role.to_owned());

    messages.iter().any(|message| {
        let message_role = message.get("role");
        let role_matches = message_role == Some(&expected_role);
        let message_content = message.get("content").and_then(Value::as_str);
        let content_matches =
            message_content.is_some_and(|content| content.contains(content_needle));

        role_matches && content_matches
    })
}

fn assistant_tool_result_content(messages: &[Value]) -> Option<&str> {
    let assistant_role = Value::String("assistant".to_owned());

    messages
        .iter()
        .filter(|message| message.get("role") == Some(&assistant_role))
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find(|content| content.contains("[tool_result]\n[ok]"))
}

#[test]
fn build_turn_reply_followup_messages_include_truncation_hint_for_truncated_tool_results() {
    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#.to_owned(),
        },
        "summarize note.md",
    );

    let user_prompt = user_followup_prompt(&messages);
    assert!(user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Original request:\nsummarize note.md"));
}

#[test]
fn build_turn_reply_followup_messages_applies_payload_budget() {
    let payload_summary = serde_json::json!({
        "content": "x".repeat(512),
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-file",
            "payload_summary": payload_summary,
            "payload_chars": 512,
            "payload_truncated": false
        })
    );
    let mut budget = FollowupPayloadBudget::new(64, 64);
    let first_messages = build_turn_reply_followup_messages_with_warning_and_budget(
        &system_messages(),
        "preface",
        ToolDrivenFollowupPayload::ToolResult {
            text: tool_result.clone(),
        },
        None,
        "summarize note.md",
        None,
        &mut budget,
    );
    let second_messages = build_turn_reply_followup_messages_with_warning_and_budget(
        &system_messages(),
        "preface",
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        None,
        "summarize note.md",
        None,
        &mut budget,
    );
    let first_tool_result = first_messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find(|content| content.starts_with("[tool_result]\n"))
        .expect("first followup should include a tool result");
    let second_tool_result = second_messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find(|content| content.starts_with("[tool_result]\n"))
        .expect("second followup should include a tool result");

    assert!(first_tool_result.contains("[tool_result_truncated]"));
    assert!(second_tool_result.contains("budget_exhausted=true"));
}

#[test]
fn build_turn_reply_followup_messages_do_not_include_truncation_hint_for_failure() {
    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
            retryable: false,
        },
        "summarize note.md",
    );

    let user_prompt = user_followup_prompt(&messages);
    assert!(!user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT));
}

#[test]
fn build_turn_reply_followup_messages_promotes_external_skill_invoke_to_system_context() {
    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
        },
        "summarize note.md",
    );

    assert!(
        has_role_content(
            &messages,
            "system",
            "Follow the managed skill instruction before answering.",
        ),
        "safe-lane followup should promote invoked external skill instructions into system context: {messages:?}"
    );
    assert!(
        assistant_tool_result_content(&messages).is_none(),
        "safe-lane followup should not carry invoke payload forward as an ordinary assistant tool_result: {messages:?}"
    );
}

#[test]
fn build_turn_reply_followup_messages_rejects_truncated_external_skill_invoke_payload() {
    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":true}"#.to_owned(),
        },
        "summarize note.md",
    );

    assert!(
        !has_role_content(
            &messages,
            "system",
            "Follow the managed skill instruction before answering.",
        ),
        "truncated invoke payload must not activate managed skill system context: {messages:?}"
    );
    assert!(
        assistant_tool_result_content(&messages).is_some(),
        "truncated invoke payload should stay as ordinary assistant tool_result content: {messages:?}"
    );
}

#[test]
fn build_turn_reply_followup_messages_reduces_file_read_payload_summary() {
    let content = (0..96)
        .map(|index| format!("line {index}: {}", "x".repeat(48)))
        .collect::<Vec<_>>()
        .join("\n");
    let payload_summary = serde_json::json!({
        "adapter": "core-tools",
        "tool_name": "file.read",
        "path": "/repo/README.md",
        "bytes": 8_192,
        "truncated": false,
        "content": content,
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-file",
            "payload_summary": payload_summary,
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "summarize README.md",
    );

    let (envelope, summary) =
        crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

    assert_eq!(envelope["tool"], "read");
    assert_eq!(envelope["payload_truncated"], true);
    assert_eq!(summary["path"], "/repo/README.md");
    assert_eq!(summary["bytes"], 8_192);
    assert_eq!(summary["truncated"], false);
    assert!(summary.get("content_preview").is_some());
    assert!(summary.get("content_chars").is_some());
    assert_eq!(summary["content_truncated"], true);
}

#[test]
fn build_turn_reply_followup_messages_reduces_shell_exec_payload_summary() {
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "shell.exec",
            "tool_call_id": "call-shell",
            "payload_summary": serde_json::json!({
                "adapter": "core-tools",
                "tool_name": "shell.exec",
                "command": "cargo",
                "args": ["test", "--workspace"],
                "cwd": "/repo",
                "exit_code": 0,
                "stdout": (0..80)
                    .map(|index| format!("stdout line {index}: {}", "x".repeat(40)))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "stderr": (0..48)
                    .map(|index| format!("stderr line {index}: {}", "e".repeat(32)))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .to_string(),
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "summarize the test run",
    );

    let (envelope, summary) =
        crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

    assert_eq!(envelope["tool"], "exec");
    assert_eq!(envelope["payload_truncated"], true);
    assert_eq!(summary["command"], "cargo");
    assert_eq!(summary["exit_code"], 0);
    assert!(summary.get("stdout_preview").is_some());
    assert!(summary.get("stdout_chars").is_some());
    assert_eq!(summary["stdout_truncated"], true);
    assert!(summary.get("stderr_preview").is_some());
    assert!(summary.get("stderr_chars").is_some());
    assert_eq!(summary["stderr_truncated"], true);
    assert!(
        summary["stdout_preview"]
            .as_str()
            .expect("stdout preview should exist")
            .contains("stdout line 0"),
        "expected compact stdout preview, got: {summary:?}"
    );
    assert!(
        summary["stderr_preview"]
            .as_str()
            .expect("stderr preview should exist")
            .contains("stderr line 0"),
        "expected compact stderr preview, got: {summary:?}"
    );
}

#[test]
fn build_turn_reply_followup_messages_compacts_tool_search_payload_summary() {
    let payload_summary = serde_json::json!({
        "adapter": "core-tools",
        "tool_name": "tool.search",
        "query": "read repo file",
        "returned": 2,
        "results": [
            {
                "tool_id": "file.read",
                "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                "argument_hint": "path:string,offset?:integer,limit?:integer",
                "required_fields": ["path"],
                "required_field_groups": [["path"]],
                "tags": ["core", "file", "read"],
                "why": ["summary matches query", "tag matches read"],
                "lease": "lease-file"
            },
            {
                "tool_id": "shell.exec",
                "summary": "Execute a shell command in the workspace.",
                "argument_hint": "command:string,args?:string[]",
                "required_fields": ["command"],
                "required_field_groups": [["command"]],
                "tags": ["core", "shell", "exec"],
                "why": ["summary matches query", "tag matches exec"],
                "lease": "lease-shell"
            }
        ]
    });
    let payload_summary_str = payload_summary.to_string();
    let tool_result = format!(
        "[ok] {}",
        serde_json::json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_chars": 2_048,
            "payload_summary": payload_summary_str,
            "payload_truncated": false
        })
    );

    let messages = build_test_followup_messages(
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "find the right tool",
    );

    let (envelope, summary) =
        crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);
    let summary_str = envelope["payload_summary"]
        .as_str()
        .expect("payload summary should stay encoded json");
    let results = summary["results"]
        .as_array()
        .expect("results should be an array");
    let first = &results[0];

    assert_eq!(envelope["tool"], "tool.search");
    assert_eq!(envelope["payload_truncated"], false);
    assert_ne!(summary_str, payload_summary.to_string());
    assert_eq!(summary["query"], "read repo file");
    assert!(summary.get("adapter").is_none());
    assert!(summary.get("tool_name").is_none());
    assert_eq!(summary["returned"], 2);
    assert_eq!(results.len(), 2);
    assert_eq!(first["tool_id"], "file.read");
    assert_eq!(first["lease"], "lease-file");
    for entry in results {
        assert!(entry.get("tool_id").and_then(Value::as_str).is_some());
        assert!(entry.get("summary").and_then(Value::as_str).is_some());
        assert!(entry.get("argument_hint").and_then(Value::as_str).is_some());
        assert!(
            entry
                .get("required_fields")
                .and_then(Value::as_array)
                .is_some()
        );
        assert!(
            entry
                .get("required_field_groups")
                .and_then(Value::as_array)
                .is_some()
        );
        assert!(entry.get("lease").and_then(Value::as_str).is_some());
        assert!(entry.get("tags").is_none());
        assert!(entry.get("why").is_none());
    }
}
