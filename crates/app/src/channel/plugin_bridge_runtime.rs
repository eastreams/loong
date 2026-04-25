use std::collections::BTreeSet;

use loong_kernel::{
    PluginActivationStatus, PluginBridgeKind, PluginIR, PluginScanReport, PluginScanner,
    PluginSetupReadinessContext, PluginTranslationReport, PluginTranslator,
};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::CliResult;
use crate::config::{
    LoongConfig, ResolvedOnebotChannelConfig, ResolvedQqbotChannelConfig,
    ResolvedWeixinChannelConfig, ResolvedWhatsappPersonalChannelConfig,
};

use super::{ChannelPlatform, normalize_channel_catalog_id};

pub const CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1: &str = "loong_channel_bridge_v1";
pub const CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION: &str = "send_message";
pub const CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION: &str = "receive_batch";
pub const CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION: &str = "ack_inbound";
pub const CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION: &str = "complete_batch";
const DEFAULT_PROCESS_STDIO_ENTRYPOINT_HINT: &str = "stdin/stdout::invoke";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManagedPluginBridgeRuntimeBinding {
    pub channel_id: String,
    pub platform: ChannelPlatform,
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account_id: String,
    pub account_label: String,
    pub plugin: PluginIR,
    pub endpoint: String,
    pub runtime_contract: String,
    pub runtime_operations: BTreeSet<String>,
    pub runtime_context: Value,
}

impl ManagedPluginBridgeRuntimeBinding {
    #[must_use]
    pub fn supports_operation(&self, operation: &str) -> bool {
        let normalized_operation = normalize_runtime_operation(operation);
        self.runtime_operations
            .contains(normalized_operation.as_str())
    }
}

pub fn resolve_managed_plugin_bridge_runtime_binding(
    config: &LoongConfig,
    raw_channel_id: &str,
    requested_account_id: Option<&str>,
) -> CliResult<ManagedPluginBridgeRuntimeBinding> {
    let normalized_channel_id = normalize_channel_catalog_id(raw_channel_id);
    let Some(channel_id) = normalized_channel_id else {
        return Err(format!(
            "managed bridge runtime does not support unknown channel `{raw_channel_id}`"
        ));
    };

    if !config.runtime_plugins.enabled {
        return Err(
            "managed bridge runtime is disabled; set [runtime_plugins].enabled = true".to_owned(),
        );
    }

    let resolved_roots = config.runtime_plugins.resolved_roots();
    if resolved_roots.is_empty() {
        return Err(
            "managed bridge runtime is enabled but runtime plugin roots are empty".to_owned(),
        );
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

    let resolved_account = resolve_runtime_account(config, channel_id, requested_account_id)?;
    let runtime_candidates = collect_runtime_candidates(
        channel_id,
        &translation,
        &activation,
        requested_account_id,
        config,
    );
    let runnable_candidates =
        filter_runnable_runtime_candidates(config, &resolved_account, &runtime_candidates);
    if runnable_candidates.candidates.is_empty() && !runnable_candidates.failure_reasons.is_empty()
    {
        let failure_reasons = runnable_candidates.failure_reasons.join("; ");
        return Err(failure_reasons);
    }
    let selected_candidate = select_runtime_candidate(
        channel_id,
        &runnable_candidates.candidates,
        configured_plugin_id(config, channel_id),
    )?;
    let endpoint = resolved_runtime_endpoint(selected_candidate.plugin, &resolved_account)?;
    let runtime_operations = normalized_runtime_operations(selected_candidate.plugin);
    let runtime_contract = resolved_runtime_contract(selected_candidate.plugin)?;
    validate_binding_execution_requirements(config, selected_candidate.plugin)?;
    let runtime_context = build_runtime_context(channel_id, &resolved_account);

    Ok(ManagedPluginBridgeRuntimeBinding {
        channel_id: channel_id.to_owned(),
        platform: selected_candidate.platform,
        configured_account_id: resolved_account.configured_account_id,
        configured_account_label: resolved_account.configured_account_label,
        account_id: resolved_account.account_id,
        account_label: resolved_account.account_label,
        plugin: selected_candidate.plugin.clone(),
        endpoint,
        runtime_contract,
        runtime_operations,
        runtime_context,
    })
}

#[derive(Debug, Clone)]
struct ManagedPluginBridgeRuntimeCandidate<'a> {
    platform: ChannelPlatform,
    plugin: &'a PluginIR,
}

