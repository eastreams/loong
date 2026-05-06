use super::*;

#[test]
fn build_turn_reply_followup_messages_include_truncation_hint_for_truncated_tool_results() {
    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#.to_owned(),
        },
        "summarize note.md",
    );

    let user_prompt = messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Original request:\nsummarize note.md"));
}

#[test]
fn build_turn_reply_followup_messages_do_not_include_truncation_hint_for_failure() {
    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
            retryable: false,
        },
        "summarize note.md",
    );

    let user_prompt = messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(!user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT));
}

#[test]
fn build_turn_reply_followup_messages_promotes_external_skill_invoke_to_system_context() {
    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"status":"ok","tool":"read","tool_call_id":"call-1","payload_semantics":"skill_context","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
        },
        "summarize note.md",
    );

    assert!(
        messages.iter().any(|message| message.get("role")
            == Some(&Value::String("system".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content
                    .contains("Follow the managed skill instruction before answering."))
                .unwrap_or(false)),
        "safe-lane followup should promote invoked external skill instructions into system context: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .filter(|message| message.get("role") == Some(&Value::String("assistant".to_owned())))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .all(|content| !content.contains("[tool_result]\n[ok]")),
        "safe-lane followup should not carry invoke payload forward as an ordinary assistant tool_result: {messages:?}"
    );
}

#[test]
fn build_turn_reply_followup_messages_rejects_truncated_external_skill_invoke_payload() {
    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"status":"ok","tool":"read","tool_call_id":"call-1","payload_semantics":"skill_context","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":true}"#.to_owned(),
        },
        "summarize note.md",
    );

    assert!(
        !messages.iter().any(|message| message.get("role")
            == Some(&Value::String("system".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content
                    .contains("Follow the managed skill instruction before answering."))
                .unwrap_or(false)),
        "truncated invoke payload must not activate managed skill system context: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .filter(|message| message.get("role") == Some(&Value::String("assistant".to_owned())))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .any(|content| content.contains("[tool_result]\n[ok]")),
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

    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "summarize README.md",
    );

    let assistant_tool_result = messages
        .iter()
        .find(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("assistant tool_result followup message should exist");
    let line = assistant_tool_result
        .lines()
        .nth(1)
        .expect("assistant tool_result should keep payload line");
    let envelope: Value = serde_json::from_str(
        line.strip_prefix("[ok] ")
            .expect("tool result line should preserve status prefix"),
    )
    .expect("reduced followup envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("file.read payload summary should stay valid json");

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

    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult { text: tool_result },
        "summarize the test run",
    );

    let (envelope, summary) =
        crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

    assert_eq!(envelope["tool"], "bash");
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
fn build_turn_reply_followup_messages_only_auto_adds_contract_for_retryable_failures() {
    let failure_messages = build_turn_reply_followup_messages_with_warning(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolFailure {
            reason: "missing_tool_call_followup: emit the next tool call now".to_owned(),
            retryable: true,
        },
        None,
        "summarize the repository",
        None,
    );
    let failure_prompt = failure_messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("retryable failure prompt should exist");
    assert!(
        failure_prompt.contains("Structured continuation contract:"),
        "retryable failure followup should carry a continuation contract: {failure_prompt:?}"
    );
    assert!(
        failure_prompt.contains("retryable"),
        "retryable failure followup should keep retry guidance: {failure_prompt:?}"
    );

    let tool_result_messages = build_turn_reply_followup_messages_with_warning(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
        ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"status":"ok","tool":"web","tool_call_id":"call-web","payload_summary":"{\"continuation\":{\"state\":\"insufficient_page_evidence\",\"is_terminal\":false,\"recommended_tool\":\"web\",\"recommended_payload\":{\"url\":\"https://example.com\"}}}","payload_chars":128,"payload_truncated":false}"#.to_owned(),
        },
        None,
        "summarize the page",
        None,
    );
    let tool_result_prompt = tool_result_messages
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("tool result prompt should exist");
    assert!(
        !tool_result_prompt.contains("Structured continuation contract:"),
        "tool-result continuation contract should stay lane-owned, not auto-added by the generic followup builder: {tool_result_prompt:?}"
    );
    assert!(
        tool_result_prompt.contains("Continuation guidance:"),
        "tool-result followup should still surface continuation guidance from the payload: {tool_result_prompt:?}"
    );
}
