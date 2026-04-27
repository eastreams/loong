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
            text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
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
            text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":true}"#.to_owned(),
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
fn build_safe_lane_plan_graph_uses_precise_visible_names_for_grouped_hidden_invokes() {
    let config = LoongConfig::default();
    let lane_decision = LaneDecision {
        lane: ExecutionLane::Safe,
        risk_score: 0,
        complexity_score: 0,
        reasons: Vec::new(),
    };
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".to_owned(),
            args_json: json!({
                "tool_id": "agent",
                "lease": "lease-agent",
                "arguments": {
                    "operation": "delegate-background",
                    "task": "summarize the repo"
                }
            }),
            source: "provider_tool_call".to_owned(),
            session_id: "session-a".to_owned(),
            turn_id: "turn-a".to_owned(),
            tool_call_id: "call-agent".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    let plan = build_safe_lane_plan_graph(&config, &lane_decision, &turn, 2, 0);
    let tool_node = plan
        .nodes
        .iter()
        .find(|node| node.kind == PlanNodeKind::Tool)
        .expect("plan should include a tool node");

    assert_eq!(tool_node.label, "invoke `delegate_async`");
    assert_eq!(tool_node.tool_name.as_deref(), Some("delegate_async"));
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

    let messages = build_turn_reply_followup_messages(
        &[serde_json::json!({
            "role": "system",
            "content": "sys"
        })],
        "preface",
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
