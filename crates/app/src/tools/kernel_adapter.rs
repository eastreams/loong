use async_trait::async_trait;
use loongclaw_contracts::ToolPlaneError;
use loongclaw_kernel::{CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest};

pub struct MvpToolAdapter;

#[async_trait]
impl CoreToolAdapter for MvpToolAdapter {
    fn name(&self) -> &str {
        "mvp-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, ToolPlaneError> {
        super::execute_tool_core(request).map_err(ToolPlaneError::Execution)
    }
}