#[derive(Debug, Clone)]
struct ManagedPluginBridgeResolvedAccount {
    configured_account_id: String,
    configured_account_label: String,
    account_id: String,
    account_label: String,
    endpoint_override: Option<String>,
    runtime_context: Value,
}

#[derive(Debug, Clone)]
struct ManagedPluginBridgeRunnableCandidates<'a> {
    candidates: Vec<ManagedPluginBridgeRuntimeCandidate<'a>>,
    failure_reasons: Vec<String>,
}

fn scan_runtime_plugin_roots(roots: &[std::path::PathBuf]) -> CliResult<PluginScanReport> {
    let scanner = PluginScanner::new();
    let mut combined_report = PluginScanReport::default();

    for root in roots {
        let root_report = scanner.scan_path(root).map_err(|error| {
            format!("runtime plugin scan failed for {}: {error}", root.display())
        })?;
        merge_plugin_scan_report(&mut combined_report, root_report);
    }

    Ok(combined_report)
}

fn merge_plugin_scan_report(target: &mut PluginScanReport, source: PluginScanReport) {
    target.scanned_files = target.scanned_files.saturating_add(source.scanned_files);
    target.matched_plugins = target
        .matched_plugins
        .saturating_add(source.matched_plugins);

    for descriptor in source.descriptors {
        target.descriptors.push(descriptor);
    }
}

fn runtime_plugin_setup_readiness_context(
    config: &LoongConfig,
) -> CliResult<PluginSetupReadinessContext> {
    let mut verified_env_vars = BTreeSet::new();

    for (key, value) in std::env::vars_os() {
        let value_string = value.to_string_lossy();
        let trimmed_value = value_string.trim();
        if trimmed_value.is_empty() {
            continue;
        }

        let key_string = key.to_string_lossy();
        let verified_env_var = key_string.to_string();
        verified_env_vars.insert(verified_env_var);
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
        Value::Null => {}
        Value::Bool(_) => {}
        Value::Number(_) => {}
        Value::String(_) => {}
    }
}

fn collect_runtime_candidates<'a>(
    channel_id: &str,
    translation: &'a PluginTranslationReport,
    activation: &'a loong_kernel::PluginActivationPlan,
    _requested_account_id: Option<&str>,
    _config: &LoongConfig,
) -> Vec<ManagedPluginBridgeRuntimeCandidate<'a>> {
    let mut candidates = Vec::new();

    for plugin in &translation.entries {
        let Some(channel_bridge) = plugin.channel_bridge.as_ref() else {
            continue;
        };

        let bridge_channel_id = channel_bridge.channel_id.as_deref();
        if bridge_channel_id != Some(channel_id) {
            continue;
        }

        if !channel_bridge.readiness.ready {
            continue;
        }

        let activation_candidate = activation.candidate_for(&plugin.source_path, &plugin.plugin_id);
        let Some(activation_candidate) = activation_candidate else {
            continue;
        };

        let activation_status = activation_candidate.status;
        if activation_status != PluginActivationStatus::Ready {
            continue;
        }

        let platform = platform_for_channel_id(channel_id);
        let Some(platform) = platform else {
            continue;
        };

        candidates.push(ManagedPluginBridgeRuntimeCandidate { platform, plugin });
    }

    candidates
}

fn filter_runnable_runtime_candidates<'a>(
    config: &LoongConfig,
    resolved_account: &ManagedPluginBridgeResolvedAccount,
    candidates: &'a [ManagedPluginBridgeRuntimeCandidate<'a>],
) -> ManagedPluginBridgeRunnableCandidates<'a> {
    let mut runnable_candidates = Vec::new();
    let mut failure_reasons = Vec::new();

    for candidate in candidates {
        let endpoint_result = resolved_runtime_endpoint(candidate.plugin, resolved_account);
        let endpoint = match endpoint_result {
            Ok(endpoint) => endpoint,
            Err(error) => {
                failure_reasons.push(error);
                continue;
            }
        };

        let runtime_contract_result = resolved_runtime_contract(candidate.plugin);
        let runtime_contract = match runtime_contract_result {
            Ok(runtime_contract) => runtime_contract,
            Err(error) => {
                failure_reasons.push(error);
                continue;
            }
        };

        let requirements_result = validate_binding_execution_requirements(config, candidate.plugin);
        if let Err(error) = requirements_result {
            failure_reasons.push(error);
            continue;
        }

        let _ = endpoint;
        let _ = runtime_contract;
        runnable_candidates.push(candidate.clone());
    }

    ManagedPluginBridgeRunnableCandidates {
        candidates: runnable_candidates,
        failure_reasons,
    }
}

