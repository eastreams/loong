//! Test support utilities for loong-bench integration tests.
//!
//! Available when the `test-support` feature is enabled.

pub use crate::ProgrammaticPressureScenario;
pub use crate::ProgrammaticPressureScenarioKind;
pub use crate::copy_benchmark_file;
pub use crate::run_spec_pressure_once;

use kernel::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::json;

/// Test native tool executor that handles native tool requests.
pub fn test_native_tool_executor(
    request: ToolCoreRequest,
) -> Option<Result<ToolCoreOutcome, String>> {
    if !loong_spec::tool_name_requires_native_tool_executor(request.tool_name.as_str()) {
        return None;
    }
    Some(Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "native-tools",
            "tool": request.tool_name,
        }),
    }))
}

/// Test native tool executor that declines native tool requests.
pub fn declining_native_tool_executor(
    request: ToolCoreRequest,
) -> Option<Result<ToolCoreOutcome, String>> {
    if loong_spec::tool_name_requires_native_tool_executor(request.tool_name.as_str()) {
        return None;
    }
    Some(Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "native-tools",
            "tool": request.tool_name,
        }),
    }))
}
