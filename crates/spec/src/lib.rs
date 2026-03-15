use std::sync::OnceLock;
#[cfg(any(test, feature = "test-hooks"))]
use std::{collections::BTreeMap, sync::Mutex};

use kernel::{ToolCoreOutcome, ToolCoreRequest};

pub mod kernel_bootstrap;
pub mod programmatic;
pub mod spec_execution;
pub mod spec_runtime;

pub use kernel_bootstrap::{BootstrapBuilder, KernelBuilder, default_pack_manifest};
pub use programmatic::{
    acquire_programmatic_circuit_slot, execute_programmatic_tool_call,
    record_programmatic_circuit_outcome,
};
pub use spec_execution::*;
pub use spec_runtime::*;

pub const DEFAULT_PACK_ID: &str = "dev-automation";
pub const DEFAULT_AGENT_ID: &str = "agent-dev-01";
pub type NativeToolExecutor = fn(ToolCoreRequest) -> Option<Result<ToolCoreOutcome, String>>;

pub fn tool_name_requires_native_tool_executor(tool_name: &str) -> bool {
    matches!(tool_name, "claw.import" | "claw_import" | "import_claw")
}

pub fn spec_requires_native_tool_executor(spec: &RunnerSpec) -> bool {
    match &spec.operation {
        OperationSpec::ToolCore { tool_name, .. } => {
            tool_name_requires_native_tool_executor(tool_name)
        }
        OperationSpec::ToolExtension { extension, .. } => extension == "claw-migration",
        OperationSpec::Task { .. }
        | OperationSpec::ConnectorLegacy { .. }
        | OperationSpec::ConnectorCore { .. }
        | OperationSpec::ConnectorExtension { .. }
        | OperationSpec::RuntimeCore { .. }
        | OperationSpec::RuntimeExtension { .. }
        | OperationSpec::MemoryCore { .. }
        | OperationSpec::MemoryExtension { .. }
        | OperationSpec::ToolSearch { .. }
        | OperationSpec::ProgrammaticToolCall { .. } => false,
    }
}

pub static BUNDLED_APPROVAL_RISK_PROFILE: OnceLock<ApprovalRiskProfile> = OnceLock::new();
pub static BUNDLED_SECURITY_SCAN_PROFILE: OnceLock<SecurityScanProfile> = OnceLock::new();
#[cfg(any(test, feature = "test-hooks"))]
pub static WEBHOOK_TEST_RETRY_STATE: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
pub type CliResult<T> = Result<T, String>;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};
    use serde_json::json;

    use super::{OperationSpec, RunnerSpec, execute_spec, spec_requires_native_tool_executor};

    fn make_runner_spec(operation: OperationSpec) -> RunnerSpec {
        RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-native-tool-check".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-native-tool-check".to_owned(),
            ttl_s: 60,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation,
        }
    }

    #[test]
    fn spec_requires_native_tool_executor_detects_aliases_and_extension() {
        let alias_spec = make_runner_spec(OperationSpec::ToolCore {
            tool_name: "claw_import".to_owned(),
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
            tool_name: "claw.import".to_owned(),
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
}
