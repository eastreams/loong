use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use loong_bridge_runtime::BridgeExecutionPolicy;
use loong_bridge_runtime::execute_http_json_bridge_call;
use loong_bridge_runtime::execute_process_stdio_bridge_call;
use loong_contracts::Capability;
use loong_spec::CliResult;
use serde_json::{Map, Value};

use crate::mvp;
use crate::mvp::channel::ChannelAdapter;
use crate::{ChannelCliCommandFuture, ChannelSendCliArgs, ChannelServeCliArgs};

struct ManagedBridgeInvocationSuccess {
    response_payload: Value,
    runtime_evidence: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedManagedBridgeTarget {
    embedded_account_id: Option<String>,
    route_kind: String,
    route_id: String,
}

#[cfg(test)]
const MANAGED_BRIDGE_IDLE_POLL_MS: u64 = 25;
#[cfg(not(test))]
const MANAGED_BRIDGE_IDLE_POLL_MS: u64 = 500;
const MANAGED_BRIDGE_SERVE_MAX_CONSECUTIVE_FAILURES: usize = 3;
#[cfg(test)]
const MANAGED_BRIDGE_SERVE_INITIAL_BACKOFF_MS: u64 = 25;
#[cfg(not(test))]
const MANAGED_BRIDGE_SERVE_INITIAL_BACKOFF_MS: u64 = 1_000;
#[cfg(test)]
const MANAGED_BRIDGE_SERVE_MAX_BACKOFF_MS: u64 = 100;
#[cfg(not(test))]
const MANAGED_BRIDGE_SERVE_MAX_BACKOFF_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy)]
struct ManagedBridgeServeContext<'a> {
    channel_id: &'a str,
    plugin_id: &'a str,
    configured_account_id: &'a str,
}

pub async fn run_managed_plugin_bridge_send(
    config_path: Option<&str>,
    channel_id: &str,
    account_id: Option<&str>,
    target: &str,
    target_kind: mvp::channel::ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    let (resolved_path, config) = load_managed_bridge_runtime_config(config_path)?;
    let parsed_target = parse_managed_bridge_target(channel_id, target)?;
    let resolved_account_id = account_id
        .map(normalize_bridge_account_id)
        .or_else(|| parsed_target.embedded_account_id.clone());
    let binding = mvp::channel::resolve_managed_plugin_bridge_runtime_binding(
        &config,
        channel_id,
        resolved_account_id.as_deref(),
    )?;
    let supports_send = binding
        .supports_operation(mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION);
    if !supports_send {
        return Err(format!(
            "managed bridge runtime plugin {} does not support send_message",
            binding.plugin.plugin_id
        ));
    }

    mvp::runtime_env::initialize_runtime_environment(&config, Some(resolved_path.as_path()));

    let bridge_policy = bridge_execution_policy_from_config(&config)?;
    let canonical_target =
        canonicalize_managed_bridge_target(channel_id, &binding, &parsed_target)?;
    enforce_managed_bridge_outbound_policy(channel_id, &binding, &parsed_target)?;
    let outbound_target = mvp::channel::ChannelOutboundTarget::new(
        target_platform(channel_id)?,
        target_kind,
        canonical_target.as_str(),
    );
    let outbound_message = mvp::channel::ChannelOutboundMessage::Text(text.to_owned());
    let payload = send_message_payload(&binding, &outbound_target, &outbound_message);
    let invocation =
        invoke_managed_bridge_operation(&binding, &bridge_policy, "send_message", payload).await?;
    let runtime_evidence_is_null = invocation.runtime_evidence.is_null();
    if !runtime_evidence_is_null {
        tracing::debug!(
            target: "loong.managed_bridge",
            channel_id,
            plugin_id = %binding.plugin.plugin_id,
            bridge_kind = %binding.plugin.runtime.bridge_kind.as_str(),
            "managed bridge send completed with runtime evidence"
        );
    }

    #[allow(clippy::print_stdout)]
    {
        println!(
            "{} message sent via managed bridge runtime (plugin_id={}, configured_account={}, account={}, target_kind={}, target={}, route_kind={})",
            channel_id,
            binding.plugin.plugin_id,
            binding.configured_account_id,
            binding.account_label,
            target_kind.as_str(),
            canonical_target,
            parsed_target.route_kind,
        );
    }

    Ok(())
}

pub async fn run_managed_plugin_bridge_channel(
    config_path: Option<&str>,
    channel_id: &str,
    account_id: Option<&str>,
    once: bool,
) -> CliResult<()> {
    let (resolved_path, config) = load_managed_bridge_runtime_config(config_path)?;
    let binding = mvp::channel::resolve_managed_plugin_bridge_runtime_binding(
        &config, channel_id, account_id,
    )?;
    let supports_receive = binding
        .supports_operation(mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION);
    if !supports_receive {
        return Err(format!(
            "managed bridge runtime plugin {} does not support receive_batch",
            binding.plugin.plugin_id
        ));
    }

    let supports_send = binding
        .supports_operation(mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION);
    if !supports_send {
        return Err(format!(
            "managed bridge runtime plugin {} does not support send_message",
            binding.plugin.plugin_id
        ));
    }

    mvp::runtime_env::initialize_runtime_environment(&config, Some(resolved_path.as_path()));

    let kernel_scope = format!("channel-plugin-bridge-{channel_id}");
    let kernel_ctx = mvp::context::bootstrap_kernel_context_with_config(
        kernel_scope.as_str(),
        mvp::context::DEFAULT_TOKEN_TTL_S,
        &config,
    )?;
    let bridge_policy = bridge_execution_policy_from_config(&config)?;
    let stop = mvp::channel::ChannelServeStopHandle::new();
    let runtime_account_id = binding.account_id.clone();
    let runtime_account_label = binding.account_label.clone();
    let selected_plugin_id = binding.plugin.plugin_id.clone();
    let selected_bridge_kind = binding.plugin.runtime.bridge_kind.as_str().to_owned();
    let configured_account_id = binding.configured_account_id.clone();
    let configured_account_label = binding.configured_account_label.clone();
    let endpoint = binding.endpoint.clone();
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{} bridge serve starting (plugin_id={}, bridge_kind={}, configured_account={}, account={}, once={}, endpoint={})",
            channel_id,
            selected_plugin_id,
            selected_bridge_kind,
            configured_account_id,
            configured_account_label,
            once,
            endpoint,
        );
    }
    let config = Arc::new(config);
    let resolved_path = Some(resolved_path);
    let kernel_ctx = Arc::new(kernel_ctx);
    let runtime_spec = mvp::channel::ChannelServeRuntimeSpec {
        platform: binding.platform,
        operation_id: mvp::channel::CHANNEL_OPERATION_SERVE_ID,
        account_id: runtime_account_id.as_str(),
        account_label: runtime_account_label.as_str(),
    };

    mvp::channel::with_channel_serve_runtime_with_stop(
        runtime_spec,
        stop,
        move |runtime, stop| async move {
            let mut adapter = ManagedPluginBridgeChannelAdapter::new(binding, bridge_policy);
            let serve_context = ManagedBridgeServeContext {
                channel_id,
                plugin_id: selected_plugin_id.as_str(),
                configured_account_id: configured_account_id.as_str(),
            };
            run_managed_plugin_bridge_loop(
                &stop,
                &runtime,
                &mut adapter,
                once,
                serve_context,
                |message, feedback_policy| {
                    let config = config.clone();
                    let resolved_path = resolved_path.clone();
                    let kernel_ctx = kernel_ctx.clone();
                    Box::pin(async move {
                        let resolved_path = resolved_path.as_deref();
                        mvp::channel::process_inbound_with_provider(
                            config.as_ref(),
                            resolved_path,
                            &message,
                            kernel_ctx.as_ref(),
                            feedback_policy,
                        )
                        .await
                    })
                },
            )
            .await
        },
    )
    .await
}

