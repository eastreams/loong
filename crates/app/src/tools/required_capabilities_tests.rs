use std::collections::BTreeSet;

use loong_contracts::{Capability, ToolCoreRequest};
use serde_json::json;

use super::{canonical_tool_name, required_capabilities_for_request};

#[test]
fn required_capabilities_follow_effective_tool_request() {
    let direct_file_read = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({"path": "README.md"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_file_read),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let direct_glob_search = ToolCoreRequest {
        tool_name: "glob.search".to_owned(),
        payload: json!({"pattern": "**/*.rs"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_glob_search),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let direct_content_search = ToolCoreRequest {
        tool_name: "content.search".to_owned(),
        payload: json!({"query": "Loong"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_content_search),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let direct_file_write = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({"path": "notes.txt", "content": "hello"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_file_write),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemWrite])
    );

    let direct_file_edit = ToolCoreRequest {
        tool_name: "file.edit".to_owned(),
        payload: json!({"path": "notes.txt", "old_string": "a", "new_string": "b"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_file_edit),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemWrite])
    );

    let direct_memory_search = ToolCoreRequest {
        tool_name: "memory_search".to_owned(),
        payload: json!({"query": "deploy freeze"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_memory_search),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let direct_memory_get = ToolCoreRequest {
        tool_name: "memory_get".to_owned(),
        payload: json!({"path": "MEMORY.md"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_memory_get),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    for request in [
        ToolCoreRequest {
            tool_name: "session_search".to_owned(),
            payload: json!({"query": "deploy freeze"}),
        },
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({"limit": 5}),
        },
        ToolCoreRequest {
            tool_name: "session_wait".to_owned(),
            payload: json!({"session_id": "child-session"}),
        },
        ToolCoreRequest {
            tool_name: "session_tool_policy_status".to_owned(),
            payload: json!({"session_id": "root-session"}),
        },
    ] {
        assert_eq!(
            required_capabilities_for_request(&request),
            BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead])
        );
    }

    let direct_web_fetch = ToolCoreRequest {
        tool_name: "web.fetch".to_owned(),
        payload: json!({"url": "https://example.com"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_web_fetch),
        BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress])
    );

    let direct_http_request = ToolCoreRequest {
        tool_name: "http.request".to_owned(),
        payload: json!({"url": "https://example.com"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_http_request),
        BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress])
    );

    let direct_web_search = ToolCoreRequest {
        tool_name: "web.search".to_owned(),
        payload: json!({"query": "loong"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_web_search),
        BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress])
    );

    let direct_browser_open = ToolCoreRequest {
        tool_name: "browser.open".to_owned(),
        payload: json!({"url": "https://example.com"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_browser_open),
        BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress])
    );

    let direct_browser_extract = ToolCoreRequest {
        tool_name: "browser.extract".to_owned(),
        payload: json!({"mode": "page_text"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_browser_extract),
        BTreeSet::from([Capability::InvokeTool])
    );

    let direct_browser_click = ToolCoreRequest {
        tool_name: "browser.click".to_owned(),
        payload: json!({"id": 1}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_browser_click),
        BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress])
    );

    let direct_bash_exec = ToolCoreRequest {
        tool_name: "bash.exec".to_owned(),
        payload: json!({"command": "printf ok"}),
    };
    assert_eq!(
        required_capabilities_for_request(&direct_bash_exec),
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkEgress,
        ])
    );

    let invoked_file_read = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "file.read",
            "lease": "unused",
            "arguments": {"path": "README.md"}
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_file_read),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let invoked_memory_search = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "memory_search",
            "lease": "unused",
            "arguments": {"query": "deploy freeze"}
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_memory_search),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let invoked_web_fetch = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "web.fetch",
            "lease": "unused",
            "arguments": {"url": "https://example.com"}
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_web_fetch),
        BTreeSet::from([Capability::InvokeTool, Capability::NetworkEgress])
    );

    let invoked_bash_exec = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "bash.exec",
            "lease": "unused",
            "arguments": {"command": "printf ok"}
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_bash_exec),
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkEgress,
        ])
    );

    let invoked_config_import_plan = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "config.import",
            "lease": "unused",
            "arguments": {"mode": "plan", "input_path": "imports/nanobot"}
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_config_import_plan),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead])
    );

    let invoked_config_import_apply = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "config.import",
            "lease": "unused",
            "arguments": {
                "mode": "apply",
                "input_path": "imports/nanobot",
                "output_path": "loong.toml"
            }
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_config_import_apply),
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ])
    );

    let invoked_config_import_apply_selected = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "config.import",
            "lease": "unused",
            "arguments": {
                "mode": "apply_selected",
                "input_path": "imports/nanobot",
                "output_path": "loong.toml",
                "source_id": "source-1"
            }
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_config_import_apply_selected),
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ])
    );

    let invoked_config_import_rollback = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "config.import",
            "lease": "unused",
            "arguments": {
                "mode": "rollback_last_apply",
                "output_path": "loong.toml"
            }
        }),
    };
    assert_eq!(
        required_capabilities_for_request(&invoked_config_import_rollback),
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ])
    );

    let malformed_invoke = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({"lease": "unused"}),
    };
    assert_eq!(
        required_capabilities_for_request(&malformed_invoke),
        BTreeSet::from([Capability::InvokeTool])
    );

    assert_eq!(canonical_tool_name("file.read"), "file.read");
}