fn select_runtime_candidate<'a>(
    channel_id: &str,
    candidates: &'a [ManagedPluginBridgeRuntimeCandidate<'a>],
    configured_plugin_id: Option<String>,
) -> CliResult<&'a ManagedPluginBridgeRuntimeCandidate<'a>> {
    if let Some(configured_plugin_id) = configured_plugin_id {
        let mut matching_candidates = Vec::new();

        for candidate in candidates {
            if candidate.plugin.plugin_id == configured_plugin_id {
                matching_candidates.push(candidate);
            }
        }

        if matching_candidates.is_empty() {
            return Err(format!(
                "managed bridge runtime for {channel_id} could not find configured managed_bridge_plugin_id={configured_plugin_id} among execution-ready plugins"
            ));
        }

        if matching_candidates.len() > 1 {
            return Err(format!(
                "managed bridge runtime for {channel_id} found duplicate execution-ready plugins for managed_bridge_plugin_id={configured_plugin_id}"
            ));
        }

        let selected_candidate = matching_candidates
            .first()
            .copied()
            .ok_or_else(|| {
                format!(
                    "managed bridge runtime for {channel_id} could not resolve configured managed_bridge_plugin_id={configured_plugin_id}"
                )
            })?;
        return Ok(selected_candidate);
    }

    if candidates.is_empty() {
        return Err(format!(
            "managed bridge runtime for {channel_id} found no execution-ready plugins"
        ));
    }

    if candidates.len() > 1 {
        let mut plugin_ids = Vec::new();

        for candidate in candidates {
            let plugin_id = candidate.plugin.plugin_id.clone();
            plugin_ids.push(plugin_id);
        }

        let rendered_plugin_ids = plugin_ids.join(",");
        return Err(format!(
            "managed bridge runtime for {channel_id} is ambiguous; execution-ready plugins={rendered_plugin_ids}"
        ));
    }

    let selected_candidate = candidates.first().ok_or_else(|| {
        format!("managed bridge runtime for {channel_id} found no execution-ready plugins")
    })?;
    Ok(selected_candidate)
}

fn configured_plugin_id(config: &LoongConfig, channel_id: &str) -> Option<String> {
    let raw_plugin_id = match channel_id {
        "weixin" => config.weixin.managed_bridge_plugin_id.as_deref(),
        "qqbot" => config.qqbot.managed_bridge_plugin_id.as_deref(),
        "onebot" => config.onebot.managed_bridge_plugin_id.as_deref(),
        "whatsapp-personal" => config.whatsapp_personal.managed_bridge_plugin_id.as_deref(),
        _ => None,
    };

    let raw_plugin_id = raw_plugin_id?;

    let trimmed_plugin_id = raw_plugin_id.trim();
    if trimmed_plugin_id.is_empty() {
        return None;
    }

    Some(trimmed_plugin_id.to_owned())
}

fn resolve_runtime_account(
    config: &LoongConfig,
    channel_id: &str,
    requested_account_id: Option<&str>,
) -> CliResult<ManagedPluginBridgeResolvedAccount> {
    match channel_id {
        "weixin" => {
            let resolved = config.weixin.resolve_account(requested_account_id)?;
            Ok(resolve_weixin_account(resolved))
        }
        "qqbot" => {
            let resolved = config.qqbot.resolve_account(requested_account_id)?;
            Ok(resolve_qqbot_account(resolved))
        }
        "onebot" => {
            let resolved = config.onebot.resolve_account(requested_account_id)?;
            Ok(resolve_onebot_account(resolved))
        }
        "whatsapp-personal" => {
            let resolved = config
                .whatsapp_personal
                .resolve_account(requested_account_id)?;
            Ok(resolve_whatsapp_personal_account(resolved))
        }
        _ => Err(format!(
            "managed bridge runtime does not support channel `{channel_id}`"
        )),
    }
}

