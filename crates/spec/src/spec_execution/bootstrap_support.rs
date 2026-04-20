use super::*;

pub(crate) fn current_epoch_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(super) fn bootstrap_policy(spec: &RunnerSpec) -> Option<BootstrapPolicy> {
    let bootstrap = spec.bootstrap.as_ref()?;
    if !bootstrap.enabled {
        return None;
    }

    let mut policy = BootstrapPolicy::default();
    if let Some(value) = bootstrap.allow_http_json_auto_apply {
        policy.allow_http_json_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_process_stdio_auto_apply {
        policy.allow_process_stdio_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_native_ffi_auto_apply {
        policy.allow_native_ffi_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_wasm_component_auto_apply {
        policy.allow_wasm_component_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_mcp_server_auto_apply {
        policy.allow_mcp_server_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_acp_bridge_auto_apply {
        policy.allow_acp_bridge_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_acp_runtime_auto_apply {
        policy.allow_acp_runtime_auto_apply = value;
    }
    if let Some(value) = bootstrap.block_unverified_high_risk_auto_apply {
        policy.block_unverified_high_risk_auto_apply = value;
    }
    if let Some(value) = bootstrap.enforce_ready_execution {
        policy.enforce_ready_execution = value;
    }
    if let Some(value) = bootstrap.max_tasks {
        policy.max_tasks = value.max(1);
    }
    Some(policy)
}

pub(super) fn apply_default_selection(
    kernel: &mut LoongKernel<StaticPolicyEngine>,
    defaults: Option<&DefaultCoreSelection>,
) -> CliResult<()> {
    if let Some(defaults) = defaults {
        if let Some(connector) = defaults.connector.as_deref() {
            kernel
                .set_default_core_connector_adapter(connector)
                .map_err(|error| {
                    format!("invalid default connector core adapter ({connector}): {error}")
                })?;
        }
        if let Some(runtime) = defaults.runtime.as_deref() {
            kernel
                .set_default_core_runtime_adapter(runtime)
                .map_err(|error| {
                    format!("invalid default runtime core adapter ({runtime}): {error}")
                })?;
        }
        if let Some(tool) = defaults.tool.as_deref() {
            kernel
                .set_default_core_tool_adapter(tool)
                .map_err(|error| format!("invalid default tool core adapter ({tool}): {error}"))?;
        }
        if let Some(memory) = defaults.memory.as_deref() {
            kernel
                .set_default_core_memory_adapter(memory)
                .map_err(|error| {
                    format!("invalid default memory core adapter ({memory}): {error}")
                })?;
        }
    }
    Ok(())
}
