use serde_json::json;

use super::*;

fn discovery_followup_messages(tool_id: &str, lease: &str) -> Vec<Value> {
    let payload_summary = serde_json::to_string(&json!({
        "results": [
            {
                "tool_id": tool_id,
                "lease": lease,
            }
        ]
    }))
    .expect("encode search payload summary");
    let envelope = serde_json::to_string(&json!({
        "status": "ok",
        "tool": "tool.search",
        "tool_call_id": "call-search",
        "payload_summary": payload_summary,
        "payload_chars": payload_summary.chars().count(),
        "payload_truncated": false,
    }))
    .expect("encode search envelope");
    vec![json!({
        "role": "assistant",
        "content": format!("[tool_result]\n[ok] {envelope}"),
    })]
}

fn discovery_followup_part_messages(tool_id: &str, lease: &str) -> Vec<Value> {
    let payload_summary = serde_json::to_string(&json!({
        "results": [
            {
                "tool_id": tool_id,
                "lease": lease,
            }
        ]
    }))
    .expect("encode search payload summary");
    let envelope = serde_json::to_string(&json!({
        "status": "ok",
        "tool": "tool.search",
        "tool_call_id": "call-search",
        "payload_summary": payload_summary,
        "payload_chars": payload_summary.chars().count(),
        "payload_truncated": false,
    }))
    .expect("encode search envelope");
    vec![json!({
        "role": "assistant",
        "content": [{
            "type": "input_text",
            "text": format!("[tool_result]\n[ok] {envelope}"),
        }],
    })]
}

#[test]
fn extract_provider_turn_parses_tool_calls() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "checking",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "file.read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
    assert_eq!(turn.tool_intents[0].tool_call_id, "call_1");
}

#[test]
fn extract_provider_turn_surfaces_malformed_json_args() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "calling",
                "tool_calls": [{
                    "id": "call_bad",
                    "type": "function",
                    "function": {
                        "name": "file.read",
                        "arguments": "{{not valid json"
                    }
                }]
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.tool_intents.len(), 1);
    let args = &turn.tool_intents[0].args_json;
    assert!(
        args.get("_parse_error").is_some(),
        "malformed args should surface parse error, got: {args}"
    );
    assert_eq!(
        args.get("_raw_arguments").and_then(|v| v.as_str()),
        Some("{{not valid json")
    );
}

#[test]
fn extract_provider_turn_normalizes_underscore_tool_aliases() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "calling",
                "tool_calls": [{
                    "id": "call_underscore",
                    "type": "function",
                    "function": {
                        "name": "file_read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
}

#[test]
fn extract_provider_turn_with_scope_prefers_direct_surface_for_direct_tools_after_search() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "checking",
                "tool_calls": [{
                    "id": "call_compat",
                    "type": "function",
                    "function": {
                        "name": "file.read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            }
        }]
    });
    let messages = discovery_followup_messages("read", "lease-openai");

    let turn = extract_provider_turn_with_scope_and_messages(
        &body,
        Some("session-shape"),
        Some("turn-shape"),
        &messages,
    )
    .expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].session_id, "session-shape");
    assert_eq!(turn.tool_intents[0].turn_id, "turn-shape");
    assert_eq!(turn.tool_intents[0].tool_call_id, "call_compat");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
}

#[cfg(feature = "feishu-integration")]
#[test]
fn extract_provider_turn_with_scope_ignores_runtime_discovered_feishu_hidden_tools() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "updating card",
                "tool_calls": [{
                    "id": "call_feishu_card_update_1",
                    "type": "function",
                    "function": {
                        "name": "feishu_card_update",
                        "arguments": "{\"markdown\":\"callback updated\"}"
                    }
                }]
            }
        }]
    });
    let messages = discovery_followup_messages("feishu.card.update", "lease-feishu");

    let turn = extract_provider_turn_with_scope_and_messages(
        &body,
        Some("session-feishu"),
        Some("turn-feishu"),
        &messages,
    )
    .expect("turn");
    assert_eq!(turn.assistant_text, "updating card");
    assert!(turn.tool_intents.is_empty());
}