async fn request_managed_plugin_bridge_serve_stop(
    config_path: Option<&str>,
    channel_id: &str,
    account_id: Option<&str>,
) -> CliResult<()> {
    let (_resolved_path, config) = load_managed_bridge_runtime_config(config_path)?;
    let binding = mvp::channel::resolve_managed_plugin_bridge_runtime_binding(
        &config, channel_id, account_id,
    )?;
    let outcome = mvp::channel::request_channel_operation_stop(
        binding.platform,
        mvp::channel::CHANNEL_OPERATION_SERVE_ID,
        Some(binding.account_id.as_str()),
    )?;

    let outcome_label = match outcome {
        mvp::channel::ChannelOperationStopRequestOutcome::Requested => "requested",
        mvp::channel::ChannelOperationStopRequestOutcome::AlreadyRequested => "already_requested",
        mvp::channel::ChannelOperationStopRequestOutcome::AlreadyStopped => "already_stopped",
    };
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{} bridge serve stop {} (plugin_id={}, configured_account={}, account={})",
            channel_id,
            outcome_label,
            binding.plugin.plugin_id,
            binding.configured_account_id,
            binding.account_label,
        );
    }

    Ok(())
}

async fn request_managed_plugin_bridge_serve_duplicate_cleanup(
    config_path: Option<&str>,
    channel_id: &str,
    account_id: Option<&str>,
) -> CliResult<()> {
    let (_resolved_path, config) = load_managed_bridge_runtime_config(config_path)?;
    let binding = mvp::channel::resolve_managed_plugin_bridge_runtime_binding(
        &config, channel_id, account_id,
    )?;
    let result = mvp::channel::request_channel_operation_duplicate_cleanup(
        binding.platform,
        mvp::channel::CHANNEL_OPERATION_SERVE_ID,
        Some(binding.account_id.as_str()),
    )?;

    let outcome_label = match result.outcome {
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::Requested => "requested",
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::AlreadyRequested => {
            "already_requested"
        }
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::NoDuplicates => "no_duplicates",
        mvp::channel::ChannelOperationDuplicateCleanupOutcome::AlreadyStopped => "already_stopped",
    };
    let preferred_owner_pid = result
        .preferred_owner_pid
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let cleanup_owner_pids = if result.targeted_owner_pids.is_empty() {
        "-".to_owned()
    } else {
        result
            .targeted_owner_pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    #[allow(clippy::print_stdout)]
    {
        println!(
            "{} bridge serve duplicate cleanup {} (plugin_id={}, configured_account={}, account={}, preferred_owner_pid={}, cleanup_owner_pids={})",
            channel_id,
            outcome_label,
            binding.plugin.plugin_id,
            binding.configured_account_id,
            binding.account_label,
            preferred_owner_pid,
            cleanup_owner_pids,
        );
    }

    Ok(())
}

fn load_managed_bridge_runtime_config(
    config_path: Option<&str>,
) -> CliResult<(PathBuf, mvp::config::LoongConfig)> {
    mvp::config::load(config_path)
}

fn bridge_execution_policy_from_config(
    config: &mvp::config::LoongConfig,
) -> CliResult<BridgeExecutionPolicy> {
    let supported_bridges = config
        .runtime_plugins
        .resolved_supported_bridges()
        .map_err(|error| format!("resolve runtime plugin bridge kinds failed: {error}"))?;
    let execute_http_json = supported_bridges.contains(&kernel::PluginBridgeKind::HttpJson);
    let execute_process_stdio = supported_bridges.contains(&kernel::PluginBridgeKind::ProcessStdio);
    let mut allowed_process_commands = BTreeSet::new();
    let normalized_allowed_commands = config.runtime_plugins.normalized_allowed_process_commands();

    for allowed_command in normalized_allowed_commands {
        allowed_process_commands.insert(allowed_command);
    }

    Ok(BridgeExecutionPolicy {
        execute_process_stdio,
        execute_http_json,
        allowed_process_commands,
    })
}

fn target_platform(channel_id: &str) -> CliResult<mvp::channel::ChannelPlatform> {
    match channel_id {
        "weixin" => Ok(mvp::channel::ChannelPlatform::Weixin),
        "qqbot" => Ok(mvp::channel::ChannelPlatform::Qqbot),
        "onebot" => Ok(mvp::channel::ChannelPlatform::Onebot),
        _ => Err(format!(
            "managed bridge runtime does not support channel `{channel_id}`"
        )),
    }
}

fn send_message_payload(
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
    target: &mvp::channel::ChannelOutboundTarget,
    message: &mvp::channel::ChannelOutboundMessage,
) -> Value {
    let target_value = serde_json::to_value(target).unwrap_or(Value::Null);
    let message_value = serde_json::to_value(message).unwrap_or(Value::Null);
    let mut payload_map = Map::new();
    payload_map.insert(
        "runtime_context".to_owned(),
        binding.runtime_context.clone(),
    );
    payload_map.insert("target".to_owned(), target_value);
    payload_map.insert("message".to_owned(), message_value);

    Value::Object(payload_map)
}

fn parse_managed_bridge_target(
    channel_id: &str,
    raw_target: &str,
) -> CliResult<ParsedManagedBridgeTarget> {
    let trimmed_target = raw_target.trim();
    if trimmed_target.is_empty() {
        return Err(format!("{channel_id}-send requires --target"));
    }

    let allowed_route_kinds = managed_bridge_route_kinds(channel_id);
    let parts = trimmed_target.split(':').collect::<Vec<_>>();
    let (embedded_account_id, route_kind, route_id) = match parts.as_slice() {
        [route_kind, route_id] => (None, *route_kind, *route_id),
        [account_id, route_kind, route_id] => (
            Some(normalize_bridge_account_id(account_id)),
            *route_kind,
            *route_id,
        ),
        [raw_channel_id, account_id, route_kind, route_id] => {
            let normalized_channel_id = mvp::channel::normalize_channel_catalog_id(raw_channel_id)
                .ok_or_else(|| {
                    format!(
                        "{channel_id} target prefix `{raw_channel_id}` is not a recognized channel id"
                    )
                })?;
            if normalized_channel_id != channel_id {
                return Err(format!(
                    "{channel_id} target uses channel prefix `{normalized_channel_id}`, expected `{channel_id}`"
                ));
            }
            (
                Some(normalize_bridge_account_id(account_id)),
                *route_kind,
                *route_id,
            )
        }
        _ => {
            return Err(format!(
                "{channel_id} target must use `{channel_id}:<account>:<kind>:<id>`, `<account>:<kind>:<id>`, or `<kind>:<id>`"
            ));
        }
    };

    let route_kind = allowed_route_kinds
        .iter()
        .find(|candidate| **candidate == route_kind)
        .copied()
        .ok_or_else(|| {
            format!(
                "{channel_id} target kind `{route_kind}` is unsupported; use {}",
                allowed_route_kinds
                    .iter()
                    .map(|value| format!("`{value}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    let route_id = route_id.trim();
    if route_id.is_empty() {
        return Err(format!("{channel_id} target conversation id is empty"));
    }

    Ok(ParsedManagedBridgeTarget {
        embedded_account_id,
        route_kind: route_kind.to_owned(),
        route_id: route_id.to_owned(),
    })
}

fn canonicalize_managed_bridge_target(
    channel_id: &str,
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
    parsed_target: &ParsedManagedBridgeTarget,
) -> CliResult<String> {
    if let Some(embedded_account_id) = parsed_target.embedded_account_id.as_deref()
        && embedded_account_id != binding.configured_account_id
    {
        return Err(format!(
            "{channel_id} target resolved account `{embedded_account_id}`, but the selected configured account is `{}`",
            binding.configured_account_id
        ));
    }

    Ok(format!(
        "{channel_id}:{}:{}:{}",
        binding.configured_account_id, parsed_target.route_kind, parsed_target.route_id
    ))
}

fn managed_bridge_route_kinds(channel_id: &str) -> &'static [&'static str] {
    match channel_id {
        "weixin" => &["contact", "room"],
        "qqbot" => &["c2c", "group", "channel"],
        "onebot" => &["private", "group"],
        _ => &[],
    }
}

fn enforce_managed_bridge_outbound_policy(
    channel_id: &str,
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
    parsed_target: &ParsedManagedBridgeTarget,
) -> CliResult<()> {
    match channel_id {
        "weixin" => {
            if parsed_target.route_kind != "contact" {
                return Ok(());
            }
            let allowed_contact_ids =
                runtime_context_string_list(&binding.runtime_context, "allowed_contact_ids");
            if route_id_is_allowed(
                allowed_contact_ids.as_slice(),
                parsed_target.route_id.as_str(),
            ) {
                return Ok(());
            }
            Err(format!(
                "weixin target `{}` is not allowed by configured allowed_contact_ids",
                parsed_target.route_id
            ))
        }
        "qqbot" => {
            let allowed_peer_ids =
                runtime_context_string_list(&binding.runtime_context, "allowed_peer_ids");
            if route_id_is_allowed(allowed_peer_ids.as_slice(), parsed_target.route_id.as_str()) {
                return Ok(());
            }
            Err(format!(
                "qqbot target `{}` is not allowed by configured allowed_peer_ids",
                parsed_target.route_id
            ))
        }
        "onebot" => {
            if parsed_target.route_kind != "group" {
                return Ok(());
            }
            let allowed_group_ids =
                runtime_context_string_list(&binding.runtime_context, "allowed_group_ids");
            if route_id_is_allowed(
                allowed_group_ids.as_slice(),
                parsed_target.route_id.as_str(),
            ) {
                return Ok(());
            }
            Err(format!(
                "onebot group target `{}` is not allowed by configured allowed_group_ids",
                parsed_target.route_id
            ))
        }
        _ => Ok(()),
    }
}

fn runtime_context_string_list(runtime_context: &Value, key: &str) -> Vec<String> {
    runtime_context
        .get("account")
        .and_then(|value| value.get("config"))
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn route_id_is_allowed(allowed_values: &[String], route_id: &str) -> bool {
    if allowed_values.is_empty() {
        return true;
    }

    allowed_values.iter().any(|allowed| {
        let trimmed_allowed = allowed.trim();
        trimmed_allowed == "*" || trimmed_allowed == route_id
    })
}

fn receive_batch_payload(binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding) -> Value {
    let mut payload_map = Map::new();
    payload_map.insert(
        "runtime_context".to_owned(),
        binding.runtime_context.clone(),
    );

    Value::Object(payload_map)
}

fn ack_inbound_payload(
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
    message: &mvp::channel::ChannelInboundMessage,
) -> Value {
    let message_value = serde_json::to_value(message).unwrap_or(Value::Null);
    let mut payload_map = Map::new();
    payload_map.insert(
        "runtime_context".to_owned(),
        binding.runtime_context.clone(),
    );
    payload_map.insert("message".to_owned(), message_value);

    Value::Object(payload_map)
}

async fn invoke_managed_bridge_operation(
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
    bridge_policy: &BridgeExecutionPolicy,
    operation: &str,
    payload: Value,
) -> CliResult<ManagedBridgeInvocationSuccess> {
    let provider = provider_config_from_binding(binding);
    let channel = channel_config_from_binding(binding);
    let required_capabilities = BTreeSet::from([Capability::InvokeConnector]);
    let command = loong_contracts::ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: operation.to_owned(),
        required_capabilities,
        payload,
    };
    let bridge_kind = binding.plugin.runtime.bridge_kind;

    match bridge_kind {
        kernel::PluginBridgeKind::HttpJson => {
            let execution_result =
                execute_http_json_bridge_call(&provider, &channel, &command).await;
            let execution_result = execution_result.map_err(|failure| failure.reason)?;
            Ok(ManagedBridgeInvocationSuccess {
                response_payload: execution_result.response_payload,
                runtime_evidence: execution_result.runtime_evidence,
            })
        }
        kernel::PluginBridgeKind::ProcessStdio => {
            let execution_result =
                execute_process_stdio_bridge_call(&provider, &channel, &command, bridge_policy)
                    .await;
            let execution_result = execution_result.map_err(|failure| failure.reason)?;
            Ok(ManagedBridgeInvocationSuccess {
                response_payload: execution_result.response_payload,
                runtime_evidence: execution_result.runtime_evidence,
            })
        }
        kernel::PluginBridgeKind::NativeFfi
        | kernel::PluginBridgeKind::WasmComponent
        | kernel::PluginBridgeKind::McpServer
        | kernel::PluginBridgeKind::AcpBridge
        | kernel::PluginBridgeKind::AcpRuntime
        | kernel::PluginBridgeKind::Unknown => Err(format!(
            "managed bridge runtime does not support bridge kind `{}` for plugin {}",
            bridge_kind.as_str(),
            binding.plugin.plugin_id
        )),
    }
}

fn provider_config_from_binding(
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
) -> kernel::ProviderConfig {
    let mut metadata = binding.plugin.metadata.clone();
    metadata.insert(
        "bridge_kind".to_owned(),
        binding.plugin.runtime.bridge_kind.as_str().to_owned(),
    );
    metadata.insert(
        "adapter_family".to_owned(),
        binding.plugin.runtime.adapter_family.clone(),
    );
    metadata.insert(
        "entrypoint".to_owned(),
        binding.plugin.runtime.entrypoint_hint.clone(),
    );
    let command = binding.plugin.metadata.get("command").cloned();
    let command = command.unwrap_or_else(|| binding.plugin.runtime.entrypoint_hint.clone());
    metadata.insert("command".to_owned(), command);
    metadata.insert(
        "channel_runtime_contract".to_owned(),
        binding.runtime_contract.clone(),
    );

    kernel::ProviderConfig {
        provider_id: binding.plugin.provider_id.clone(),
        connector_name: binding.plugin.connector_name.clone(),
        version: binding
            .plugin
            .plugin_version
            .clone()
            .unwrap_or_else(|| "0.1.0".to_owned()),
        metadata,
    }
}

fn channel_config_from_binding(
    binding: &mvp::channel::ManagedPluginBridgeRuntimeBinding,
) -> kernel::ChannelConfig {
    let metadata = BTreeMap::from([
        ("source_plugin".to_owned(), binding.plugin.plugin_id.clone()),
        (
            "configured_account_id".to_owned(),
            binding.configured_account_id.clone(),
        ),
    ]);

    kernel::ChannelConfig {
        channel_id: binding.channel_id.clone(),
        provider_id: binding.plugin.provider_id.clone(),
        endpoint: binding.endpoint.clone(),
        enabled: true,
        metadata,
    }
}

struct ManagedPluginBridgeChannelAdapter {
    binding: mvp::channel::ManagedPluginBridgeRuntimeBinding,
    bridge_policy: BridgeExecutionPolicy,
}

impl ManagedPluginBridgeChannelAdapter {
    fn new(
        binding: mvp::channel::ManagedPluginBridgeRuntimeBinding,
        bridge_policy: BridgeExecutionPolicy,
    ) -> Self {
        Self {
            binding,
            bridge_policy,
        }
    }
}

#[async_trait]
impl mvp::channel::ChannelAdapter for ManagedPluginBridgeChannelAdapter {
    fn name(&self) -> &str {
        self.binding.plugin.plugin_id.as_str()
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<mvp::channel::ChannelInboundMessage>> {
        let payload = receive_batch_payload(&self.binding);
        let invocation = invoke_managed_bridge_operation(
            &self.binding,
            &self.bridge_policy,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
            payload,
        )
        .await?;
        let response_object = invocation.response_payload.as_object();
        let Some(response_object) = response_object else {
            return Err("managed bridge receive_batch response must be an object".to_owned());
        };

        let messages_value = response_object.get("messages").cloned();
        let Some(messages_value) = messages_value else {
            return Err("managed bridge receive_batch response must include messages".to_owned());
        };

        serde_json::from_value(messages_value)
            .map_err(|error| format!("decode managed bridge messages failed: {error}"))
    }

    async fn send_message(
        &self,
        target: &mvp::channel::ChannelOutboundTarget,
        message: &mvp::channel::ChannelOutboundMessage,
    ) -> CliResult<()> {
        let payload = send_message_payload(&self.binding, target, message);
        let _ = invoke_managed_bridge_operation(
            &self.binding,
            &self.bridge_policy,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
            payload,
        )
        .await?;

        Ok(())
    }

    async fn ack_inbound(
        &mut self,
        message: &mvp::channel::ChannelInboundMessage,
    ) -> CliResult<()> {
        let supports_ack = self
            .binding
            .supports_operation(mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION);
        if !supports_ack {
            return Ok(());
        }

        let payload = ack_inbound_payload(&self.binding, message);
        let _ = invoke_managed_bridge_operation(
            &self.binding,
            &self.bridge_policy,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
            payload,
        )
        .await?;

        Ok(())
    }

    async fn complete_batch(&mut self) -> CliResult<()> {
        let supports_complete = self.binding.supports_operation(
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
        );
        if !supports_complete {
            return Ok(());
        }

        let payload = receive_batch_payload(&self.binding);
        let _ = invoke_managed_bridge_operation(
            &self.binding,
            &self.bridge_policy,
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            payload,
        )
        .await?;

        Ok(())
    }
}

async fn run_managed_plugin_bridge_loop<A, F>(
    stop: &mvp::channel::ChannelServeStopHandle,
    runtime: &mvp::channel::ChannelOperationRuntimeTracker,
    adapter: &mut A,
    once: bool,
    context: ManagedBridgeServeContext<'_>,
    mut process: F,
) -> CliResult<()>
where
    A: ChannelAdapter + Send + ?Sized,
    F: FnMut(
        mvp::channel::ChannelInboundMessage,
        mvp::channel::ChannelTurnFeedbackPolicy,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<String>> + Send>>,
{
    let mut consecutive_failures = 0usize;
    loop {
        if stop.is_requested() {
            return Ok(());
        }

        let iteration = tokio::select! {
            _ = stop.wait() => return Ok(()),
            result = run_managed_plugin_bridge_iteration(runtime, adapter, &mut process) => result,
        };
        match iteration {
            Ok(had_messages) => {
                if consecutive_failures > 0 {
                    let recovered_failures = consecutive_failures;
                    consecutive_failures = 0;
                    runtime.clear_failure().await?;
                    report_managed_bridge_serve_recovered(context, recovered_failures);
                }

                if once {
                    return Ok(());
                }

                if had_messages {
                    continue;
                }

                let sleep = tokio::time::sleep(Duration::from_millis(MANAGED_BRIDGE_IDLE_POLL_MS));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = stop.wait() => return Ok(()),
                    _ = &mut sleep => {}
                }
            }
            Err(error) => {
                runtime.record_failure(error.as_str()).await?;
                if once {
                    return Err(error);
                }

                consecutive_failures = consecutive_failures.saturating_add(1);
                if consecutive_failures >= MANAGED_BRIDGE_SERVE_MAX_CONSECUTIVE_FAILURES {
                    return Err(format!(
                        "{} bridge serve failed after {} consecutive managed bridge runtime errors (plugin_id={}, configured_account={}): {}",
                        context.channel_id,
                        consecutive_failures,
                        context.plugin_id,
                        context.configured_account_id,
                        error,
                    ));
                }

                let backoff_ms = managed_bridge_serve_backoff_ms(consecutive_failures)
                    .min(MANAGED_BRIDGE_SERVE_MAX_BACKOFF_MS);
                report_managed_bridge_serve_retry(
                    context,
                    consecutive_failures,
                    backoff_ms,
                    error.as_str(),
                );
                let sleep = tokio::time::sleep(Duration::from_millis(backoff_ms));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = stop.wait() => return Ok(()),
                    _ = &mut sleep => {}
                }
            }
        }
    }
}

async fn run_managed_plugin_bridge_iteration<A, F>(
    runtime: &mvp::channel::ChannelOperationRuntimeTracker,
    adapter: &mut A,
    process: &mut F,
) -> CliResult<bool>
where
    A: ChannelAdapter + Send + ?Sized,
    F: FnMut(
        mvp::channel::ChannelInboundMessage,
        mvp::channel::ChannelTurnFeedbackPolicy,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<String>> + Send>>,
{
    let batch = adapter.receive_batch().await?;
    mvp::channel::process_channel_batch(
        adapter,
        batch,
        Some(runtime),
        |message, feedback_policy| process(message, feedback_policy),
    )
    .await
}

fn managed_bridge_serve_backoff_ms(consecutive_failures: usize) -> u64 {
    let exponent = consecutive_failures.saturating_sub(1);
    let shift = u32::try_from(exponent).unwrap_or(u32::MAX).min(8);
    let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
    MANAGED_BRIDGE_SERVE_INITIAL_BACKOFF_MS
        .saturating_mul(multiplier)
        .min(MANAGED_BRIDGE_SERVE_MAX_BACKOFF_MS)
}

fn report_managed_bridge_serve_retry(
    context: ManagedBridgeServeContext<'_>,
    consecutive_failures: usize,
    backoff_ms: u64,
    error: &str,
) {
    tracing::warn!(
        target: "loong.managed_bridge",
        channel_id = context.channel_id,
        plugin_id = context.plugin_id,
        configured_account = context.configured_account_id,
        consecutive_failures,
        backoff_ms,
        error = error,
        "managed bridge serve iteration failed; retrying after transient backoff"
    );
    #[allow(clippy::print_stderr)]
    {
        eprintln!(
            "{} bridge serve transient failure {}/{} (plugin_id={}, configured_account={}); retrying in {}ms: {}",
            context.channel_id,
            consecutive_failures,
            MANAGED_BRIDGE_SERVE_MAX_CONSECUTIVE_FAILURES,
            context.plugin_id,
            context.configured_account_id,
            backoff_ms,
            error,
        );
    }
}

fn report_managed_bridge_serve_recovered(
    context: ManagedBridgeServeContext<'_>,
    recovered_failures: usize,
) {
    tracing::info!(
        target: "loong.managed_bridge",
        channel_id = context.channel_id,
        plugin_id = context.plugin_id,
        configured_account = context.configured_account_id,
        recovered_failures,
        "managed bridge serve recovered after transient failure"
    );
    #[allow(clippy::print_stderr)]
    {
        eprintln!(
            "{} bridge serve recovered after {} transient failure(s) (plugin_id={}, configured_account={})",
            context.channel_id,
            recovered_failures,
            context.plugin_id,
            context.configured_account_id,
        );
    }
}

pub fn default_weixin_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    mvp::channel::WEIXIN_CATALOG_COMMAND_FAMILY_DESCRIPTOR.default_send_target_kind
}

pub fn parse_weixin_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_managed_bridge_send_target_kind(
        mvp::channel::WEIXIN_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
        raw,
    )
}

pub fn default_qqbot_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    mvp::channel::QQBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR.default_send_target_kind
}

pub fn parse_qqbot_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_managed_bridge_send_target_kind(
        mvp::channel::QQBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
        raw,
    )
}

pub fn default_onebot_send_target_kind() -> mvp::channel::ChannelOutboundTargetKind {
    mvp::channel::ONEBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR.default_send_target_kind
}

pub fn parse_onebot_send_target_kind(
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    parse_managed_bridge_send_target_kind(
        mvp::channel::ONEBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
        raw,
    )
}

fn parse_managed_bridge_send_target_kind(
    family: mvp::channel::ChannelCatalogCommandFamilyDescriptor,
    raw: &str,
) -> Result<mvp::channel::ChannelOutboundTargetKind, String> {
    let target_kind = raw.parse::<mvp::channel::ChannelOutboundTargetKind>()?;
    let operation = family.send;
    let supports_target_kind = operation.supports_target_kind(target_kind);
    if supports_target_kind {
        return Ok(target_kind);
    }

    let supported_target_kinds = operation
        .supported_target_kinds
        .iter()
        .map(|kind| format!("`{}`", kind.as_str()))
        .collect::<Vec<_>>();
    let rendered_target_kinds = supported_target_kinds.join(" or ");
    Err(format!(
        "{} --target-kind does not support `{}`; use {}",
        family.channel_id,
        target_kind.as_str(),
        rendered_target_kinds
    ))
}

pub fn run_weixin_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    run_managed_plugin_bridge_send_cli_impl("weixin", args)
}

pub fn run_qqbot_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    run_managed_plugin_bridge_send_cli_impl("qqbot", args)
}

