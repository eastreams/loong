use async_trait::async_trait;
use loong_contracts::ToolPlaneError;
use loong_kernel::{CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest};

use super::runtime_config::ToolRuntimeConfig;
use crate::config::ObservabilityConfig;
use crate::context::{AppContextFactory, AppExecutionContext};

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

    async fn execute_core_tool_with_context(
        &self,
        request: ToolCoreRequest,
        ctx: &AppExecutionContext<'_>,
    ) -> Result<ToolCoreOutcome, ToolPlaneError> {
        // Migrated tools need the kernel context so protected side effects can
        // run through access modules. The context-free entry point remains only
        // for tools that have not moved yet.
        match &self.config {
            Some(config) => {
                super::tool_dispatch::execute_tool_core_with_config_and_context(
                    request,
                    config,
                    &self.observability_config,
                    ctx,
                )
                .await
            }
            None => {
                let observability_config = default_observability_config();
                super::tool_dispatch::execute_tool_core_with_config_and_context(
                    request,
                    super::runtime_config::get_tool_runtime_config(),
                    &observability_config,
                    ctx,
                )
                .await
            }
        }
        .map_err(ToolPlaneError::Execution)
    }
}

pub type MvpToolAdapter = KernelToolAdapter;
