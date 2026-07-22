use serde_json::json;

use super::*;

#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

#[test]
fn standalone_legacy_tool_invoke_rejects_capability_override() {
    let arguments = serde_json::Map::new();
    let lease = issue_tool_lease(&tool_path("config.import"), &arguments)
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
#[test]
fn typed_alias_lease_validates_against_runtime_canonical_path() {
    let lease_payload = serde_json::Map::new();
    let lease = issue_tool_lease(&tool_path("read"), &lease_payload)
        .expect("canonical read lease should be issued");
    let envelope = json!({
        "tool_id": "file.read",
        "lease": lease,
        "arguments": { "path": "notes.txt" },
    });
    let parsed = parse_tool_invoke_request("tool.invoke", &envelope)
        .expect("valid envelope should preserve its raw typed path");
    assert_eq!(parsed.path, tool_path("file.read"));

    let resolved = parsed
        .resolve(tool_path("read"))
        .expect("canonical runtime path should match the issued lease");
    assert_eq!(resolved.path, tool_path("read"));
}

#[test]
fn tool_lease_binds_exact_canonical_path_segments() {
    let exact_path = tool_path("alpha.beta");
    let split_path = ToolPath::new(["alpha", "beta"]).expect("test tool path must be valid");
    let lease_payload = serde_json::Map::new();
    let lease =
        issue_tool_lease(&exact_path, &lease_payload).expect("exact path lease should be issued");
    let envelope = json!({
        "tool_id": exact_path.to_string(),
        "lease": lease,
        "arguments": {},
    });
    let parsed = parse_tool_invoke_request("tool.invoke", &envelope)
        .expect("canonical exact path should parse");

    assert_eq!(parsed.path, exact_path);
    let error = parsed
        .resolve(split_path)
        .expect_err("a split path must not satisfy an embedded-dot lease");
    assert!(error.contains("tool mismatch"));
}
