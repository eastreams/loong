use std::sync::OnceLock;
#[cfg(any(test, feature = "test-hooks"))]
use std::{collections::BTreeMap, sync::Mutex};

use kernel::{ToolCoreOutcome, ToolCoreRequest};

pub mod kernel_bootstrap;
pub mod programmatic;
pub mod spec_execution;
pub mod spec_runtime;

pub use kernel_bootstrap::{KernelBuilder, default_pack_manifest};
pub use programmatic::{
    acquire_programmatic_circuit_slot, execute_programmatic_tool_call,
    record_programmatic_circuit_outcome,
};
pub use spec_execution::*;
pub use spec_runtime::*;

pub const DEFAULT_PACK_ID: &str = "dev-automation";
pub const DEFAULT_AGENT_ID: &str = "agent-dev-01";
pub type NativeToolExecutor = fn(ToolCoreRequest) -> Option<Result<ToolCoreOutcome, String>>;
pub static BUNDLED_APPROVAL_RISK_PROFILE: OnceLock<ApprovalRiskProfile> = OnceLock::new();
pub static BUNDLED_SECURITY_SCAN_PROFILE: OnceLock<SecurityScanProfile> = OnceLock::new();
#[cfg(any(test, feature = "test-hooks"))]
pub static WEBHOOK_TEST_RETRY_STATE: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
pub type CliResult<T> = Result<T, String>;