fn resolve_weixin_account(
    resolved: ResolvedWeixinChannelConfig,
) -> ManagedPluginBridgeResolvedAccount {
    let mut config_map = Map::new();
    let bridge_url = resolved.bridge_url();
    if let Some(bridge_url) = bridge_url.clone() {
        config_map.insert("bridge_url".to_owned(), Value::String(bridge_url));
    }

    let bridge_access_token = resolved.bridge_access_token();
    if let Some(bridge_access_token) = bridge_access_token {
        config_map.insert(
            "bridge_access_token".to_owned(),
            Value::String(bridge_access_token),
        );
    }

    let allowed_contact_ids =
        serde_json::to_value(&resolved.allowed_contact_ids).unwrap_or(Value::Array(Vec::new()));
    config_map.insert("allowed_contact_ids".to_owned(), allowed_contact_ids);

    let runtime_context = build_channel_runtime_context(
        &resolved.configured_account_id,
        &resolved.configured_account_label,
        &resolved.account.id,
        &resolved.account.label,
        Value::Object(config_map),
    );

    ManagedPluginBridgeResolvedAccount {
        configured_account_id: resolved.configured_account_id,
        configured_account_label: resolved.configured_account_label,
        account_id: resolved.account.id,
        account_label: resolved.account.label,
        endpoint_override: bridge_url,
        runtime_context,
    }
}

fn resolve_qqbot_account(
    resolved: ResolvedQqbotChannelConfig,
) -> ManagedPluginBridgeResolvedAccount {
    let mut config_map = Map::new();
    let app_id = resolved.app_id();
    if let Some(app_id) = app_id {
        config_map.insert("app_id".to_owned(), Value::String(app_id));
    }

    let client_secret = resolved.client_secret();
    if let Some(client_secret) = client_secret {
        config_map.insert("client_secret".to_owned(), Value::String(client_secret));
    }

    let allowed_peer_ids =
        serde_json::to_value(&resolved.allowed_peer_ids).unwrap_or(Value::Array(Vec::new()));
    config_map.insert("allowed_peer_ids".to_owned(), allowed_peer_ids);

    let runtime_context = build_channel_runtime_context(
        &resolved.configured_account_id,
        &resolved.configured_account_label,
        &resolved.account.id,
        &resolved.account.label,
        Value::Object(config_map),
    );

    ManagedPluginBridgeResolvedAccount {
        configured_account_id: resolved.configured_account_id,
        configured_account_label: resolved.configured_account_label,
        account_id: resolved.account.id,
        account_label: resolved.account.label,
        endpoint_override: None,
        runtime_context,
    }
}

fn resolve_onebot_account(
    resolved: ResolvedOnebotChannelConfig,
) -> ManagedPluginBridgeResolvedAccount {
    let mut config_map = Map::new();
    let websocket_url = resolved.websocket_url();
    if let Some(websocket_url) = websocket_url.clone() {
        config_map.insert("websocket_url".to_owned(), Value::String(websocket_url));
    }

    let access_token = resolved.access_token();
    if let Some(access_token) = access_token {
        config_map.insert("access_token".to_owned(), Value::String(access_token));
    }

    let allowed_group_ids =
        serde_json::to_value(&resolved.allowed_group_ids).unwrap_or(Value::Array(Vec::new()));
    config_map.insert("allowed_group_ids".to_owned(), allowed_group_ids);

    let runtime_context = build_channel_runtime_context(
        &resolved.configured_account_id,
        &resolved.configured_account_label,
        &resolved.account.id,
        &resolved.account.label,
        Value::Object(config_map),
    );

    ManagedPluginBridgeResolvedAccount {
        configured_account_id: resolved.configured_account_id,
        configured_account_label: resolved.configured_account_label,
        account_id: resolved.account.id,
        account_label: resolved.account.label,
        endpoint_override: websocket_url,
        runtime_context,
    }
}

