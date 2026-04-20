use std::{collections::BTreeSet, path::PathBuf};

use kernel::{BridgeSupportMatrix, PluginDescriptor};

use super::*;

pub(super) fn resolve_plugin_setup_readiness_context<I>(
    readiness_spec: Option<&PluginSetupReadinessSpec>,
    env_vars: I,
) -> PluginSetupReadinessContext
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let Some(readiness_spec) = readiness_spec else {
        let verified_env_vars = collect_verified_env_var_names(env_vars);

        return PluginSetupReadinessContext {
            verified_env_vars,
            verified_config_keys: BTreeSet::new(),
        };
    };

    let mut verified_env_vars = BTreeSet::new();
    if readiness_spec.inherit_process_env {
        verified_env_vars = collect_verified_env_var_names(env_vars);
    }

    let explicit_verified_env_vars = collect_verified_name_list(&readiness_spec.verified_env_vars);
    verified_env_vars.extend(explicit_verified_env_vars);

    let verified_config_keys = collect_verified_name_list(&readiness_spec.verified_config_keys);

    PluginSetupReadinessContext {
        verified_env_vars,
        verified_config_keys,
    }
}

pub(crate) fn collect_verified_env_var_names<I>(env_vars: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut verified_env_vars = BTreeSet::new();

    for (raw_name, raw_value) in env_vars {
        let name = raw_name.to_string_lossy().into_owned();
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            continue;
        }

        let value = raw_value.to_string_lossy().into_owned();
        let trimmed_value = value.trim();
        if trimmed_value.is_empty() {
            continue;
        }

        verified_env_vars.insert(name);
    }

    verified_env_vars
}

fn collect_verified_name_list(values: &[String]) -> BTreeSet<String> {
    let mut verified_names = BTreeSet::new();

    for raw_value in values {
        let value = raw_value.trim().to_owned();
        if value.is_empty() {
            continue;
        }

        verified_names.insert(value);
    }

    verified_names
}

pub(super) fn bridge_support_spec_matrix(bridge: &BridgeSupportSpec) -> BridgeSupportMatrix {
    let mut matrix = BridgeSupportMatrix::default();
    if !bridge.supported_bridges.is_empty() {
        matrix.supported_bridges = bridge.supported_bridges.iter().copied().collect();
    }
    if !bridge.supported_adapter_families.is_empty() {
        matrix.supported_adapter_families =
            bridge.supported_adapter_families.iter().cloned().collect();
    }
    if !bridge.supported_compatibility_modes.is_empty() {
        matrix.supported_compatibility_modes = bridge
            .supported_compatibility_modes
            .iter()
            .copied()
            .collect();
    }
    if !bridge.supported_compatibility_shims.is_empty() {
        matrix.supported_compatibility_shims = bridge
            .supported_compatibility_shims
            .iter()
            .cloned()
            .collect();
    }
    if !bridge.supported_compatibility_shim_profiles.is_empty() {
        matrix.supported_compatibility_shim_profiles = bridge
            .supported_compatibility_shim_profiles
            .iter()
            .cloned()
            .map(PluginCompatibilityShimSupport::normalized)
            .map(|profile| (profile.shim.clone(), profile))
            .collect();
        matrix
            .supported_compatibility_shims
            .extend(matrix.supported_compatibility_shim_profiles.keys().cloned());
    }
    matrix
}

fn raw_bridge_runtime_spec(spec: &RunnerSpec) -> SecurityRuntimeExecutionSpec {
    let raw_runtime = spec
        .bridge_support
        .as_ref()
        .filter(|bridge| bridge.enabled)
        .and_then(|bridge| bridge.security_scan.as_ref())
        .map(|scan| scan.runtime.clone());

    raw_runtime.unwrap_or_default()
}

pub(super) fn bridge_runtime_policy(
    spec: &RunnerSpec,
    security_scan: Option<&SecurityScanSpec>,
) -> CliResult<BridgeRuntimePolicy> {
    let Some(bridge) = &spec.bridge_support else {
        return Ok(BridgeRuntimePolicy::default());
    };
    if !bridge.enabled {
        return Ok(BridgeRuntimePolicy::default());
    }

    let runtime = security_scan
        .map(|scan| scan.runtime.clone())
        .unwrap_or_default();
    let raw_runtime = raw_bridge_runtime_spec(spec);
    let bridge_circuit_breaker = if security_scan.is_some() {
        runtime.bridge_circuit_breaker.clone()
    } else {
        raw_runtime.bridge_circuit_breaker
    };
    validate_connector_circuit_breaker_policy(
        &bridge_circuit_breaker,
        "bridge runtime circuit breaker",
    )?;
    let (compatibility_matrix, _) = bridge_support_matrix(spec);
    let (wasm_require_hash_pin, wasm_required_sha256_by_plugin) = security_scan
        .map(|scan| {
            (
                scan.wasm.require_hash_pin,
                scan.wasm.required_sha256_by_plugin.clone(),
            )
        })
        .unwrap_or_else(|| (false, BTreeMap::new()));
    let wasm_allowed_path_prefixes = runtime
        .allowed_path_prefixes
        .iter()
        .map(PathBuf::from)
        .map(|path| normalize_path_for_policy(&path))
        .collect();
    let wasm_guest_readable_config_keys =
        collect_verified_name_list(&runtime.guest_readable_config_keys);
    validate_wasm_guest_readable_config_keys(&wasm_guest_readable_config_keys)?;

    Ok(BridgeRuntimePolicy {
        execute_process_stdio: bridge.execute_process_stdio,
        execute_http_json: bridge.execute_http_json,
        execute_wasm_component: runtime.execute_wasm_component,
        compatibility_matrix,
        allowed_process_commands: bridge
            .allowed_process_commands
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect(),
        bridge_circuit_breaker,
        wasm_allowed_path_prefixes,
        wasm_guest_readable_config_keys,
        wasm_max_component_bytes: runtime.max_component_bytes,
        wasm_max_output_bytes: runtime.max_output_bytes,
        wasm_fuel_limit: runtime.fuel_limit,
        wasm_timeout_ms: runtime.timeout_ms,
        wasm_require_hash_pin,
        wasm_required_sha256_by_plugin,
        enforce_execution_success: bridge.enforce_execution_success,
    })
}

pub(super) fn filter_scan_report_by_activation(
    report: &PluginScanReport,
    activation: &PluginActivationPlan,
) -> PluginScanReport {
    let ready_keys: BTreeSet<(String, String)> = activation
        .candidates
        .iter()
        .filter(|candidate| matches!(candidate.status, PluginActivationStatus::Ready))
        .map(|candidate| (candidate.source_path.clone(), candidate.plugin_id.clone()))
        .collect();

    filter_scan_report_by_keys(report, &ready_keys)
}

pub(super) fn filter_scan_report_by_keys(
    report: &PluginScanReport,
    allowed_keys: &BTreeSet<(String, String)>,
) -> PluginScanReport {
    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .filter(|descriptor| {
            allowed_keys.contains(&(
                descriptor.path.clone(),
                descriptor.manifest.plugin_id.clone(),
            ))
        })
        .cloned()
        .collect();
    let diagnostic_findings = report
        .diagnostic_findings
        .iter()
        .filter(|finding| {
            let (Some(source_path), Some(plugin_id)) =
                (finding.source_path.as_deref(), finding.plugin_id.as_deref())
            else {
                return false;
            };

            allowed_keys.contains(&(source_path.to_owned(), plugin_id.to_owned()))
        })
        .cloned()
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        diagnostic_findings,
        descriptors,
    }
}
