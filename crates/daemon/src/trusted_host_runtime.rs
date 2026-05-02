use std::collections::{BTreeMap, BTreeSet};

use loong_bridge_runtime::{BridgeExecutionPolicy, execute_process_stdio_bridge_call};
use loong_contracts::ConnectorCommand;
use serde::Serialize;
use serde_json::{Value, json};

use crate::CliResult;
use crate::kernel::{
    self, Capability, PluginActivationStatus, PluginBridgeKind, PluginIR, PluginScanner,
    PluginSetupReadinessContext, PluginTranslator,
};
use crate::mvp;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ProcessStdioExtensionInvocationOutcome {
    pub response_payload: Value,
    pub runtime_evidence: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct TrustedHostHookDispatchResult {
    pub plugin_id: String,
    pub source_path: String,
    pub response_payload: Value,
    pub runtime_evidence: Value,
}

pub(crate) fn build_process_stdio_bridge_policy_from_allow_commands(
    allow_commands: Vec<String>,
    empty_error_message: &str,
) -> CliResult<BridgeExecutionPolicy> {
    if allow_commands.is_empty() {
        return Err(empty_error_message.to_owned());
    }

    Ok(BridgeExecutionPolicy {
        execute_process_stdio: true,
        execute_http_json: false,
        allowed_process_commands: allow_commands
            .into_iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>(),
    })
}

pub(crate) fn build_read_only_trusted_host_hook_payload(hook: &str, payload: Value) -> Value {
    json!({
        "event": hook,
        "host_hook": hook,
        "hook_kind": "read_only",
        "hook_payload": payload,
    })
}

pub(crate) fn build_read_only_trusted_host_tui_surface_payload(
    surface: &str,
    payload: Value,
) -> Value {
    json!({
        "event": "tui_surface",
        "host_tui_surface": surface,
        "surface_kind": "read_only",
        "surface_payload": payload,
    })
}

pub(crate) async fn invoke_process_stdio_extension_operation(
    plugin: &PluginIR,
    operation: &str,
    payload: Value,
    bridge_policy: &BridgeExecutionPolicy,
) -> CliResult<ProcessStdioExtensionInvocationOutcome> {
    let provider = extension_provider_config(plugin);
    let channel = kernel::ChannelConfig {
        channel_id: "native_extension".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "local://native-extension".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let connector_command = ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: operation.to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload,
    };

    let outcome =
        execute_process_stdio_bridge_call(&provider, &channel, &connector_command, bridge_policy)
            .await
            .map_err(|failure| failure.reason)?;

    Ok(ProcessStdioExtensionInvocationOutcome {
        response_payload: outcome.response_payload,
        runtime_evidence: outcome.runtime_evidence,
    })
}

pub(crate) async fn dispatch_trusted_host_hook(
    config: &mvp::config::LoongConfig,
    hook: &str,
    payload: Value,
) -> CliResult<Vec<TrustedHostHookDispatchResult>> {
    let trusted_plugins = collect_trusted_host_hook_plugins(config, hook)?;
    if trusted_plugins.is_empty() {
        return Ok(Vec::new());
    }

    let bridge_policy = BridgeExecutionPolicy {
        execute_process_stdio: true,
        execute_http_json: false,
        allowed_process_commands: config
            .runtime_plugins
            .normalized_allowed_process_commands()
            .into_iter()
            .collect::<BTreeSet<_>>(),
    };
    let hook_payload = build_read_only_trusted_host_hook_payload(hook, payload);
    let mut results = Vec::new();

    for plugin in trusted_plugins {
        let outcome = invoke_process_stdio_extension_operation(
            &plugin,
            "extension/event",
            hook_payload.clone(),
            &bridge_policy,
        )
        .await
        .map_err(|error| {
            format!(
                "trusted host hook `{hook}` failed for plugin {}: {error}",
                plugin.plugin_id
            )
        })?;
        results.push(TrustedHostHookDispatchResult {
            plugin_id: plugin.plugin_id.clone(),
            source_path: plugin.source_path.clone(),
            response_payload: outcome.response_payload,
            runtime_evidence: outcome.runtime_evidence,
        });
    }

    Ok(results)
}

pub(crate) async fn dispatch_turn_start_hook_for_request(
    config: &mvp::config::LoongConfig,
    session_hint: Option<&str>,
    request: &mvp::agent_runtime::AgentTurnRequest,
) -> CliResult<()> {
    let payload = trusted_host_request_context_payload(session_hint, request);
    dispatch_trusted_host_hook(config, "turn_start", payload)
        .await
        .map(|_| ())
}

fn trusted_host_request_context_payload(
    session_hint: Option<&str>,
    request: &mvp::agent_runtime::AgentTurnRequest,
) -> Value {
    json!({
        "session_hint": session_hint.map(str::trim).filter(|value| !value.is_empty()),
        "turn_mode": request.turn_mode,
        "message": request.message,
        "metadata": request.metadata,
        "acp_requested": request.acp || matches!(request.turn_mode, mvp::agent_runtime::AgentTurnMode::Acp),
        "live_surface_enabled": request.live_surface_enabled,
        "address": {
            "channel_id": request.channel_id,
            "account_id": request.account_id,
            "conversation_id": request.conversation_id,
            "participant_id": request.participant_id,
            "thread_id": request.thread_id,
        }
    })
}

fn collect_trusted_host_hook_plugins(
    config: &mvp::config::LoongConfig,
    hook: &str,
) -> CliResult<Vec<PluginIR>> {
    if !config.runtime_plugins.enabled {
        return Ok(Vec::new());
    }

    let root_selection = config.runtime_plugins.resolved_root_selection();
    let resolved_roots = root_selection.roots;
    if resolved_roots.is_empty() {
        return Ok(Vec::new());
    }

    let scan_report = scan_runtime_plugin_roots(&resolved_roots)?;
    let translator = PluginTranslator::new();
    let translation = translator.translate_scan_report(&scan_report);
    let readiness_context = runtime_plugin_setup_readiness_context(config)?;
    let bridge_matrix = config
        .runtime_plugins
        .resolved_bridge_support_matrix()
        .map_err(|error| format!("resolve runtime plugin bridge matrix failed: {error}"))?;
    let activation = translator.plan_activation(&translation, &bridge_matrix, &readiness_context);
    let mut trusted_plugins = Vec::new();

    for plugin in translation.entries {
        let activation_candidate = activation.candidate_for(&plugin.source_path, &plugin.plugin_id);
        let Some(activation_candidate) = activation_candidate else {
            continue;
        };
        if activation_candidate.status != PluginActivationStatus::Ready {
            continue;
        }

        let declarations =
            kernel::plugin_native_extension_declarations_from_metadata(&plugin.metadata);
        if declarations.family.as_deref() != Some(kernel::TRUSTED_HOST_EXTENSION_FAMILY)
            || declarations.trust_lane.as_deref() != Some(kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE)
        {
            continue;
        }
        if !declarations.host_hooks.iter().any(|value| value == hook) {
            continue;
        }
        if !declarations.metadata_issues.is_empty() {
            return Err(format!(
                "trusted host extension {} declares invalid host hook metadata: {}",
                plugin.plugin_id,
                declarations.metadata_issues.join("; ")
            ));
        }
        if !declarations
            .methods
            .iter()
            .any(|method| method == "extension/event")
        {
            return Err(format!(
                "trusted host extension {} must declare extension/event in loong_extension_methods_json",
                plugin.plugin_id
            ));
        }
        if plugin.runtime.bridge_kind != PluginBridgeKind::ProcessStdio {
            return Err(format!(
                "trusted host extension {} currently requires bridge_kind `process_stdio`; got `{}`",
                plugin.plugin_id,
                plugin.runtime.bridge_kind.as_str()
            ));
        }
        validate_process_stdio_plugin_execution(config, &plugin).map_err(|error| {
            format!(
                "trusted host extension {} is not executable: {error}",
                plugin.plugin_id
            )
        })?;
        trusted_plugins.push(plugin);
    }

    let trusted_plugins = if root_selection.source == "auto_discovered" {
        let selection =
            kernel::prefer_first_plugin_ids(trusted_plugins, |plugin| plugin.plugin_id.as_str());
        selection.effective
    } else {
        trusted_plugins
    };

    let mut trusted_plugins = trusted_plugins;
    trusted_plugins.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });

    Ok(trusted_plugins)
}