fn resolve_whatsapp_personal_account(
    resolved: ResolvedWhatsappPersonalChannelConfig,
) -> ManagedPluginBridgeResolvedAccount {
    let mut config_map = Map::new();
    let bridge_url = resolved.bridge_url();
    if let Some(bridge_url) = bridge_url.clone() {
        config_map.insert("bridge_url".to_owned(), Value::String(bridge_url));
    }

    let auth_dir = resolved.auth_dir();
    if let Some(auth_dir) = auth_dir {
        config_map.insert("auth_dir".to_owned(), Value::String(auth_dir));
    }

    let allowed_chat_ids =
        serde_json::to_value(&resolved.allowed_chat_ids).unwrap_or(Value::Array(Vec::new()));
    config_map.insert("allowed_chat_ids".to_owned(), allowed_chat_ids);

    let runtime_context = build_channel_runtime_context(
        &resolved.configured_account_id,
        &resolved.configured_account_label,
        &resolved.account.id,
        &resolved.account.label,
        Value::Object(config_map),
    );

    ManagedPluginBridgeResolvedAccount {
        configured_account_id: resolved.configured_account_id,
        configured_account_label: resolved.configured_account_label,
        account_id: resolved.account.id,
        account_label: resolved.account.label,
        endpoint_override: bridge_url,
        runtime_context,
    }
}

fn build_channel_runtime_context(
    configured_account_id: &str,
    configured_account_label: &str,
    account_id: &str,
    account_label: &str,
    config: Value,
) -> Value {
    let mut context_map = Map::new();
    context_map.insert(
        "configured_account_id".to_owned(),
        Value::String(configured_account_id.to_owned()),
    );
    context_map.insert(
        "configured_account_label".to_owned(),
        Value::String(configured_account_label.to_owned()),
    );
    context_map.insert(
        "account_id".to_owned(),
        Value::String(account_id.to_owned()),
    );
    context_map.insert(
        "account_label".to_owned(),
        Value::String(account_label.to_owned()),
    );
    context_map.insert("config".to_owned(), config);

    Value::Object(context_map)
}

fn build_runtime_context(channel_id: &str, account: &ManagedPluginBridgeResolvedAccount) -> Value {
    let mut runtime_context_map = Map::new();
    runtime_context_map.insert(
        "channel_id".to_owned(),
        Value::String(channel_id.to_owned()),
    );
    runtime_context_map.insert("account".to_owned(), account.runtime_context.clone());

    Value::Object(runtime_context_map)
}

fn resolved_runtime_endpoint(
    plugin: &PluginIR,
    account: &ManagedPluginBridgeResolvedAccount,
) -> CliResult<String> {
    if let Some(endpoint_override) = account.endpoint_override.as_ref() {
        let trimmed_endpoint_override = endpoint_override.trim();
        if !trimmed_endpoint_override.is_empty() {
            return Ok(trimmed_endpoint_override.to_owned());
        }
    }

    if let Some(endpoint) = plugin.endpoint.as_ref() {
        let trimmed_endpoint = endpoint.trim();
        if !trimmed_endpoint.is_empty() {
            return Ok(trimmed_endpoint.to_owned());
        }
    }

    let entrypoint_hint = plugin.runtime.entrypoint_hint.trim();
    if !entrypoint_hint.is_empty() {
        return Ok(entrypoint_hint.to_owned());
    }

    Err(format!(
        "managed bridge runtime plugin {} has no usable endpoint or entrypoint hint",
        plugin.plugin_id
    ))
}

fn resolved_runtime_contract(plugin: &PluginIR) -> CliResult<String> {
    let Some(channel_bridge) = plugin.channel_bridge.as_ref() else {
        return Err(format!(
            "managed bridge runtime plugin {} is missing channel bridge metadata",
            plugin.plugin_id
        ));
    };

    let Some(runtime_contract) = channel_bridge.runtime_contract.as_ref() else {
        return Err(format!(
            "managed bridge runtime plugin {} does not declare channel_runtime_contract",
            plugin.plugin_id
        ));
    };

    let trimmed_runtime_contract = runtime_contract.trim();
    if trimmed_runtime_contract != CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1 {
        return Err(format!(
            "managed bridge runtime plugin {} declares unsupported channel runtime contract `{trimmed_runtime_contract}`",
            plugin.plugin_id
        ));
    }

    Ok(trimmed_runtime_contract.to_owned())
}

