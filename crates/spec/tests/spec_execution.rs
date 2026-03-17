use std::collections::BTreeSet;

use kernel::Capability;
use loongclaw_spec::test_support::make_runner_spec;
use loongclaw_spec::{OperationSpec, execute_spec, spec_requires_native_tool_executor};
use serde_json::json;

#[test]
fn spec_requires_native_tool_executor_detects_aliases_and_extension() {
    let alias_spec = make_runner_spec(OperationSpec::ToolCore {
        tool_name: "claw_migrate".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({"mode": "plan"}),
        core: None,
    });
    let extension_spec = make_runner_spec(OperationSpec::ToolExtension {
        extension_action: "plan".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({"input_path": "/tmp/demo"}),
        extension: "claw-migration".to_owned(),
        core: None,
    });
    let unrelated_spec = make_runner_spec(OperationSpec::ToolCore {
        tool_name: "file.read".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({"path": "/tmp/demo"}),
        core: None,
    });

    assert!(spec_requires_native_tool_executor(&alias_spec));
    assert!(spec_requires_native_tool_executor(&extension_spec));
    assert!(!spec_requires_native_tool_executor(&unrelated_spec));
}

#[tokio::test]
async fn execute_spec_blocks_native_tool_without_executor() {
    let spec = make_runner_spec(OperationSpec::ToolCore {
        tool_name: "claw.migrate".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({"mode": "plan"}),
        core: None,
    });

    let report = execute_spec(&spec, false).await;

    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .as_deref()
            .expect("blocked reason should exist")
            .contains("native tool executor")
    );
}
