use serde_json::json;

use super::*;

#[test]
fn standalone_legacy_tool_invoke_rejects_capability_override() {
    let arguments = serde_json::Map::new();
    let lease = issue_tool_lease("config.import", &arguments)
        .expect("legacy config import lease should be issued");
    let request = ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "config.import",
            "lease": lease,
            "arguments": arguments,
            "capabilities_override": [],
        }),
    };

    let error = execute_tool_invoke_tool_with_config(
        request,
        &runtime_config::ToolRuntimeConfig::default(),
    )
    .expect_err("legacy execution must not discard capability narrowing");

    assert!(error.contains("capabilities_override requires a registered typed tool"));
}

#[cfg(feature = "tool-file")]
// Both exposure modes must receive the same valid leased envelope; only the
// policy at the caller boundary is allowed to differ.
fn file_read_alias_request() -> ToolCoreRequest {
    let lease_payload = serde_json::Map::new();
    let lease =
        issue_tool_lease("read", &lease_payload).expect("canonical read lease should be issued");
    ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": "file.read",
            "lease": lease,
            "arguments": { "path": "notes.txt" },
        }),
    }
}

#[cfg(feature = "tool-file")]
#[test]
fn provider_exposed_alias_is_rejected_after_path_canonicalization() {
    let error = resolve_tool_invoke_request(
        &file_read_alias_request(),
        ToolInvokeProviderExposure::RejectProviderExposed,
    )
    .expect_err("provider-exposed read must remain a direct provider call");

    assert_eq!(
        error,
        "tool_not_provider_exposed: read must be called directly as a core tool"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn provider_exposed_alias_is_canonicalized_for_typed_lookup() {
    let resolved = resolve_tool_invoke_request(
        &file_read_alias_request(),
        ToolInvokeProviderExposure::AllowProviderExposed,
    )
    .expect("typed ingress may invoke provider-exposed tools");

    assert_eq!(resolved.request.tool_name, "read");
    assert_eq!(resolved.request.payload, json!({ "path": "notes.txt" }));
}
