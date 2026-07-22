use std::collections::BTreeMap;

use serde_json::json;

use super::*;

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
    assert_eq!(turn.tool_intents[0].tool_name(), "file.read");
    assert!(matches!(
        &turn.tool_intents[0].tool_name,
        ToolIntentTarget::Unresolved { .. }
    ));
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
}

#[test]
fn extract_provider_turn_with_scope_does_not_reinterpret_unregistered_file_paths() {
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
    let turn = extract_provider_turn_with_scope(&body, Some("turn-shape")).expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name(), "file.read");
    assert_eq!(turn.tool_intents[0].turn_id, "turn-shape");
    assert_eq!(turn.tool_intents[0].tool_call_id, "call_compat");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
}

#[test]
fn request_surface_resolves_provider_name_to_exact_registered_path() {
    let body = json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_typed",
                    "type": "function",
                    "function": { "name": "typed_only", "arguments": "{}" }
                }]
            }
        }]
    });
    let path =
        loong_contracts::ToolPath::new(["typed.only"]).expect("test tool path must be valid");
    let surface = ProviderToolRequestSurface::new(
        vec![json!({
            "type": "function",
            "function": { "name": "typed_only", "parameters": { "type": "object" } }
        })],
        BTreeMap::from([("typed_only".to_owned(), path.clone())]),
    )
    .expect("typed request surface");

    let turn = extract_provider_turn_for_request(&body, Some("turn-typed"), &surface)
        .expect("provider turn");

    assert_eq!(
        turn.tool_intents[0].tool_name.registered_path(),
        Some(&path)
    );
    assert_eq!(turn.tool_intents[0].tool_name(), "typed_only");
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
    let surface = ProviderToolRequestSurface::new(Vec::new(), BTreeMap::new())
        .expect("empty request surface");
    let turn =
        extract_provider_turn_for_request(&body, Some("turn-feishu"), &surface).expect("turn");
    assert_eq!(turn.assistant_text, "updating card");
    assert!(turn.tool_intents.is_empty());
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
    let turn = extract_provider_turn_with_scope(&body, Some("turn-responses"))
        .expect("responses turn without search context should stay direct");
    assert_eq!(turn.assistant_text, "Reading the file.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
    assert_eq!(turn.tool_intents[0].turn_id, "turn-responses");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    assert_eq!(turn.tool_intents[0].tool_call_id, "call_resp_1");
}

#[test]
fn extract_provider_turn_supports_responses_function_calls_with_array_content() {
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
    let turn = extract_provider_turn_with_scope(&body, Some("turn-responses"))
        .expect("responses turn with array-form content");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
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
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(
        turn.assistant_text,
        "sorry, that command failed. let me retry with a simpler approach:"
    );
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "bash");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "bash");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({
            "command": "sh",
            "args": ["-lc", "echo hi > out.txt"]
        })
    );
}

#[test]
fn extract_provider_turn_normalizes_direct_surface_in_function_call_blocks() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "now i'll read the file.\n<function_calls>\n<invoke name=\"file_read\" arguments=\"{&quot;path&quot;:&quot;note.md&quot;}\"></invoke>\n</function_calls>"
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "now i'll read the file.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "note.md"}));
}

#[test]
fn extract_provider_turn_normalizes_direct_surface_in_json_blocks() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "now i'll read the file.\n{\n  \"name\": \"file_read\",\n  \"arguments\": {\n    \"path\": \"note.md\"\n  }\n}"
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "now i'll read the file.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "note.md"}));
}

#[test]
fn extract_provider_turn_accepts_legacy_browse_request_wrapper() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "now i'll open the page.\n{\n  \"tool\": \"browser.open\",\n  \"request\": {\n    \"url\": \"https://example.com\"\n  }\n}"
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "now i'll open the page.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "browse");
    assert_eq!(
        turn.tool_intents[0].args_json,
        json!({"url": "https://example.com"})
    );
}

#[test]
fn extract_provider_turn_repairs_misordered_browse_wrapper() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "open the page.\n{\"url\":\"https://example.com\"},\"tool\":\"browse.open\"}"
            }
        }]
    });
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "open the page.");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "browse");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "web");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "AGENTS.md"}));
    assert_eq!(turn.tool_intents[1].tool_name.name(), "read");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    assert_eq!(turn.tool_intents[1].tool_name.name(), "read");
    assert_eq!(
        turn.tool_intents[1].args_json,
        json!({"path": "ARCHITECTURE.md"})
    );
    assert_eq!(turn.tool_intents[2].tool_name.name(), "read");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "read");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "AGENTS.md"}));
    assert_eq!(turn.tool_intents[1].tool_name.name(), "read");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "bash");
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
    assert_eq!(turn.tool_intents[0].tool_name.name(), "bash");
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

    let tool_definitions = vec![json!({
        "type": "function",
        "function": {
            "name": "shell.exec",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "args": {"type": "array", "items": {"type": "string"}}
                }
            }
        }
    })];
    let surface = ProviderToolRequestSurface::new(tool_definitions, BTreeMap::new())
        .expect("request surface");
    let turn = extract_provider_turn_for_request(&body, None, &surface).expect("turn");
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
fn extract_provider_turn_without_tool_definitions_does_not_use_catalog_schema() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "let me retry.\n<function=shell.exec><parameter=command>true</parameter></function>"
            }
        }]
    });

    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].args_json, json!({"command": true}));
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
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
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
    let turn = extract_provider_turn(&body).expect("turn");
    assert_eq!(turn.assistant_text, "checking");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name.name(), "file_read");
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