fn extension_provider_config(plugin: &PluginIR) -> kernel::ProviderConfig {
    let mut metadata = plugin.metadata.clone();
    metadata.insert(
        "plugin_package_root".to_owned(),
        plugin.package_root.clone(),
    );
    metadata.insert("plugin_id".to_owned(), plugin.plugin_id.clone());
    metadata.insert("plugin_source_path".to_owned(), plugin.source_path.clone());
    metadata.insert(
        "bridge_kind".to_owned(),
        plugin.runtime.bridge_kind.as_str().to_owned(),
    );
    metadata.insert(
        "adapter_family".to_owned(),
        plugin.runtime.adapter_family.clone(),
    );
    metadata.insert(
        "entrypoint_hint".to_owned(),
        plugin.runtime.entrypoint_hint.clone(),
    );
    metadata.insert(
        "source_language".to_owned(),
        plugin.runtime.source_language.clone(),
    );

    kernel::ProviderConfig {
        provider_id: plugin.provider_id.clone(),
        connector_name: plugin.connector_name.clone(),
        version: plugin
            .plugin_version
            .clone()
            .unwrap_or_else(|| "0.0.0".to_owned()),
        metadata,
    }
}

fn scan_runtime_plugin_roots(roots: &[std::path::PathBuf]) -> CliResult<kernel::PluginScanReport> {
    let scanner = PluginScanner::new();
    let mut combined_report = kernel::PluginScanReport::default();

    for root in roots {
        let root_report = scanner.scan_path(root).map_err(|error| {
            format!("runtime plugin scan failed for {}: {error}", root.display())
        })?;
        merge_plugin_scan_report(&mut combined_report, root_report);
    }

    Ok(combined_report)
}

