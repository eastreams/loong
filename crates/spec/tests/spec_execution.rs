use std::collections::BTreeSet;

use kernel::{AuditEventKind, Capability};
use loongclaw_spec::test_support::make_runner_spec;
use loongclaw_spec::{OperationSpec, execute_spec, spec_requires_native_tool_executor};
use serde_json::json;

#[test]
fn spec_requires_native_tool_executor_detects_aliases_and_extension() {
    let native_tool_names = [
        "claw.migrate",
        "claw_migrate",
        "config.import",
        "config_import",
    ];
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

    for tool_name in native_tool_names {
        let spec = make_runner_spec(OperationSpec::ToolCore {
            tool_name: tool_name.to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"mode": "plan"}),
            core: None,
        });

        assert!(
            spec_requires_native_tool_executor(&spec),
            "expected `{tool_name}` to require a native tool executor"
        );
    }
    assert!(spec_requires_native_tool_executor(&extension_spec));
    assert!(!spec_requires_native_tool_executor(&unrelated_spec));
}

#[tokio::test]
async fn execute_spec_blocks_native_tool_without_executor() {
    let spec = make_runner_spec(OperationSpec::ToolCore {
        tool_name: "config.import".to_owned(),
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

#[tokio::test]
async fn execute_spec_returns_audit_snapshot_when_requested() {
    let spec = make_runner_spec(OperationSpec::Task {
        task_id: "spec-audit-task".to_owned(),
        objective: "exercise spec audit snapshot capture".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"kind": "audit-contract-check"}),
    });

    let report = execute_spec(&spec, true).await;

    let audit_events = report
        .audit_events
        .as_ref()
        .expect("audit events should be present when explicitly requested");
    assert!(
        !audit_events.is_empty(),
        "expected spec execution to retain at least one in-memory audit event"
    );
    assert!(
        audit_events
            .iter()
            .any(|event| matches!(event.kind, AuditEventKind::TokenIssued { .. })),
        "expected token issuance to be retained in the audit snapshot"
    );
}

#[tokio::test]
async fn execute_spec_suppresses_audit_snapshot_when_not_requested() {
    let spec = make_runner_spec(OperationSpec::Task {
        task_id: "spec-audit-task".to_owned(),
        objective: "exercise spec audit snapshot suppression".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"kind": "audit-contract-check"}),
    });

    let report = execute_spec(&spec, false).await;

    assert!(
        report.audit_events.is_none(),
        "audit snapshots should stay opt-in for spec execution reports"
    );
}

#[tokio::test]
async fn execute_spec_task_reports_supervisor_terminal_state() {
    let spec = make_runner_spec(OperationSpec::Task {
        task_id: "spec-supervisor-task".to_owned(),
        objective: "exercise task supervisor in spec execution".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"kind": "supervisor-contract-check"}),
    });

    let report = execute_spec(&spec, false).await;

    assert_eq!(report.operation_kind, "task");
    assert_eq!(
        report.outcome["supervisor_state"]["Completed"]["status"],
        "ok"
    );
    assert_eq!(
        report.outcome["supervisor_state"]["Completed"]["output"]["task"],
        "spec-supervisor-task"
    );
    assert!(report.outcome["error"].is_null());
}

#[tokio::test]
async fn execute_spec_task_preserves_faulted_supervisor_terminal_state() {
    let spec = make_runner_spec(OperationSpec::Task {
        task_id: "spec-faulted-task".to_owned(),
        objective: "exercise faulted task supervisor in spec execution".to_owned(),
        required_capabilities: BTreeSet::from([Capability::MemoryRead]),
        payload: json!({"kind": "supervisor-contract-check"}),
    });

    let report = execute_spec(&spec, false).await;

    assert_eq!(report.operation_kind, "task");
    assert!(report.blocked_reason.is_none());
    assert!(report.outcome["route"].is_null());
    assert!(report.outcome["outcome"].is_null());
    assert!(
        report.outcome["supervisor_state"].get("Faulted").is_some(),
        "spec task execution should preserve the faulted supervisor state",
    );
    assert!(
        report.outcome["error"]
            .as_str()
            .is_some_and(|error| error.contains("task execution from spec failed")),
        "spec task execution should preserve a structured error summary",
    );
}