#[test]
fn bridge_context_skips_truncated_search_results() {
    let payload_summary = serde_json::to_string(&json!({
        "results": [
            {
                "tool_id": "read",
                "lease": "lease-truncated",
            }
        ]
    }))
    .expect("encode");
    let envelope = serde_json::to_string(&json!({
        "status": "ok",
        "tool": "tool.search",
        "tool_call_id": "call-search",
        "payload_summary": payload_summary,
        "payload_chars": payload_summary.chars().count(),
        "payload_truncated": true,
    }))
    .expect("encode envelope");
    let messages = vec![json!({
        "role": "assistant",
        "content": format!("[tool_result]\n[ok] {envelope}"),
    })];

    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "reading",
                "tool_calls": [{
                    "id": "call_trunc",
                    "type": "function",
                    "function": {
                        "name": "file_read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            }
        }]
    });
    let turn = extract_provider_turn_with_scope_and_messages(
        &body,
        Some("session-trunc"),
        Some("turn-trunc"),
        &messages,
    )
    .expect("turn");
    // When payload is truncated, bridge context should be empty,
    // so the hidden alias should fall back to the direct visible surface instead of tool.invoke.
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
}

#[test]
fn extract_provider_turn_handles_text_only() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "hello world"
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "hello world");
    assert!(turn.tool_intents.is_empty());
}

#[test]
fn extract_provider_turn_supports_responses_function_calls() {
    let body = serde_json::json!({
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": "Reading the file."}
                ]
            },
            {
                "type": "function_call",
                "name": "file_read",
                "arguments": "{\"path\":\"README.md\"}",
                "call_id": "call_resp_1"
            }
        ]
    });
    let messages = discovery_followup_messages("read", "lease-responses");
    let turn = extract_provider_turn_with_scope(
        &body,
        Some("session-responses"),
        Some("turn-responses"),
    )
    .expect("responses turn without search context should stay direct");
    assert_eq!(turn.assistant_text, "Reading the file.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].session_id, "session-responses");
    assert_eq!(turn.tool_intents[0].turn_id, "turn-responses");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    assert_eq!(turn.tool_intents[0].tool_call_id, "call_resp_1");

    let turn = extract_provider_turn_with_scope_and_messages(
        &body,
        Some("session-responses"),
        Some("turn-responses"),
        &messages,
    )
    .expect("responses turn with search context");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
}

#[test]
fn extract_provider_turn_supports_responses_function_calls_with_array_followup_messages() {
    let body = serde_json::json!({
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": "Reading the file."}
                ]
            },
            {
                "type": "function_call",
                "name": "file_read",
                "arguments": "{\"path\":\"README.md\"}",
                "call_id": "call_resp_1"
            }
        ]
    });
    let messages = discovery_followup_part_messages("file.read", "lease-responses-parts");

    let turn = extract_provider_turn_with_scope_and_messages(
        &body,
        Some("session-responses"),
        Some("turn-responses"),
        &messages,
    )
    .expect("responses turn with array-form search context");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
}

#[test]
fn extract_provider_turn_parses_inline_shell_function_block() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "sorry, that command failed. let me retry with a simpler approach:\n<function=shell.exec><parameter=command>ls /root</parameter></function>"
            }
        }]
    });
    let messages = discovery_followup_messages("exec", "lease-shell-inline");

    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(
        turn.assistant_text,
        "sorry, that command failed. let me retry with a simpler approach:"
    );
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "bash");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({"command":"ls /root"})
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
        "parsed"
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["inline_function"]["tool_count"],
        1
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
        "parsed"
    );
}

#[test]
fn extract_provider_turn_parses_invoke_blocks_with_quoted_gt_in_arguments() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me run the shell command.\n<function_calls>\n<invoke name=\"shell.exec\" arguments=\"{&quot;command&quot;:&quot;sh&quot;,&quot;args&quot;:[&quot;-lc&quot;,&quot;echo hi > out.txt&quot;]}\"></invoke>\n</function_calls>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");

    assert_eq!(turn.assistant_text, "let me run the shell command.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "bash");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({
            "command": "sh",
            "args": ["-lc", "echo hi > out.txt"]
        })
    );
}

