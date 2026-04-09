use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use loongclaw_bridge_runtime::BridgeExecutionPolicy;
use loongclaw_bridge_runtime::execute_http_json_bridge_call;
use loongclaw_bridge_runtime::execute_process_stdio_bridge_call;
use loongclaw_contracts::Capability;
use loongclaw_spec::CliResult;
use serde_json::{Map, Value};

use crate::mvp;
use crate::mvp::channel::ChannelAdapter;
use crate::{ChannelCliCommandFuture, ChannelSendCliArgs, ChannelServeCliArgs};

struct ManagedBridgeInvocationSuccess {
    response_payload: Value,
    runtime_evidence: Value,
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
    let binding = mvp::channel::resolve_managed_plugin_bridge_runtime_binding(
        &config, channel_id, account_id,
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
    let outbound_target =
        mvp::channel::ChannelOutboundTarget::new(target_platform(channel_id)?, target_kind, target);
    let outbound_message = mvp::channel::ChannelOutboundMessage::Text(text.to_owned());
    let payload = send_message_payload(&binding, &outbound_target, &outbound_message);
    let invocation =
        invoke_managed_bridge_operation(&binding, &bridge_policy, "send_message", payload).await?;
    let runtime_evidence_is_null = invocation.runtime_evidence.is_null();
    if !runtime_evidence_is_null {
        tracing::debug!(
            target: "loongclaw.managed_bridge",
            channel_id,
            plugin_id = %binding.plugin.plugin_id,
            bridge_kind = %binding.plugin.runtime.bridge_kind.as_str(),
            "managed bridge send completed with runtime evidence"
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
            run_managed_plugin_bridge_loop(
                config,
                resolved_path,
                kernel_ctx,
                &stop,
                &runtime,
                &mut adapter,
                once,
            )
            .await
        },
    )
    .await
}

fn load_managed_bridge_runtime_config(
    config_path: Option<&str>,
) -> CliResult<(PathBuf, mvp::config::LoongClawConfig)> {
    mvp::config::load(config_path)
}

fn bridge_execution_policy_from_config(
    config: &mvp::config::LoongClawConfig,
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
    let command = loongclaw_contracts::ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: operation.to_owned(),
        required_capabilities,
        payload,
    };
    let bridge_kind = binding.plugin.runtime.bridge_kind;

    match bridge_kind {
        kernel::PluginBridgeKind::HttpJson => {
            let execution_result = execute_http_json_bridge_call(&provider, &channel, &command);
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

async fn run_managed_plugin_bridge_loop(
    config: Arc<mvp::config::LoongClawConfig>,
    resolved_path: Option<PathBuf>,
    kernel_ctx: Arc<mvp::KernelContext>,
    stop: &mvp::channel::ChannelServeStopHandle,
    runtime: &mvp::channel::ChannelOperationRuntimeTracker,
    adapter: &mut ManagedPluginBridgeChannelAdapter,
    once: bool,
) -> CliResult<()> {
    loop {
        if stop.is_requested() {
            return Ok(());
        }

        let batch = adapter.receive_batch().await?;
        let had_messages = mvp::channel::process_channel_batch(
            adapter,
            batch,
            Some(runtime),
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
        .await?;

        if once {
            return Ok(());
        }

        if had_messages {
            continue;
        }

        let sleep = tokio::time::sleep(Duration::from_millis(500));
        tokio::pin!(sleep);
        tokio::select! {
            _ = stop.wait() => return Ok(()),
            _ = &mut sleep => {}
        }
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
        .await?;
        println!(
            "{} message sent via managed bridge runtime (target={}, target_kind={})",
            channel_id,
            target,
            args.target_kind.as_str(),
        );
        Ok(())
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
    let command_name = format!("{channel_id}-send");
    let target = target.map(str::trim);
    let Some(target) = target else {
        return Err(format!("{command_name} requires --target"));
    };
    if target.is_empty() {
        return Err(format!("{command_name} requires a non-empty --target"));
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
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
        let manifest_path = plugin_directory.join("loongclaw.plugin.json");
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

        let mut config = mvp::config::LoongClawConfig::default();
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        config.weixin.enabled = true;
        config.weixin.bridge_url = Some(endpoint.clone());
        config.weixin.bridge_access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "bridge-token".to_owned(),
        ));
        config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

        let config_path = config_root.path().join("loongclaw.toml");
        let encoded_config = toml::to_string(&config).expect("serialize config");
        fs::write(&config_path, encoded_config).expect("write config");
        let config_path_string = config_path.to_string_lossy().to_string();

        run_managed_plugin_bridge_send(
            Some(config_path_string.as_str()),
            "weixin",
            None,
            "weixin:default:contact:wxid_alice",
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
}