fn merge_plugin_scan_report(
    target: &mut kernel::PluginScanReport,
    source: kernel::PluginScanReport,
) {
    target.scanned_files = target.scanned_files.saturating_add(source.scanned_files);
    target.matched_plugins = target
        .matched_plugins
        .saturating_add(source.matched_plugins);
    for descriptor in source.descriptors {
        target.descriptors.push(descriptor);
    }
}

fn runtime_plugin_setup_readiness_context(
    config: &mvp::config::LoongConfig,
) -> CliResult<PluginSetupReadinessContext> {
    let mut verified_env_vars = BTreeSet::new();
    for (key, value) in std::env::vars_os() {
        let value_string = value.to_string_lossy();
        let trimmed_value = value_string.trim();
        if trimmed_value.is_empty() {
            continue;
        }
        verified_env_vars.insert(key.to_string_lossy().to_string());
    }

    let config_value = serde_json::to_value(config)
        .map_err(|error| format!("serialize config failed: {error}"))?;
    let mut verified_config_keys = BTreeSet::new();
    collect_config_paths(&config_value, None, &mut verified_config_keys);

    Ok(PluginSetupReadinessContext {
        verified_env_vars,
        verified_config_keys,
    })
}

fn collect_config_paths(value: &Value, prefix: Option<&str>, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next_prefix = match prefix {
                    Some(prefix) => format!("{prefix}.{key}"),
                    None => key.clone(),
                };
                if child.is_null() {
                    continue;
                }
                out.insert(next_prefix.clone());
                collect_config_paths(child, Some(next_prefix.as_str()), out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_config_paths(child, prefix, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn validate_process_stdio_plugin_execution(
    config: &mvp::config::LoongConfig,
    plugin: &PluginIR,
) -> CliResult<()> {
    if plugin.runtime.bridge_kind != PluginBridgeKind::ProcessStdio {
        return Ok(());
    }

    let Some(command) = resolved_process_stdio_command(plugin) else {
        return Err(format!(
            "runtime plugin {} requires provider metadata.command or metadata.entrypoint for process_stdio execution",
            plugin.plugin_id
        ));
    };

    let normalized_allowed_commands = config.runtime_plugins.normalized_allowed_process_commands();
    if process_command_is_allowed(command.as_str(), &normalized_allowed_commands) {
        return Ok(());
    }

    Err(format!(
        "runtime plugin {} uses process command `{}` that is not allowlisted in runtime_plugins.allowed_process_commands",
        plugin.plugin_id, command
    ))
}

fn resolved_process_stdio_command(plugin: &PluginIR) -> Option<String> {
    let explicit_command = non_empty_metadata_value(&plugin.metadata, "command");
    if explicit_command.is_some() {
        return explicit_command;
    }

    let explicit_entrypoint = non_empty_metadata_value(&plugin.metadata, "entrypoint");
    if explicit_entrypoint.is_some() {
        return explicit_entrypoint;
    }

    let runtime_entrypoint = plugin.runtime.entrypoint_hint.trim();
    if runtime_entrypoint.is_empty() || runtime_entrypoint == "stdin/stdout::invoke" {
        return None;
    }

    Some(runtime_entrypoint.to_owned())
}

fn non_empty_metadata_value(
    metadata: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    let value = metadata.get(key)?;
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return None;
    }

    Some(trimmed_value.to_owned())
}

fn process_command_is_allowed(command: &str, allowed_commands: &[String]) -> bool {
    let trimmed_command = command.trim();
    let normalized_command = trimmed_command.to_ascii_lowercase();
    if allowed_commands.contains(&normalized_command) {
        return true;
    }

    let command_path = std::path::Path::new(trimmed_command);
    let has_path_component = command_path.is_absolute()
        || command_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
    if has_path_component {
        return false;
    }

    let Some(file_name) = command_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    allowed_commands.contains(&file_name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::{Path, PathBuf};

    fn write_file(root: &Path, relative_path: &str, contents: &str) {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write file");
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create unique temp dir");
        path
    }

    fn install_trusted_host_runtime_plugin(root: &Path) {
        write_file(
            root,
            "runtime-plugins/trusted-host/loong.plugin.json",
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "trusted-host-extension",
  "provider_id": "trusted-host-extension",
  "connector_name": "trusted-host-extension",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "javascript-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "javascript",
    "command": "node",
    "args_json": "[\"index.js\"]",
    "process_timeout_ms": "15000",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_family": "trusted_host_extension",
    "loong_extension_trust_lane": "trusted_host",
    "loong_extension_methods_json": "[\"extension/event\"]",
    "loong_extension_host_hooks_json": "[\"turn_start\"]"
  }
}"#,
        );
        write_file(
            root,
            "runtime-plugins/trusted-host/index.js",
            "#!/usr/bin/env node\nconst fs = require('fs');\nfunction emitResponse(line) { const trimmed = line.trim(); if (!trimmed) return; const request = JSON.parse(trimmed); const payload = request.payload ?? {}; const markerPath = payload.payload?.hook_payload?.metadata?.hook_marker_path ?? null; if (markerPath) { fs.writeFileSync(markerPath, payload.payload?.host_hook ?? 'unknown'); } const response = { method: request.method ?? '', id: request.id ?? null, payload: { handled_hook: payload.payload?.host_hook ?? null, turn_id: payload.payload?.hook_payload?.turn_id ?? null, session_hint: payload.payload?.hook_payload?.session_hint ?? null } }; process.stdout.write(`${JSON.stringify(response)}\\n`); } process.stdin.setEncoding('utf8'); let buffered=''; process.stdin.on('data', chunk => { buffered += chunk; let newlineIndex = buffered.indexOf('\\n'); while (newlineIndex !== -1) { const line = buffered.slice(0, newlineIndex); buffered = buffered.slice(newlineIndex + 1); emitResponse(line); newlineIndex = buffered.indexOf('\\n'); } }); process.stdin.on('end', () => { if (buffered.trim()) emitResponse(buffered); }); process.stdin.resume();\n",
        );
    }

    fn runtime_plugins_test_config(root: &Path) -> mvp::config::LoongConfig {
        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["process_stdio".to_owned()];
        config.runtime_plugins.allowed_process_commands = vec!["node".to_owned()];
        config
    }

    #[tokio::test]
    async fn dispatch_trusted_host_hook_runs_ready_process_stdio_extension() {
        let root = unique_temp_dir("loong-daemon-trusted-host-hook-dispatch");
        install_trusted_host_runtime_plugin(&root);
        let config = runtime_plugins_test_config(&root);

        let results =
            dispatch_trusted_host_hook(&config, "turn_start", json!({"turn_id":"demo-turn"}))
                .await
                .expect("dispatch trusted host hook");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, "trusted-host-extension");
        assert_eq!(
            results[0].response_payload["handled_hook"],
            json!("turn_start")
        );
        assert_eq!(results[0].response_payload["turn_id"], json!("demo-turn"));
    }

    #[tokio::test]
    async fn dispatch_turn_start_hook_for_request_writes_marker_for_trusted_extension() {
        let root = unique_temp_dir("loong-daemon-trusted-host-turn-start");
        install_trusted_host_runtime_plugin(&root);
        let config = runtime_plugins_test_config(&root);
        let marker_path = root.join("turn-start-marker.txt");
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            "hook_marker_path".to_owned(),
            marker_path.display().to_string(),
        );

        dispatch_turn_start_hook_for_request(
            &config,
            Some("session-123"),
            &mvp::agent_runtime::AgentTurnRequest {
                message: "hello".to_owned(),
                turn_mode: mvp::agent_runtime::AgentTurnMode::Oneshot,
                metadata,
                ..Default::default()
            },
        )
        .await
        .expect("dispatch turn_start hook");

        let marker_contents =
            fs::read_to_string(&marker_path).expect("turn_start hook should write marker");
        assert_eq!(marker_contents, "turn_start");
    }
}
