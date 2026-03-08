use async_trait::async_trait;
use loongclaw_contracts::MemoryPlaneError;
use loongclaw_kernel::{CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest};

pub struct MvpMemoryAdapter;

#[async_trait]
impl CoreMemoryAdapter for MvpMemoryAdapter {
    fn name(&self) -> &str {
        "mvp-memory"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        super::execute_memory_core(request).map_err(MemoryPlaneError::Execution)
    }
}
