use std::sync::OnceLock;
#[cfg(any(test, feature = "test-hooks"))]
use std::{collections::BTreeMap, sync::Mutex};

use kernel::{ToolCoreOutcome, ToolCoreRequest};

pub mod kernel_bootstrap;
pub mod programmatic;
pub mod spec_execution;
pub mod spec_runtime;

pub mod test_support;

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
    matches!(
        tool_name,
        "config.import" | "config_import" | "claw.migrate" | "claw_migrate"
    )
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
        | OperationSpec::PluginInventory { .. }
        | OperationSpec::PluginPreflight { .. }
        | OperationSpec::ProgrammaticToolCall { .. } => false,
    }
}

pub static BUNDLED_APPROVAL_RISK_PROFILE: OnceLock<ApprovalRiskProfile> = OnceLock::new();
pub static BUNDLED_BRIDGE_SUPPORT_NATIVE_BALANCED: OnceLock<Result<BridgeSupportSpec, String>> =
    OnceLock::new();
pub static BUNDLED_BRIDGE_SUPPORT_OPENCLAW_ECOSYSTEM_BALANCED: OnceLock<
    Result<BridgeSupportSpec, String>,
> = OnceLock::new();
pub static BUNDLED_PLUGIN_PREFLIGHT_POLICY: OnceLock<Result<PluginPreflightPolicyProfile, String>> =
    OnceLock::new();
pub static BUNDLED_SECURITY_SCAN_PROFILE: OnceLock<SecurityScanProfile> = OnceLock::new();
#[cfg(any(test, feature = "test-hooks"))]
pub static WEBHOOK_TEST_RETRY_STATE: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
pub type CliResult<T> = Result<T, String>;