fn normalized_runtime_operations(plugin: &PluginIR) -> BTreeSet<String> {
    let mut normalized_operations = BTreeSet::new();
    let Some(channel_bridge) = plugin.channel_bridge.as_ref() else {
        return normalized_operations;
    };

    for runtime_operation in &channel_bridge.runtime_operations {
        let normalized_operation = normalize_runtime_operation(runtime_operation);
        if normalized_operation.is_empty() {
            continue;
        }

        normalized_operations.insert(normalized_operation);
    }

    normalized_operations
}

fn normalize_runtime_operation(operation: &str) -> String {
    let trimmed_operation = operation.trim();
    let lowercase_operation = trimmed_operation.to_ascii_lowercase();
    lowercase_operation.replace('-', "_")
}

fn platform_for_channel_id(channel_id: &str) -> Option<ChannelPlatform> {
    match channel_id {
        "weixin" => Some(ChannelPlatform::Weixin),
        "qqbot" => Some(ChannelPlatform::Qqbot),
        "onebot" => Some(ChannelPlatform::Onebot),
        "whatsapp-personal" => Some(ChannelPlatform::WhatsApp),
        _ => None,
    }
}

fn validate_binding_execution_requirements(
    config: &LoongConfig,
    plugin: &PluginIR,
) -> CliResult<()> {
    let bridge_kind = plugin.runtime.bridge_kind;
    if bridge_kind != PluginBridgeKind::ProcessStdio {
        return Ok(());
    }

    let command = resolved_process_stdio_command(plugin);
    let Some(command) = command else {
        return Err(format!(
            "managed bridge runtime plugin {} requires provider metadata.command or metadata.entrypoint for process_stdio execution",
            plugin.plugin_id
        ));
    };

    let normalized_allowed_commands = config.runtime_plugins.normalized_allowed_process_commands();
    let command_is_allowed =
        process_command_is_allowed(command.as_str(), &normalized_allowed_commands);
    if command_is_allowed {
        return Ok(());
    }

    Err(format!(
        "managed bridge runtime plugin {} uses process command `{}` that is not allowlisted in runtime_plugins.allowed_process_commands",
        plugin.plugin_id, command,
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
    if runtime_entrypoint.is_empty() {
        return None;
    }
    if runtime_entrypoint == DEFAULT_PROCESS_STDIO_ENTRYPOINT_HINT {
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
    let direct_match = allowed_commands.contains(&normalized_command);
    if direct_match {
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

    let file_name = command_path.file_name();
    let file_name = file_name.and_then(|name| name.to_str());
    let Some(file_name) = file_name else {
        return false;
    };

    let normalized_file_name = file_name.to_ascii_lowercase();
    allowed_commands.contains(&normalized_file_name)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    fn sample_manifest(
        plugin_id: &str,
        channel_id: &str,
        bridge_kind: &str,
        runtime_operations: Vec<&str>,
    ) -> loong_kernel::PluginManifest {
        let runtime_operations = runtime_operations
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let runtime_operations_json =
            serde_json::to_string(&runtime_operations).expect("serialize runtime operations");
        let metadata = BTreeMap::from([
            ("bridge_kind".to_owned(), bridge_kind.to_owned()),
            ("adapter_family".to_owned(), "channel-bridge".to_owned()),
            (
                "transport_family".to_owned(),
                "wechat_clawbot_ilink_bridge".to_owned(),
            ),
            ("target_contract".to_owned(), "weixin_reply_loop".to_owned()),
            (
                "channel_runtime_contract".to_owned(),
                CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1.to_owned(),
            ),
            (
                "channel_runtime_operations_json".to_owned(),
                runtime_operations_json,
            ),
            ("command".to_owned(), "node".to_owned()),
        ]);

        loong_kernel::PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("1.0.0".to_owned()),
            plugin_id: plugin_id.to_owned(),
            provider_id: format!("{plugin_id}-provider"),
            connector_name: format!("{plugin_id}-connector"),
            channel_id: Some(channel_id.to_owned()),
            endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
            capabilities: BTreeSet::new(),
            trust_tier: loong_kernel::PluginTrustTier::Unverified,
            metadata,
            summary: None,
            tags: Vec::new(),
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup: Some(loong_kernel::PluginSetup {
                mode: loong_kernel::PluginSetupMode::MetadataOnly,
                surface: Some("channel".to_owned()),
                required_env_vars: Vec::new(),
                recommended_env_vars: Vec::new(),
                required_config_keys: Vec::new(),
                default_env_var: None,
                docs_urls: Vec::new(),
                remediation: None,
            }),
            slot_claims: Vec::new(),
            compatibility: None,
        }
    }

    fn write_manifest(root: &Path, directory_name: &str, manifest: &loong_kernel::PluginManifest) {
        let plugin_directory = root.join(directory_name);
        let manifest_path = plugin_directory.join("loong.plugin.json");
        let encoded_manifest =
            serde_json::to_string_pretty(manifest).expect("serialize plugin manifest");

        fs::create_dir_all(&plugin_directory).expect("create plugin directory");
        fs::write(&manifest_path, encoded_manifest).expect("write plugin manifest");
    }

    #[test]
    fn resolve_managed_bridge_runtime_binding_uses_selected_ready_plugin() {
        let root = TempDir::new().expect("create runtime plugin root");
        let manifest = sample_manifest(
            "weixin-bridge-runtime",
            "weixin",
            "http_json",
            vec![
                CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
            ],
        );
        write_manifest(root.path(), "weixin-bridge-runtime", &manifest);

        let mut config = LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "bridge-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let binding = resolve_managed_plugin_bridge_runtime_binding(&config, "weixin", None)
            .expect("resolve managed bridge runtime binding");

        assert_eq!(binding.plugin.plugin_id, "weixin-bridge-runtime");
        assert_eq!(binding.platform, ChannelPlatform::Weixin);
        assert_eq!(
            binding.endpoint,
            "https://bridge.example.test/weixin".to_owned()
        );
        assert!(binding.supports_operation(CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION));
        assert!(binding.supports_operation(CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION));
    }

    #[test]
    fn resolve_managed_bridge_runtime_binding_requires_allowlisted_process_command() {
        let root = TempDir::new().expect("create runtime plugin root");
        let manifest = sample_manifest(
            "qqbot-bridge-runtime",
            "qqbot",
            "process_stdio",
            vec![CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION],
        );
        write_manifest(root.path(), "qqbot-bridge-runtime", &manifest);

        let mut config = LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["process_stdio".to_owned()];
        config.qqbot.enabled = true;
        config.qqbot.app_id = Some(loong_contracts::SecretRef::Inline("10001".to_owned()));
        config.qqbot.client_secret = Some(loong_contracts::SecretRef::Inline(
            "client-secret".to_owned(),
        ));

        let error = resolve_managed_plugin_bridge_runtime_binding(&config, "qqbot", None)
            .expect_err("process_stdio binding should require allowlisted command");

        assert!(error.contains("runtime_plugins.allowed_process_commands"));
    }

    #[test]
    fn resolve_managed_bridge_runtime_binding_accepts_entrypoint_when_command_is_missing() {
        let root = TempDir::new().expect("create runtime plugin root");
        let mut manifest = sample_manifest(
            "qqbot-bridge-runtime",
            "qqbot",
            "process_stdio",
            vec![CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION],
        );
        manifest.metadata.remove("command");
        manifest
            .metadata
            .insert("entrypoint".to_owned(), "node".to_owned());
        write_manifest(root.path(), "qqbot-bridge-runtime", &manifest);

        let mut config = LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["process_stdio".to_owned()];
        config.runtime_plugins.allowed_process_commands = vec!["node".to_owned()];
        config.qqbot.enabled = true;
        config.qqbot.app_id = Some(loong_contracts::SecretRef::Inline("10001".to_owned()));
        config.qqbot.client_secret = Some(loong_contracts::SecretRef::Inline(
            "client-secret".to_owned(),
        ));

        let binding = resolve_managed_plugin_bridge_runtime_binding(&config, "qqbot", None)
            .expect("entrypoint-backed process bridge should resolve");

        assert_eq!(binding.plugin.plugin_id, "qqbot-bridge-runtime");
    }

    #[test]
    fn process_command_is_allowed_rejects_path_spoofing() {
        let allowed_commands = vec!["python3".to_owned()];

        assert!(process_command_is_allowed("python3", &allowed_commands));
        assert!(!process_command_is_allowed(
            "/tmp/python3",
            &allowed_commands
        ));
        assert!(!process_command_is_allowed("./python3", &allowed_commands));
    }
}