#[test]
fn extract_provider_turn_prefers_direct_surface_for_function_call_followups_after_search() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "now i'll read the file.\n<function_calls>\n<invoke name=\"file_read\" arguments=\"{&quot;path&quot;:&quot;note.md&quot;}\"></invoke>\n</function_calls>"
            }
        }]
    });
    let messages = discovery_followup_messages("read", "lease-invoke-followup");

    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(turn.assistant_text, "now i'll read the file.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "note.md"}));
}

#[test]
fn extract_provider_turn_prefers_direct_surface_for_plain_json_followups_after_search() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "now i'll read the file.\n{\n  \"name\": \"file_read\",\n  \"arguments\": {\n    \"path\": \"note.md\"\n  }\n}"
            }
        }]
    });
    let messages = discovery_followup_messages("read", "lease-json-followup");

    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(turn.assistant_text, "now i'll read the file.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "note.md"}));
}

#[test]
fn extract_provider_turn_accepts_legacy_request_wrapper_for_browse_followups() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "now i'll open the page.\n{\n  \"tool\": \"browser.open\",\n  \"request\": {\n    \"url\": \"https://example.com\"\n  }\n}"
            }
        }]
    });
    let messages = discovery_followup_messages("browse", "browse-wrapper-followup");

    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(turn.assistant_text, "now i'll open the page.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "browse");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({"url": "https://example.com"})
    );
}

#[test]
fn extract_provider_turn_repairs_misordered_browse_wrapper_after_search() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "open the page.\n{\"url\":\"https://example.com\"},\"tool\":\"browse.open\"}"
            }
        }]
    });
    let messages = discovery_followup_messages("browse", "browse-misordered-followup");

    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(turn.assistant_text, "open the page.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "browse");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({"url": "https://example.com"})
    );
}

#[test]
fn extract_provider_turn_recovers_glued_tool_request_markup_and_trailing_summary_text() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "[tool_request]\n{\"url\":\"https://example.com\"},\"name\":\"web\"}Example Domain is a short documentation example page."
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(
        turn.assistant_text,
        "Example Domain is a short documentation example page."
    );
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "web");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({"url": "https://example.com"})
    );
}

#[test]
fn extract_provider_turn_recovers_multiple_glued_tool_request_wrappers_before_final_text() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "[tool_request]\n{\"arguments\":{\"path\":\"AGENTS.md\"},\"name\":\"read\"}[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}I do not yet have the tool outputs needed to summarize the repository."
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(
        turn.assistant_text,
        "I do not yet have the tool outputs needed to summarize the repository."
    );
    assert_eq!(turn.tool_intents.len(), 2);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "AGENTS.md"}));
    assert_eq!(turn.tool_intents[1].tool_name, "read");
    assert_eq!(
        turn.tool_intents[1].args_json,
        json!({"path": "docs/README.md"})
    );
}

#[test]
fn extract_provider_turn_recovers_multiple_glued_tool_request_wrappers_without_final_text() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "[tool_request]\n{\"arguments\":{\"path\":\"README.md\"},\"name\":\"read\"}[tool_request]\n{\"arguments\":{\"path\":\"ARCHITECTURE.md\"},\"name\":\"read\"}[tool_request]\n{\"arguments\":{\"path\":\"docs/ROADMAP.md\"},\"name\":\"read\"}"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "");
    assert_eq!(turn.tool_intents.len(), 3);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    assert_eq!(turn.tool_intents[1].tool_name, "read");
    assert_eq!(
        turn.tool_intents[1].args_json,
        json!({"path": "ARCHITECTURE.md"})
    );
    assert_eq!(turn.tool_intents[2].tool_name, "read");
    assert_eq!(
        turn.tool_intents[2].args_json,
        json!({"path": "docs/ROADMAP.md"})
    );
}

#[test]
fn extract_provider_turn_strips_same_line_tool_request_wrapper_after_leading_preface() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "to summarize repo need inspect key docs.[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(
        turn.assistant_text,
        "to summarize repo need inspect key docs."
    );
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({"path": "docs/README.md"})
    );
}