pub fn run_onebot_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    run_managed_plugin_bridge_send_cli_impl("onebot", args)
}

fn run_managed_plugin_bridge_send_cli_impl<'a>(
    channel_id: &'static str,
    args: ChannelSendCliArgs<'a>,
) -> ChannelCliCommandFuture<'a> {
    Box::pin(async move {
        let target = require_managed_bridge_target(channel_id, args.target)?;
        run_managed_plugin_bridge_send(
            args.config_path,
            channel_id,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

pub fn run_weixin_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    run_managed_plugin_bridge_serve_cli_impl("weixin", args)
}

pub fn run_qqbot_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    run_managed_plugin_bridge_serve_cli_impl("qqbot", args)
}

pub fn run_onebot_serve_cli_impl(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    run_managed_plugin_bridge_serve_cli_impl("onebot", args)
}

fn run_managed_plugin_bridge_serve_cli_impl<'a>(
    channel_id: &'static str,
    args: ChannelServeCliArgs<'a>,
) -> ChannelCliCommandFuture<'a> {
    Box::pin(async move {
        let _ = (args.bind_override, args.path_override);
        if args.stop_requested {
            return request_managed_plugin_bridge_serve_stop(
                args.config_path,
                channel_id,
                args.account,
            )
            .await;
        }
        if args.stop_duplicates_requested {
            return request_managed_plugin_bridge_serve_duplicate_cleanup(
                args.config_path,
                channel_id,
                args.account,
            )
            .await;
        }
        crate::with_graceful_shutdown(run_managed_plugin_bridge_channel(
            args.config_path,
            channel_id,
            args.account,
            args.once,
        ))
        .await
    })
}

