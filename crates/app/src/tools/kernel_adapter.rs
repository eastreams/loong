use async_trait::async_trait;
use loong_contracts::ToolPlaneError;
use loong_kernel::{CoreToolAdapter, Kernel, ToolCoreOutcome, ToolCoreRequest};

use super::runtime_config::ToolRuntimeConfig;
use crate::config::ObservabilityConfig;
use crate::context::AppContextFactory;

pub struct KernelToolAdapter {
    config: Option<ToolRuntimeConfig>,
    observability_config: ObservabilityConfig,
}

impl Default for KernelToolAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelToolAdapter {
    pub fn new() -> Self {
        Self {
            config: None,
            observability_config: default_observability_config(),
        }
    }

    pub fn with_config(config: ToolRuntimeConfig) -> Self {
        Self {
            config: Some(config),
            observability_config: default_observability_config(),
        }
    }

    pub fn with_config_and_observability(
        config: ToolRuntimeConfig,
        observability_config: ObservabilityConfig,
    ) -> Self {
        Self {
            config: Some(config),
            observability_config,
        }
    }
}

fn default_observability_config() -> ObservabilityConfig {
    ObservabilityConfig::runtime_default()
}

pub(crate) fn register_kernel_tools(
    kernel: &mut Kernel<AppContextFactory>,
    config: ToolRuntimeConfig,
    observability_config: ObservabilityConfig,
) -> Result<(), loong_kernel::KernelError> {
    // Kernel owns only the legacy adapter bridge. App-owned typed tools are
    // registered in `tools::plane` and are invoked by app orchestration.
    kernel.register_core_tool_adapter(KernelToolAdapter::with_config_and_observability(
        config,
        observability_config,
    ));
    kernel.set_default_core_tool_adapter("mvp-tools")?;
    Ok(())
}

#[async_trait]
impl CoreToolAdapter<AppContextFactory> for KernelToolAdapter {
    fn name(&self) -> &str {
        "mvp-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, ToolPlaneError> {
        match &self.config {
            Some(config) => super::tool_dispatch::execute_tool_core_with_config_and_observability(
                request,
                config,
                &self.observability_config,
            ),
            None => super::execute_tool_core(request),
        }
        .map_err(ToolPlaneError::Execution)
    }
}

pub type MvpToolAdapter = KernelToolAdapter;