#[test]
fn extract_provider_turn_recovers_tool_request_array_wrapper_with_trailing_text() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "[tool_request]\n[{\"arguments\":{\"path\":\"AGENTS.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}]This repository is a Rust workspace."
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "This repository is a Rust workspace.");
    assert_eq!(turn.tool_intents.len(), 2);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "AGENTS.md"}));
    assert_eq!(turn.tool_intents[1].tool_name, "read");
    assert_eq!(
        turn.tool_intents[1].args_json,
        json!({"path": "docs/README.md"})
    );
}

#[test]
fn extract_provider_turn_does_not_execute_plain_json_top_level_arguments_without_envelope() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n{\n  \"name\": \"tool_search\",\n  \"query\": \"read note.md\"\n}"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n{\n  \"name\": \"tool_search\",\n  \"query\": \"read note.md\"\n}"
    );
}

#[test]
fn extract_provider_turn_marks_invalid_stringified_json_tool_arguments_malformed() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me search for the right tool first.\n{\n  \"name\": \"tool_search\",\n  \"arguments\": \"{bad\"\n}"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "let me search for the right tool first.\n{\n  \"name\": \"tool_search\",\n  \"arguments\": \"{bad\"\n}"
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["json_tool_block"]["status"],
        "malformed"
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["json_tool_block"]["error_code"],
        "invalid_json"
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["json_tool_block"]["status"],
        "malformed"
    );
}

#[test]
fn extract_provider_turn_does_not_execute_nested_tool_like_plain_json_objects() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n{\n  \"meta\":\n  {\n    \"name\": \"tool_search\",\n    \"arguments\": {\n      \"query\": \"read note.md\"\n    }\n  }\n}"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n{\n  \"meta\":\n  {\n    \"name\": \"tool_search\",\n    \"arguments\": {\n      \"query\": \"read note.md\"\n    }\n  }\n}"
    );
}

#[test]
fn extract_provider_turn_does_not_execute_fenced_json_tool_examples() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n```json\n{\n  \"name\": \"tool_search\",\n  \"arguments\": {\n    \"query\": \"read note.md\"\n  }\n}\n```"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n```json\n{\n  \"name\": \"tool_search\",\n  \"arguments\": {\n    \"query\": \"read note.md\"\n  }\n}\n```"
    );
}

#[test]
fn extract_provider_turn_does_not_execute_literal_inline_function_examples() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "if you want to invoke it manually, you can write it like ` <function=shell.exec><parameter=command>ls</parameter></function> `."
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "if you want to invoke it manually, you can write it like ` <function=shell.exec><parameter=command>ls</parameter></function> `."
    );
}

#[test]
fn extract_provider_turn_does_not_execute_fenced_inline_function_examples() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n```xml\n<function=shell.exec><parameter=command>ls</parameter></function>\n```"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n```xml\n<function=shell.exec><parameter=command>ls</parameter></function>\n```"
    );
}

#[test]
fn extract_provider_turn_does_not_execute_indented_code_block_examples() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n\n    <function=shell.exec><parameter=command>ls</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n\n    <function=shell.exec><parameter=command>ls</parameter></function>"
    );
}

#[test]
fn extract_provider_turn_does_not_execute_multiline_indented_code_block_examples() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n\n    step one\n    <function=shell.exec><parameter=command>ls</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n\n    step one\n    <function=shell.exec><parameter=command>ls</parameter></function>"
    );
}

#[test]
fn extract_provider_turn_does_not_execute_tab_indented_code_block_examples() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "example:\n\n\t<function=shell.exec><parameter=command>ls</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.assistant_text,
        "example:\n\n\t<function=shell.exec><parameter=command>ls</parameter></function>"
    );
}

#[test]
fn extract_provider_turn_parses_indented_inline_function_when_not_code_block() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me retry:\n    <function=shell.exec><parameter=command>ls</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "let me retry:");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "bash");
    assert_eq!(turn.tool_intents[0].args_json, json!({"command": "ls"}));
}

#[test]
fn extract_provider_turn_parses_tab_indented_inline_function_when_not_code_block() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me retry:\n\t<function=shell.exec><parameter=command>ls</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "let me retry:");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "bash");
    assert_eq!(turn.tool_intents[0].args_json, json!({"command": "ls"}));
}