fn require_managed_bridge_target<'a>(
    channel_id: &str,
    target: Option<&'a str>,
) -> CliResult<&'a str> {
    let command_name = format!("channels send {channel_id}");
    let target = target.map(str::trim);
    let Some(target) = target else {
        return Err(format!("{command_name} requires --target"));
    };
    if target.is_empty() {
        return Err(format!("{command_name} requires a non-empty --target"));
    }

    Ok(target)
}

fn normalize_bridge_account_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "default".to_owned();
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_was_separator = false;
    for value in trimmed.chars() {
        if value.is_ascii_alphanumeric() {
            normalized.push(value.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }
        if matches!(value, '_' | '-') {
            if !normalized.is_empty() && !last_was_separator {
                normalized.push(value);
                last_was_separator = true;
            }
            continue;
        }
        if !normalized.is_empty() && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }

    while matches!(normalized.chars().last(), Some('-' | '_')) {
        normalized.pop();
    }

    if normalized.is_empty() {
        "default".to_owned()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::Json;
    use axum::Router;
    use axum::extract::State;
    use axum::routing::post;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct CaptureState {
        requests: Arc<Mutex<Vec<Value>>>,
    }

    #[derive(Debug, Clone)]
    enum ScriptedReceiveStep {
        Batch(Vec<mvp::channel::ChannelInboundMessage>),
        Error(String),
    }

    #[derive(Clone, Default)]
    struct ScriptedAdapterState {
        receive_calls: Arc<AtomicUsize>,
        send_calls: Arc<AtomicUsize>,
        ack_calls: Arc<AtomicUsize>,
        complete_calls: Arc<AtomicUsize>,
    }

    struct ScriptedChannelAdapter {
        name: String,
        receive_steps: Mutex<Vec<ScriptedReceiveStep>>,
        state: ScriptedAdapterState,
    }

    impl ScriptedChannelAdapter {
        fn new(
            name: impl Into<String>,
            receive_steps: Vec<ScriptedReceiveStep>,
            state: ScriptedAdapterState,
        ) -> Self {
            Self {
                name: name.into(),
                receive_steps: Mutex::new(receive_steps),
                state,
            }
        }
    }

    #[async_trait]
    impl mvp::channel::ChannelAdapter for ScriptedChannelAdapter {
        fn name(&self) -> &str {
            self.name.as_str()
        }

        async fn receive_batch(&mut self) -> CliResult<Vec<mvp::channel::ChannelInboundMessage>> {
            self.state.receive_calls.fetch_add(1, Ordering::Relaxed);
            let mut steps = self
                .receive_steps
                .lock()
                .expect("lock scripted receive steps");
            if steps.is_empty() {
                return Ok(Vec::new());
            }

            match steps.remove(0) {
                ScriptedReceiveStep::Batch(batch) => Ok(batch),
                ScriptedReceiveStep::Error(error) => Err(error),
            }
        }

        async fn send_message(
            &self,
            _target: &mvp::channel::ChannelOutboundTarget,
            _message: &mvp::channel::ChannelOutboundMessage,
        ) -> CliResult<()> {
            self.state.send_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn ack_inbound(
            &mut self,
            _message: &mvp::channel::ChannelInboundMessage,
        ) -> CliResult<()> {
            self.state.ack_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn complete_batch(&mut self) -> CliResult<()> {
            self.state.complete_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    async fn capture_handler(
        State(state): State<CaptureState>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let mut guard = state.requests.lock().expect("lock captured requests");
        guard.push(body);
        Json(json!({
            "payload": {
                "ok": true
            }
        }))
    }

    fn scripted_inbound_message(
        platform: mvp::channel::ChannelPlatform,
        account_id: &str,
        conversation_id: &str,
        reply_target_id: &str,
        text: &str,
    ) -> mvp::channel::ChannelInboundMessage {
        mvp::channel::ChannelInboundMessage {
            session: mvp::channel::ChannelSession::with_account(
                platform,
                account_id,
                conversation_id,
            )
            .with_configured_account_id(account_id),
            reply_target: mvp::channel::ChannelOutboundTarget::new(
                platform,
                mvp::channel::ChannelOutboundTargetKind::Conversation,
                reply_target_id,
            ),
            text: text.to_owned(),
            delivery: mvp::channel::ChannelDelivery::default(),
        }
    }

    fn write_runtime_manifest(root: &std::path::Path, endpoint: &str) {
        let runtime_operations =
            vec![mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION.to_owned()];
        let runtime_operations_json =
            serde_json::to_string(&runtime_operations).expect("serialize runtime operations");
        let metadata = BTreeMap::from([
            ("bridge_kind".to_owned(), "http_json".to_owned()),
            ("adapter_family".to_owned(), "channel-bridge".to_owned()),
            (
                "transport_family".to_owned(),
                "wechat_clawbot_ilink_bridge".to_owned(),
            ),
            ("target_contract".to_owned(), "weixin_reply_loop".to_owned()),
            (
                "channel_runtime_contract".to_owned(),
                mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1.to_owned(),
            ),
            (
                "channel_runtime_operations_json".to_owned(),
                runtime_operations_json,
            ),
        ]);
        let manifest = kernel::PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("1.0.0".to_owned()),
            plugin_id: "weixin-managed-runtime".to_owned(),
            provider_id: "weixin-managed-runtime-provider".to_owned(),
            connector_name: "weixin-managed-runtime-connector".to_owned(),
            channel_id: Some("weixin".to_owned()),
            endpoint: Some(endpoint.to_owned()),
            capabilities: BTreeSet::new(),
            trust_tier: kernel::PluginTrustTier::Unverified,
            metadata,
            summary: None,
            tags: Vec::new(),
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup: Some(kernel::PluginSetup {
                mode: kernel::PluginSetupMode::MetadataOnly,
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
        };
        let plugin_directory = root.join("weixin-managed-runtime");
        let manifest_path = plugin_directory.join("loong.plugin.json");
        let encoded_manifest =
            serde_json::to_string_pretty(&manifest).expect("serialize runtime manifest");

        fs::create_dir_all(&plugin_directory).expect("create runtime plugin directory");
        fs::write(&manifest_path, encoded_manifest).expect("write runtime plugin manifest");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_managed_plugin_bridge_send_posts_http_bridge_request() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        let config_root = TempDir::new().expect("create config root");
        let capture_state = CaptureState::default();
        let router = Router::new()
            .route("/bridge", post(capture_handler))
            .with_state(capture_state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind managed bridge test server");
        let local_addr = listener.local_addr().expect("read local addr");
        let server = axum::serve(listener, router);
        let server_task = tokio::spawn(async move {
            let _ = server.await;
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        let endpoint = format!("http://{local_addr}/bridge");

        write_runtime_manifest(runtime_root.path(), endpoint.as_str());

        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some(endpoint.clone());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "bridge-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let config_path = config_root.path().join("loong.toml");
        let encoded_config = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, encoded_config).expect("write config");
        let config_path_string = config_path.to_string_lossy().to_string();

        run_managed_plugin_bridge_send(
            Some(config_path_string.as_str()),
            "weixin",
            None,
            "contact:wxid_alice",
            mvp::channel::ChannelOutboundTargetKind::Conversation,
            "hello bridge",
        )
        .await
        .expect("managed bridge send should succeed");

        let requests = capture_state
            .requests
            .lock()
            .expect("lock captured requests");
        let request_count = requests.len();
        assert_eq!(request_count, 1);

        let request = requests.first().expect("captured request");
        let operation = request
            .get("operation")
            .and_then(Value::as_str)
            .expect("operation");
        assert_eq!(operation, "send_message");

        let payload = request.get("payload").cloned().expect("payload");
        let target = payload
            .get("target")
            .and_then(Value::as_object)
            .expect("target object");
        let message = payload
            .get("message")
            .and_then(Value::as_object)
            .expect("message object");

        assert_eq!(
            target.get("id").and_then(Value::as_str),
            Some("weixin:default:contact:wxid_alice")
        );
        assert_eq!(
            message.get("Text").and_then(Value::as_str),
            Some("hello bridge")
        );

        server_task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_managed_plugin_bridge_send_rejects_conflicting_embedded_account() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        let config_root = TempDir::new().expect("create config root");
        write_runtime_manifest(runtime_root.path(), "http://127.0.0.1:9/bridge");

        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.default_account = Some("ops".to_owned());
        config.weixin.accounts.insert(
            "ops".to_owned(),
            mvp::config::WeixinAccountConfig {
                enabled: Some(true),
                account_id: Some("ops".to_owned()),
                bridge_url: Some("http://127.0.0.1:9/bridge".to_owned()),
                bridge_url_env: None,
                bridge_access_token: Some(loong_contracts::SecretRef::Inline(
                    "bridge-token".to_owned(),
                )),
                bridge_access_token_env: None,
                allowed_contact_ids: Some(vec!["wxid_alice".to_owned()]),
            },
        );

        let config_path = config_root.path().join("loong.toml");
        let encoded_config = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, encoded_config).expect("write config");
        let config_path_string = config_path.to_string_lossy().to_string();

        let error = run_managed_plugin_bridge_send(
            Some(config_path_string.as_str()),
            "weixin",
            Some("ops"),
            "backup:contact:wxid_alice",
            mvp::channel::ChannelOutboundTargetKind::Conversation,
            "hello bridge",
        )
        .await
        .expect_err("conflicting account should fail");

        assert!(
            error.contains("selected configured account is `ops`"),
            "unexpected managed bridge target/account error: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_managed_plugin_bridge_send_rejects_disallowed_weixin_contact() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        let config_root = TempDir::new().expect("create config root");
        write_runtime_manifest(runtime_root.path(), "http://127.0.0.1:9/bridge");

        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("http://127.0.0.1:9/bridge".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "bridge-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let config_path = config_root.path().join("loong.toml");
        let encoded_config = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, encoded_config).expect("write config");
        let config_path_string = config_path.to_string_lossy().to_string();

        let error = run_managed_plugin_bridge_send(
            Some(config_path_string.as_str()),
            "weixin",
            None,
            "contact:wxid_bob",
            mvp::channel::ChannelOutboundTargetKind::Conversation,
            "hello bridge",
        )
        .await
        .expect_err("disallowed weixin contact should fail");

        assert!(
            error.contains("allowed_contact_ids"),
            "unexpected weixin allowlist error: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn request_managed_plugin_bridge_serve_stop_writes_stop_request() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        let config_root = TempDir::new().expect("create config root");
        let temp_home = TempDir::new().expect("create temp loong home");
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONG_HOME", temp_home.path().as_os_str());
        write_runtime_manifest(runtime_root.path(), "http://127.0.0.1:9/bridge");

        let runtime_dir = mvp::config::default_loong_home().join("channel-runtime");
        fs::create_dir_all(&runtime_dir).expect("create channel runtime dir");
        let runtime_path = runtime_dir.join("weixin-serve-default-5151.json");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as u64;
        let runtime_state = serde_json::json!({
            "running": true,
            "busy": false,
            "active_runs": 0,
            "consecutive_failures": 0,
            "last_run_activity_at": now_ms.saturating_sub(500),
            "last_heartbeat_at": now_ms.saturating_sub(100),
            "last_failure_at": serde_json::Value::Null,
            "last_recovery_at": serde_json::Value::Null,
            "last_error": serde_json::Value::Null,
            "pid": 5151u32,
            "account_id": "default",
            "account_label": "default",
            "owner_token": "owner-5151"
        });
        let encoded_runtime =
            serde_json::to_string_pretty(&runtime_state).expect("serialize runtime state");
        fs::write(&runtime_path, encoded_runtime).expect("write runtime state");

        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("http://127.0.0.1:9/bridge".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "bridge-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let config_path = config_root.path().join("loong.toml");
        let encoded_config = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, encoded_config).expect("write config");
        let config_path_string = config_path.to_string_lossy().to_string();

        request_managed_plugin_bridge_serve_stop(Some(config_path_string.as_str()), "weixin", None)
            .await
            .expect("request managed bridge serve stop");

        let stop_request_path =
            runtime_dir.join("weixin-serve-default-stop-request-owner-5151.json");
        let encoded_stop_request =
            fs::read_to_string(&stop_request_path).expect("read stop request");
        let stop_request: serde_json::Value =
            serde_json::from_str(&encoded_stop_request).expect("decode stop request");

        assert_eq!(
            stop_request
                .get("target_owner_token")
                .and_then(serde_json::Value::as_str),
            Some("owner-5151")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn request_managed_plugin_bridge_duplicate_cleanup_keeps_preferred_owner() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        let config_root = TempDir::new().expect("create config root");
        let temp_home = TempDir::new().expect("create temp loong home");
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONG_HOME", temp_home.path().as_os_str());
        write_runtime_manifest(runtime_root.path(), "http://127.0.0.1:9/bridge");

        let runtime_dir = mvp::config::default_loong_home().join("channel-runtime");
        fs::create_dir_all(&runtime_dir).expect("create channel runtime dir");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as u64;
        let first_runtime_state = serde_json::json!({
            "running": true,
            "busy": false,
            "active_runs": 0,
            "consecutive_failures": 0,
            "last_run_activity_at": now_ms.saturating_sub(2_000),
            "last_heartbeat_at": now_ms.saturating_sub(1_000),
            "last_failure_at": serde_json::Value::Null,
            "last_recovery_at": serde_json::Value::Null,
            "last_error": serde_json::Value::Null,
            "pid": 5151u32,
            "account_id": "default",
            "account_label": "default",
            "owner_token": "owner-5151"
        });
        fs::write(
            runtime_dir.join("weixin-serve-default-5151.json"),
            serde_json::to_string_pretty(&first_runtime_state).expect("serialize first runtime"),
        )
        .expect("write first runtime");
        let second_runtime_state = serde_json::json!({
            "running": true,
            "busy": false,
            "active_runs": 0,
            "consecutive_failures": 0,
            "last_run_activity_at": now_ms.saturating_sub(200),
            "last_heartbeat_at": now_ms.saturating_sub(100),
            "last_failure_at": serde_json::Value::Null,
            "last_recovery_at": serde_json::Value::Null,
            "last_error": serde_json::Value::Null,
            "pid": 6262u32,
            "account_id": "default",
            "account_label": "default",
            "owner_token": "owner-6262"
        });
        fs::write(
            runtime_dir.join("weixin-serve-default-6262.json"),
            serde_json::to_string_pretty(&second_runtime_state).expect("serialize second runtime"),
        )
        .expect("write second runtime");

        let mut config = mvp::config::LoongConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some("http://127.0.0.1:9/bridge".to_owned());
        config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
            "bridge-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let config_path = config_root.path().join("loong.toml");
        let encoded_config = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, encoded_config).expect("write config");
        let config_path_string = config_path.to_string_lossy().to_string();

        request_managed_plugin_bridge_serve_duplicate_cleanup(
            Some(config_path_string.as_str()),
            "weixin",
            None,
        )
        .await
        .expect("request managed bridge duplicate cleanup");

        let first_stop_request_path =
            runtime_dir.join("weixin-serve-default-stop-request-owner-5151.json");
        assert!(
            first_stop_request_path.exists(),
            "older duplicate owner should be targeted"
        );
        let second_stop_request_path =
            runtime_dir.join("weixin-serve-default-stop-request-owner-6262.json");
        assert!(
            !second_stop_request_path.exists(),
            "preferred runtime owner should be kept running"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_bridge_serve_loop_recovers_after_transient_receive_failure() {
        let adapter_state = ScriptedAdapterState::default();
        let processed_messages = Arc::new(AtomicUsize::new(0));
        let processed_messages_for_run = processed_messages.clone();
        let adapter_state_for_assert = adapter_state.clone();
        let stop = mvp::channel::ChannelServeStopHandle::new();
        let spec = mvp::channel::ChannelServeRuntimeSpec {
            platform: mvp::channel::ChannelPlatform::Weixin,
            operation_id: mvp::channel::CHANNEL_OPERATION_SERVE_ID,
            account_id: "managed-bridge-recovery",
            account_label: "managed-bridge-recovery",
        };

        mvp::channel::with_channel_serve_runtime_with_stop(
            spec,
            stop.clone(),
            move |runtime, stop| async move {
                let inbound_message = scripted_inbound_message(
                    mvp::channel::ChannelPlatform::Weixin,
                    "default",
                    "wxid_alice",
                    "weixin:default:contact:wxid_alice",
                    "hello bridge",
                );
                let mut adapter = ScriptedChannelAdapter::new(
                    "scripted-bridge",
                    vec![
                        ScriptedReceiveStep::Error("temporary receive failure".to_owned()),
                        ScriptedReceiveStep::Batch(vec![inbound_message]),
                    ],
                    adapter_state,
                );
                let stop_for_process = stop.clone();
                run_managed_plugin_bridge_loop(
                    &stop,
                    runtime.as_ref(),
                    &mut adapter,
                    false,
                    ManagedBridgeServeContext {
                        channel_id: "weixin",
                        plugin_id: "scripted-bridge",
                        configured_account_id: "managed-bridge-recovery",
                    },
                    move |_message, _feedback_policy| {
                        let processed_messages = processed_messages_for_run.clone();
                        let stop = stop_for_process.clone();
                        Box::pin(async move {
                            processed_messages.fetch_add(1, Ordering::Relaxed);
                            stop.request_stop();
                            Ok("pong".to_owned())
                        })
                    },
                )
                .await
            },
        )
        .await
        .expect("serve loop should recover after a transient receive failure");

        assert_eq!(processed_messages.load(Ordering::Relaxed), 1);
        assert_eq!(
            adapter_state_for_assert
                .receive_calls
                .load(Ordering::Relaxed),
            2
        );
        assert_eq!(
            adapter_state_for_assert.send_calls.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            adapter_state_for_assert.ack_calls.load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            adapter_state_for_assert
                .complete_calls
                .load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_bridge_serve_loop_stops_after_retry_budget() {
        let adapter_state = ScriptedAdapterState::default();
        let adapter_state_for_assert = adapter_state.clone();
        let stop = mvp::channel::ChannelServeStopHandle::new();
        let spec = mvp::channel::ChannelServeRuntimeSpec {
            platform: mvp::channel::ChannelPlatform::Weixin,
            operation_id: mvp::channel::CHANNEL_OPERATION_SERVE_ID,
            account_id: "managed-bridge-budget",
            account_label: "managed-bridge-budget",
        };

        let error = mvp::channel::with_channel_serve_runtime_with_stop(
            spec,
            stop,
            move |runtime, stop| async move {
                let mut adapter = ScriptedChannelAdapter::new(
                    "scripted-bridge",
                    vec![
                        ScriptedReceiveStep::Error("failure one".to_owned()),
                        ScriptedReceiveStep::Error("failure two".to_owned()),
                        ScriptedReceiveStep::Error("failure three".to_owned()),
                    ],
                    adapter_state,
                );
                run_managed_plugin_bridge_loop(
                    &stop,
                    runtime.as_ref(),
                    &mut adapter,
                    false,
                    ManagedBridgeServeContext {
                        channel_id: "weixin",
                        plugin_id: "scripted-bridge",
                        configured_account_id: "managed-bridge-budget",
                    },
                    |_message, _feedback_policy| Box::pin(async { Ok("unused".to_owned()) }),
                )
                .await
            },
        )
        .await
        .expect_err("serve loop should stop after exhausting the retry budget");

        assert!(
            error.contains("failed after 3 consecutive managed bridge runtime errors"),
            "unexpected retry-budget error: {error}"
        );
        assert_eq!(
            adapter_state_for_assert
                .receive_calls
                .load(Ordering::Relaxed),
            3
        );
        assert_eq!(
            adapter_state_for_assert.send_calls.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            adapter_state_for_assert.ack_calls.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            adapter_state_for_assert
                .complete_calls
                .load(Ordering::Relaxed),
            0
        );
    }
}