#[test]
fn extract_provider_turn_recovers_inline_parameter_json_types() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me retry with structured parameters.\n<function=shell.exec><parameter=command>\"echo\"</parameter><parameter=args>[\"hello\",\"world\"]</parameter><parameter=timeout_ms>3000</parameter><parameter=login>false</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({
            "command": "echo",
            "args": ["hello", "world"],
            "timeout_ms": 3000,
            "login": false
        })
    );
}

#[test]
fn extract_provider_turn_preserves_string_typed_inline_parameters() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me retry.\n<function=shell.exec><parameter=command>true</parameter><parameter=args>[\"hello\"]</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({
            "command": "true",
            "args": ["hello"]
        })
    );
}

#[test]
fn extract_provider_turn_records_malformed_inline_function_telemetry() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me retry.\n<function=shell.exec><parameter=command>ls /root</parameter>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(
        turn.assistant_text,
        "let me retry.\n<function=shell.exec><parameter=command>ls /root</parameter>"
    );
    assert!(turn.tool_intents.is_empty());
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
        "malformed"
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["inline_function"]["error_code"],
        "missing_function_close"
    );
    assert_eq!(
        turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
        "malformed"
    );
}

#[test]
fn extract_provider_turn_supports_array_content_shape() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": [
                    {"type": "text", "text": "line1"},
                    {"type": "text", "text": {"value": "line2"}}
                ]
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "line1\nline2");
    assert!(turn.tool_intents.is_empty());
}

#[test]
fn extract_provider_turn_preserves_reasoning_content_in_raw_meta() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "done",
                "reasoning_content": "thinking"
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "done");
    assert_eq!(turn.raw_meta["reasoning_content"], "thinking");
}

#[test]
fn extract_provider_turn_supports_anthropic_native_content_blocks() {
    let body = json!({
        "content": [
            {
                "type": "text",
                "text": "checking"
            },
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "file_read",
                "input": {
                    "path": "README.md"
                }
            }
        ]
    });
    let messages = discovery_followup_messages("read", "lease-anthropic");
    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].tool_call_id, "toolu_1");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
}

#[test]
fn extract_provider_turn_supports_bedrock_converse_content_blocks() {
    let body = json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [
                    {
                        "text": "checking"
                    },
                    {
                        "toolUse": {
                            "toolUseId": "toolu_1",
                            "name": "file_read",
                            "input": {
                                "path": "README.md"
                            }
                        }
                    }
                ]
            }
        },
        "stopReason": "tool_use"
    });
    let messages = discovery_followup_messages("read", "lease-bedrock");
    let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
        .expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].tool_call_id, "toolu_1");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    assert_eq!(turn.raw_meta["content"][1]["type"], "tool_use");
    assert_eq!(turn.raw_meta["content"][1]["id"], "toolu_1");
}

#[test]
fn extract_message_content_supports_part_array_shape() {
    let body = json!({
        "choices": [{
            "message": {
                "content": [
                    {"type": "text", "text": "line1"},
                    {"type": "text", "text": {"value": "line2"}}
                ]
            }
        }]
    });
    let content = extract_message_content(&body).expect("content");
    assert_eq!(content, "line1\nline2");
}

#[test]
fn extract_message_content_keeps_plain_string_shape() {
    let body = json!({
        "choices": [{
            "message": {
                "content": "  hello world  "
            }
        }]
    });
    let content = extract_message_content(&body).expect("content");
    assert_eq!(content, "hello world");
}

#[test]
fn extract_message_content_supports_responses_output_shape() {
    let body = json!({
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "output_text", "text": "line1"},
                {"type": "output_text", "text": {"value": "line2"}}
            ]
        }]
    });
    let content = extract_message_content(&body).expect("responses content");
    assert_eq!(content, "line1\nline2");
}

#[test]
fn extract_message_content_ignores_empty_parts() {
    let body = json!({
        "choices": [{
            "message": {
                "content": [
                    {"type": "text", "text": "   "},
                    {"type": "text", "text": {"value": ""}}
                ]
            }
        }]
    });
    assert!(extract_message_content(&body).is_none());
}
