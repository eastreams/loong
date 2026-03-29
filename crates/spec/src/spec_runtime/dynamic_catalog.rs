use super::*;

pub struct DynamicCatalogConnector {
    pub connector_name: String,
    pub provider_id: String,
    pub catalog: Arc<Mutex<IntegrationCatalog>>,
    pub bridge_runtime_policy: BridgeRuntimePolicy,
    pub bridge_circuit_state: Arc<TokioMutex<ConnectorCircuitRuntimeState>>,
}

#[derive(Debug, Clone)]
struct ProviderPluginCompatibilityContext {
    payload: Value,
    blocking_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct BridgeCircuitObservation {
    phase_before: String,
    phase_after: String,
    consecutive_failures: usize,
    half_open_remaining_calls: usize,
    half_open_successes: usize,
    remaining_cooldown_ms: Option<u64>,
}

impl BridgeCircuitObservation {
    fn disabled() -> Self {
        Self {
            phase_before: "disabled".to_owned(),
            phase_after: "disabled".to_owned(),
            consecutive_failures: 0,
            half_open_remaining_calls: 0,
            half_open_successes: 0,
            remaining_cooldown_ms: None,
        }
    }
}

impl DynamicCatalogConnector {
    pub fn new(
        connector_name: String,
        provider_id: String,
        catalog: Arc<Mutex<IntegrationCatalog>>,
        bridge_runtime_policy: BridgeRuntimePolicy,
    ) -> Self {
        let bridge_circuit_state =
            Arc::new(TokioMutex::new(ConnectorCircuitRuntimeState::default()));

        Self {
            connector_name,
            provider_id,
            catalog,
            bridge_runtime_policy,
            bridge_circuit_state,
        }
    }

    async fn acquire_bridge_circuit_phase(&self) -> Result<String, ConnectorError> {
        let policy = &self.bridge_runtime_policy.bridge_circuit_breaker;
        if !policy.enabled {
            return Ok("disabled".to_owned());
        }

        let mut state = self.bridge_circuit_state.lock().await;
        let now = TokioInstant::now();
        let acquire_result = acquire_connector_circuit_slot_for_state(policy, &mut state, now);

        match acquire_result {
            Ok(phase) => Ok(phase.to_owned()),
            Err(ConnectorCircuitAcquireError::Open {
                remaining_cooldown_ms,
            }) => {
                let reason = format!(
                    "plugin connector {} is circuit-open (remaining_cooldown_ms={remaining_cooldown_ms})",
                    self.connector_name
                );
                let circuit_phase = connector_circuit_phase_label(state.phase).to_owned();
                let consecutive_failures = state.consecutive_failures;
                let half_open_remaining_calls = state.half_open_remaining_calls;
                let half_open_successes = state.half_open_successes;
                let last_failure_reason = Some(reason.clone());
                drop(state);

                let persist_result = self
                    .persist_plugin_runtime_health(
                        policy,
                        circuit_phase,
                        consecutive_failures,
                        half_open_remaining_calls,
                        half_open_successes,
                        last_failure_reason,
                    )
                    .await;
                if let Err(error) = persist_result {
                    let combined_reason =
                        format!("{reason}; failed to persist plugin runtime health: {error}");
                    return Err(ConnectorError::Execution(combined_reason));
                }

                Err(ConnectorError::Execution(reason))
            }
            Err(ConnectorCircuitAcquireError::HalfOpenReopened) => {
                let reason = format!(
                    "plugin connector {} half-open window exhausted and re-opened",
                    self.connector_name
                );
                let circuit_phase = connector_circuit_phase_label(state.phase).to_owned();
                let consecutive_failures = state.consecutive_failures;
                let half_open_remaining_calls = state.half_open_remaining_calls;
                let half_open_successes = state.half_open_successes;
                let last_failure_reason = Some(reason.clone());
                drop(state);

                let persist_result = self
                    .persist_plugin_runtime_health(
                        policy,
                        circuit_phase,
                        consecutive_failures,
                        half_open_remaining_calls,
                        half_open_successes,
                        last_failure_reason,
                    )
                    .await;
                if let Err(error) = persist_result {
                    let combined_reason =
                        format!("{reason}; failed to persist plugin runtime health: {error}");
                    return Err(ConnectorError::Execution(combined_reason));
                }

                Err(ConnectorError::Execution(reason))
            }
        }
    }

    async fn record_bridge_circuit_outcome(
        &self,
        success: bool,
        phase_before: &str,
    ) -> BridgeCircuitObservation {
        let policy = &self.bridge_runtime_policy.bridge_circuit_breaker;
        if !policy.enabled {
            return BridgeCircuitObservation::disabled();
        }

        let mut state = self.bridge_circuit_state.lock().await;
        let now = TokioInstant::now();
        let phase_after =
            record_connector_circuit_outcome_for_state(policy, &mut state, success, now);
        let remaining_cooldown_ms = connector_circuit_remaining_cooldown_ms(&state, now);

        BridgeCircuitObservation {
            phase_before: phase_before.to_owned(),
            phase_after: phase_after.to_owned(),
            consecutive_failures: state.consecutive_failures,
            half_open_remaining_calls: state.half_open_remaining_calls,
            half_open_successes: state.half_open_successes,
            remaining_cooldown_ms,
        }
    }

    async fn persist_plugin_runtime_health(
        &self,
        policy: &ConnectorCircuitBreakerPolicy,
        circuit_phase: String,
        consecutive_failures: usize,
        half_open_remaining_calls: usize,
        half_open_successes: usize,
        last_failure_reason: Option<String>,
    ) -> Result<(), String> {
        let health = build_plugin_runtime_health_result(
            policy,
            circuit_phase,
            consecutive_failures,
            half_open_remaining_calls,
            half_open_successes,
            last_failure_reason,
        );
        let encoded = encode_plugin_runtime_health_result(&health)?;
        let metadata_key = PLUGIN_RUNTIME_HEALTH_METADATA_KEY.to_owned();
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_err| "integration catalog mutex poisoned".to_owned())?;
        let provider = catalog
            .provider(&self.provider_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "provider {} is not registered in integration catalog",
                    self.provider_id
                )
            })?;
        let is_plugin_backed = provider_is_plugin_backed(&provider.metadata);
        if !is_plugin_backed {
            return Ok(());
        }

        let mut updated_provider = provider;
        updated_provider.metadata.insert(metadata_key, encoded);
        catalog.upsert_provider(updated_provider);

        Ok(())
    }
}

fn bridge_execution_status_is_failure(bridge_execution: &Value) -> bool {
    bridge_execution
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "blocked" | "failed"))
}

fn bridge_circuit_breaker_runtime_value(
    policy: &ConnectorCircuitBreakerPolicy,
    observation: &BridgeCircuitObservation,
) -> Value {
    let mut payload = Map::new();
    let enabled = policy.enabled;
    let phase_before = observation.phase_before.clone();
    let phase_after = observation.phase_after.clone();
    let failure_threshold = policy.failure_threshold as u64;
    let cooldown_ms = policy.cooldown_ms;
    let half_open_max_calls = policy.half_open_max_calls as u64;
    let success_threshold = policy.success_threshold as u64;
    let consecutive_failures = observation.consecutive_failures as u64;
    let half_open_remaining_calls = observation.half_open_remaining_calls as u64;
    let half_open_successes = observation.half_open_successes as u64;

    payload.insert("enabled".to_owned(), Value::Bool(enabled));
    payload.insert("phase_before".to_owned(), Value::String(phase_before));
    payload.insert("phase_after".to_owned(), Value::String(phase_after));
    payload.insert(
        "failure_threshold".to_owned(),
        Value::Number(failure_threshold.into()),
    );
    payload.insert("cooldown_ms".to_owned(), Value::Number(cooldown_ms.into()));
    payload.insert(
        "half_open_max_calls".to_owned(),
        Value::Number(half_open_max_calls.into()),
    );
    payload.insert(
        "success_threshold".to_owned(),
        Value::Number(success_threshold.into()),
    );
    payload.insert(
        "consecutive_failures".to_owned(),
        Value::Number(consecutive_failures.into()),
    );
    payload.insert(
        "half_open_remaining_calls".to_owned(),
        Value::Number(half_open_remaining_calls.into()),
    );
    payload.insert(
        "half_open_successes".to_owned(),
        Value::Number(half_open_successes.into()),
    );

    let remaining_cooldown_ms = observation
        .remaining_cooldown_ms
        .map(|value| Value::Number(value.into()))
        .unwrap_or(Value::Null);
    payload.insert("remaining_cooldown_ms".to_owned(), remaining_cooldown_ms);

    Value::Object(payload)
}

fn attach_bridge_circuit_breaker_runtime(
    bridge_execution: &mut Value,
    policy: &ConnectorCircuitBreakerPolicy,
    observation: &BridgeCircuitObservation,
) {
    let Some(bridge_execution_object) = bridge_execution.as_object_mut() else {
        return;
    };

    let runtime_value = bridge_circuit_breaker_runtime_value(policy, observation);
    bridge_execution_object.insert("circuit_breaker".to_owned(), runtime_value);
}

fn bridge_execution_reason(bridge_execution: &Value) -> Option<String> {
    bridge_execution
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn format_bridge_execution_failure_reason(
    reason: &str,
    observation: &BridgeCircuitObservation,
) -> String {
    let phase_before = observation.phase_before.as_str();
    let phase_after = observation.phase_after.as_str();

    format!(
        "{reason} (bridge_circuit_phase_before={phase_before}, bridge_circuit_phase_after={phase_after})"
    )
}

#[async_trait]
impl CoreConnectorAdapter for DynamicCatalogConnector {
    fn name(&self) -> &str {
        &self.connector_name
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let requested_channel = command
            .payload
            .get("channel_id")
            .and_then(Value::as_str)
            .map(std::string::ToString::to_string);

        let (provider, chosen_channel) = {
            let catalog = self.catalog.lock().map_err(|_err| {
                ConnectorError::Execution("integration catalog mutex poisoned".to_owned())
            })?;

            let provider = catalog.provider(&self.provider_id).ok_or_else(|| {
                ConnectorError::Execution(format!(
                    "provider {} is not registered in integration catalog",
                    self.provider_id
                ))
            })?;

            let allowed_callers = provider_allowed_callers(provider);
            if !allowed_callers.is_empty() {
                let caller = caller_from_payload(&command.payload);
                if !caller_is_allowed(caller.as_deref(), &allowed_callers) {
                    let caller_label = caller.unwrap_or_else(|| "unknown".to_owned());
                    return Err(ConnectorError::Execution(format!(
                        "caller {caller_label} is not allowed for connector {} (allowed_callers={})",
                        self.connector_name,
                        allowed_callers
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(",")
                    )));
                }
            }

            let chosen_channel = if let Some(channel_id) = requested_channel.as_ref() {
                let channel = catalog.channel(channel_id).ok_or_else(|| {
                    ConnectorError::Execution(format!("channel {channel_id} not found"))
                })?;
                if !channel.enabled {
                    return Err(ConnectorError::Execution(format!(
                        "channel {channel_id} is disabled"
                    )));
                }
                if channel.provider_id != provider.provider_id {
                    return Err(ConnectorError::Execution(format!(
                        "channel {} does not belong to provider {}",
                        channel.channel_id, provider.provider_id
                    )));
                }
                channel.clone()
            } else {
                catalog
                    .channels_for_provider(&provider.provider_id)
                    .into_iter()
                    .find(|channel| channel.enabled)
                    .ok_or_else(|| {
                        ConnectorError::Execution(format!(
                            "no enabled channel for provider {}",
                            provider.provider_id
                        ))
                    })?
            };

            (provider.clone(), chosen_channel)
        };

        let circuit_phase_before = self.acquire_bridge_circuit_phase().await?;
        let operation = command.operation.clone();
        let payload = command.payload.clone();
        let mut bridge_execution = bridge_execution_payload(
            &provider,
            &chosen_channel,
            &command,
            &self.bridge_runtime_policy,
        )
        .await;
        let bridge_execution_success = !bridge_execution_status_is_failure(&bridge_execution);
        let circuit_observation = self
            .record_bridge_circuit_outcome(bridge_execution_success, &circuit_phase_before)
            .await;
        attach_bridge_circuit_breaker_runtime(
            &mut bridge_execution,
            &self.bridge_runtime_policy.bridge_circuit_breaker,
            &circuit_observation,
        );
        let last_failure_reason = if bridge_execution_success {
            None
        } else {
            bridge_execution_reason(&bridge_execution)
        };
        let persist_result = self
            .persist_plugin_runtime_health(
                &self.bridge_runtime_policy.bridge_circuit_breaker,
                circuit_observation.phase_after.clone(),
                circuit_observation.consecutive_failures,
                circuit_observation.half_open_remaining_calls,
                circuit_observation.half_open_successes,
                last_failure_reason,
            )
            .await;
        if let Err(error) = persist_result {
            let reason = format!("failed to persist plugin runtime health: {error}");
            return Err(ConnectorError::Execution(reason));
        }

        if bridge_execution
            .get("block_class")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "compatibility_contract")
        {
            let reason = bridge_execution
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("plugin compatibility contract blocked bridge execution");
            let reason = format_bridge_execution_failure_reason(reason, &circuit_observation);
            return Err(ConnectorError::Execution(reason));
        }

        if self.bridge_runtime_policy.enforce_execution_success
            && bridge_execution
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "blocked" | "failed"))
        {
            let reason = bridge_execution
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("bridge execution failed under strict runtime policy");
            let reason = format_bridge_execution_failure_reason(reason, &circuit_observation);
            return Err(ConnectorError::Execution(reason));
        }

        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "connector": self.connector_name,
                "provider_id": provider.provider_id,
                "provider_version": provider.version,
                "channel_id": chosen_channel.channel_id,
                "endpoint": chosen_channel.endpoint,
                "operation": operation,
                "payload": payload,
                "bridge_execution": bridge_execution,
            }),
        })
    }
}

pub async fn bridge_execution_payload(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    let bridge_kind = detect_provider_bridge_kind(provider, &channel.endpoint);
    let source_kind = inferred_provider_source_kind(&provider.metadata);
    let source_path = provider_source_path(provider);
    let source_language =
        provider_source_language(&provider.metadata, Some(source_path.as_str()), source_kind);
    let adapter_family = provider
        .metadata
        .get("adapter_family")
        .cloned()
        .unwrap_or_else(|| default_runtime_adapter_family(&source_language, bridge_kind));
    let entrypoint = provider
        .metadata
        .get("entrypoint")
        .or_else(|| provider.metadata.get("entrypoint_hint"))
        .cloned()
        .unwrap_or_else(|| default_bridge_entrypoint(bridge_kind, &channel.endpoint));
    let plugin_compatibility = provider_plugin_compatibility_context(
        provider,
        channel,
        bridge_kind,
        source_kind,
        &source_path,
        &source_language,
        &adapter_family,
        &entrypoint,
        runtime_policy,
    );

    let mut plan = match bridge_kind {
        PluginBridgeKind::HttpJson => {
            let method = provider
                .metadata
                .get("http_method")
                .map(|value| value.to_ascii_uppercase())
                .unwrap_or_else(|| "POST".to_owned());
            json!({
                "status": "planned",
                "bridge_kind": bridge_kind.as_str(),
                "adapter_family": adapter_family,
                "entrypoint": entrypoint,
                "request": {
                    "method": method,
                    "url": channel.endpoint,
                    "operation": command.operation,
                    "payload": command.payload.clone(),
                }
            })
        }
        PluginBridgeKind::ProcessStdio => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "stdio": {
                "stdin_envelope": {
                    "operation": command.operation,
                    "payload": command.payload.clone(),
                },
                "stdout_contract": "json",
            }
        }),
        PluginBridgeKind::NativeFfi => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "ffi": {
                "library": provider
                    .metadata
                    .get("library")
                    .cloned()
                    .unwrap_or_else(|| format!("lib{}.so", provider.provider_id)),
                "symbol": entrypoint,
            }
        }),
        PluginBridgeKind::WasmComponent => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "wasm": {
                "component": provider
                    .metadata
                    .get("component")
                    .cloned()
                    .unwrap_or_else(|| format!("{}.wasm", provider.provider_id)),
                "function": entrypoint,
            }
        }),
        PluginBridgeKind::McpServer => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "mcp": {
                "transport": provider
                    .metadata
                    .get("transport")
                    .cloned()
                    .unwrap_or_else(|| "stdio".to_owned()),
                "handshake": "capability_schema_exchange",
            }
        }),
        PluginBridgeKind::AcpBridge => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "acp": {
                "surface": "bridge",
                "gateway_contract": "external_bridge_runtime",
                "turn_contract": "bridge_forwarded_prompt_response",
            }
        }),
        PluginBridgeKind::AcpRuntime => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "acp": {
                "surface": "runtime",
                "session_bootstrap": "required",
                "control_plane": "external_runtime",
                "turn_contract": "session_scoped_prompt_response",
            }
        }),
        PluginBridgeKind::Unknown => json!({
            "status": "deferred",
            "bridge_kind": bridge_kind.as_str(),
            "reason": "provider metadata does not declare a resolvable bridge_kind",
            "next_action": "set metadata.bridge_kind and rerun bootstrap",
        }),
    };

    if let Some(plugin_compatibility) = plugin_compatibility {
        let blocking_reason = plugin_compatibility.blocking_reason.clone();
        let plugin_compatibility_payload = plugin_compatibility.payload;
        let Some(plan_object) = plan.as_object_mut() else {
            return plan;
        };

        plan_object.insert(
            "plugin_compatibility".to_owned(),
            plugin_compatibility_payload,
        );
        if let Some(reason) = blocking_reason {
            plan_object.insert("status".to_owned(), Value::String("blocked".to_owned()));
            plan_object.insert("reason".to_owned(), Value::String(reason));
            plan_object.insert(
                "block_class".to_owned(),
                Value::String("compatibility_contract".to_owned()),
            );
            return plan;
        }
    }

    maybe_execute_bridge(
        plan,
        bridge_kind,
        provider,
        channel,
        command,
        runtime_policy,
    )
    .await
}

fn provider_metadata_optional_string(
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    metadata
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn provider_plugin_compatibility_mode(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginCompatibilityMode> {
    provider_metadata_optional_string(metadata, "plugin_compatibility_mode").and_then(|value| {
        match value.as_str() {
            "native" => Some(PluginCompatibilityMode::Native),
            "openclaw_modern" => Some(PluginCompatibilityMode::OpenClawModern),
            "openclaw_legacy" => Some(PluginCompatibilityMode::OpenClawLegacy),
            _ => None,
        }
    })
}

fn provider_plugin_dialect(metadata: &BTreeMap<String, String>) -> Option<PluginContractDialect> {
    provider_metadata_optional_string(metadata, "plugin_dialect").and_then(|value| {
        match value.as_str() {
            "loongclaw_package_manifest" => Some(PluginContractDialect::LoongClawPackageManifest),
            "loongclaw_embedded_source" => Some(PluginContractDialect::LoongClawEmbeddedSource),
            "openclaw_modern_manifest" => Some(PluginContractDialect::OpenClawModernManifest),
            "openclaw_legacy_package" => Some(PluginContractDialect::OpenClawLegacyPackage),
            _ => None,
        }
    })
}

fn provider_plugin_source_kind(metadata: &BTreeMap<String, String>) -> Option<PluginSourceKind> {
    provider_metadata_optional_string(metadata, "plugin_source_kind").and_then(|value| match value
        .as_str()
    {
        "package_manifest" => Some(PluginSourceKind::PackageManifest),
        "embedded_source" => Some(PluginSourceKind::EmbeddedSource),
        _ => None,
    })
}

fn provider_plugin_compatibility_shim(
    metadata: &BTreeMap<String, String>,
    compatibility_mode: Option<PluginCompatibilityMode>,
) -> Option<PluginCompatibilityShim> {
    let shim_id = provider_metadata_optional_string(metadata, "plugin_compatibility_shim_id");
    let family = provider_metadata_optional_string(metadata, "plugin_compatibility_shim_family");

    match (shim_id, family) {
        (None, None) => compatibility_mode.and_then(PluginCompatibilityShim::for_mode),
        (Some(shim_id), None) => Some(PluginCompatibilityShim {
            family: shim_id.clone(),
            shim_id,
        }),
        (None, Some(family)) => Some(PluginCompatibilityShim {
            shim_id: family.clone(),
            family,
        }),
        (Some(shim_id), Some(family)) => Some(PluginCompatibilityShim { shim_id, family }),
    }
}

fn provider_plugin_contract_compatibility(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginCompatibility> {
    let host_api = provider_metadata_optional_string(metadata, "plugin_compatibility_host_api");
    let host_version_req =
        provider_metadata_optional_string(metadata, "plugin_compatibility_host_version_req");

    if host_api.is_none() && host_version_req.is_none() {
        return None;
    }

    Some(PluginCompatibility {
        host_api,
        host_version_req,
    })
}

pub(crate) fn provider_is_plugin_backed(metadata: &BTreeMap<String, String>) -> bool {
    [
        "plugin_id",
        "plugin_source_path",
        "plugin_manifest_api_version",
        "plugin_dialect",
        "plugin_compatibility_mode",
    ]
    .iter()
    .any(|key| metadata.contains_key(*key))
}

fn inferred_provider_source_kind(metadata: &BTreeMap<String, String>) -> PluginSourceKind {
    provider_plugin_source_kind(metadata)
        .or_else(|| {
            provider_plugin_dialect(metadata).map(|dialect| match dialect {
                PluginContractDialect::LoongClawPackageManifest
                | PluginContractDialect::OpenClawModernManifest
                | PluginContractDialect::OpenClawLegacyPackage => PluginSourceKind::PackageManifest,
                PluginContractDialect::LoongClawEmbeddedSource => PluginSourceKind::EmbeddedSource,
            })
        })
        .or_else(|| {
            provider_metadata_optional_string(metadata, "plugin_package_manifest_path")
                .map(|_| PluginSourceKind::PackageManifest)
        })
        .unwrap_or(PluginSourceKind::EmbeddedSource)
}

fn provider_source_path(provider: &kernel::ProviderConfig) -> String {
    provider_metadata_optional_string(&provider.metadata, "plugin_source_path")
        .unwrap_or_else(|| format!("provider://{}", provider.provider_id))
}

pub(crate) fn provider_activation_runtime_contract_state(
    metadata: &BTreeMap<String, String>,
) -> ProviderActivationRuntimeContractState {
    let raw_contract = metadata
        .get(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY)
        .cloned();
    let checksum = provider_metadata_optional_string(
        metadata,
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY,
    )
    .map(|value| value.to_ascii_lowercase());
    let metadata_present = raw_contract.is_some() || checksum.is_some();

    let Some(raw_contract) = raw_contract else {
        return ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: checksum.clone(),
            computed_checksum: None,
            integrity_issue: checksum.as_ref().map(|_| {
                "plugin activation contract metadata declares an attested checksum but no activation contract payload".to_owned()
            }),
        };
    };

    let computed_checksum = activation_runtime_contract_checksum_hex(raw_contract.as_bytes());
    let Some(checksum) = checksum else {
        return ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: None,
            computed_checksum: Some(computed_checksum),
            integrity_issue: Some(
                "plugin activation contract metadata is missing attested checksum".to_owned(),
            ),
        };
    };

    if checksum != computed_checksum {
        return ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: Some(checksum.clone()),
            computed_checksum: Some(computed_checksum.clone()),
            integrity_issue: Some(format!(
                "plugin activation contract checksum mismatch: metadata declares `{checksum}` but payload hashes to `{computed_checksum}`"
            )),
        };
    }

    match parse_plugin_activation_runtime_contract(&raw_contract) {
        Ok(contract) => ProviderActivationRuntimeContractState {
            metadata_present,
            contract: Some(contract),
            checksum: Some(checksum),
            computed_checksum: Some(computed_checksum),
            integrity_issue: None,
        },
        Err(error) => ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: Some(checksum),
            computed_checksum: Some(computed_checksum),
            integrity_issue: Some(format!(
                "plugin activation contract payload is invalid: {error}"
            )),
        },
    }
}

pub(crate) fn normalize_runtime_source_language(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust".to_owned(),
        "py" => "python".to_owned(),
        "js" | "mjs" | "cjs" | "cts" | "mts" | "jsx" => "javascript".to_owned(),
        "ts" | "tsx" => "typescript".to_owned(),
        "go" => "go".to_owned(),
        "wasm" => "wasm".to_owned(),
        "manifest" => "manifest".to_owned(),
        "unknown" | "" => "unknown".to_owned(),
        other => other.to_owned(),
    }
}

fn provider_source_language(
    metadata: &BTreeMap<String, String>,
    source_path: Option<&str>,
    source_kind: PluginSourceKind,
) -> String {
    provider_metadata_optional_string(metadata, "source_language")
        .map(|value| normalize_runtime_source_language(&value))
        .unwrap_or_else(|| {
            if matches!(source_kind, PluginSourceKind::PackageManifest) {
                return "manifest".to_owned();
            }

            source_path
                .and_then(|path| Path::new(path).extension().and_then(|ext| ext.to_str()))
                .map(normalize_runtime_source_language)
                .unwrap_or_else(|| "unknown".to_owned())
        })
}

pub(crate) fn default_runtime_adapter_family(
    source_language: &str,
    bridge_kind: PluginBridgeKind,
) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => "http-adapter".to_owned(),
        PluginBridgeKind::ProcessStdio => format!("{source_language}-stdio-adapter"),
        PluginBridgeKind::NativeFfi => format!("{source_language}-ffi-adapter"),
        PluginBridgeKind::WasmComponent => "wasm-component-adapter".to_owned(),
        PluginBridgeKind::McpServer => "mcp-adapter".to_owned(),
        PluginBridgeKind::AcpBridge => "acp-bridge-adapter".to_owned(),
        PluginBridgeKind::AcpRuntime => "acp-runtime-adapter".to_owned(),
        PluginBridgeKind::Unknown => format!("{source_language}-unknown-adapter"),
    }
}

fn projected_plugin_activation_runtime_contract(
    provider: &kernel::ProviderConfig,
    source_path: &str,
    source_kind: PluginSourceKind,
    source_language: &str,
    bridge_kind: PluginBridgeKind,
    adapter_family: &str,
    entrypoint_hint: &str,
) -> Option<PluginActivationRuntimeContract> {
    let dialect = provider_plugin_dialect(&provider.metadata)?;
    let compatibility_mode = provider_plugin_compatibility_mode(&provider.metadata)?;

    Some(PluginActivationRuntimeContract {
        plugin_id: provider_metadata_optional_string(&provider.metadata, "plugin_id")
            .unwrap_or_else(|| provider.provider_id.clone()),
        source_path: source_path.to_owned(),
        source_kind,
        dialect,
        dialect_version: provider_metadata_optional_string(
            &provider.metadata,
            "plugin_dialect_version",
        ),
        compatibility_mode,
        compatibility_shim: provider_plugin_compatibility_shim(
            &provider.metadata,
            Some(compatibility_mode),
        ),
        bridge_kind,
        adapter_family: adapter_family.to_owned(),
        entrypoint_hint: entrypoint_hint.to_owned(),
        source_language: source_language.to_owned(),
        compatibility: provider_plugin_contract_compatibility(&provider.metadata),
    })
}

fn canonical_dialect_for_mode(
    compatibility_mode: PluginCompatibilityMode,
    source_kind: PluginSourceKind,
) -> PluginContractDialect {
    match compatibility_mode {
        PluginCompatibilityMode::Native => match source_kind {
            PluginSourceKind::PackageManifest => PluginContractDialect::LoongClawPackageManifest,
            PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongClawEmbeddedSource,
        },
        PluginCompatibilityMode::OpenClawModern => PluginContractDialect::OpenClawModernManifest,
        PluginCompatibilityMode::OpenClawLegacy => PluginContractDialect::OpenClawLegacyPackage,
    }
}

fn provider_plugin_compatibility_projection_issue(
    is_plugin_backed: bool,
    dialect: Option<PluginContractDialect>,
    compatibility_mode: Option<PluginCompatibilityMode>,
    compatibility_shim: Option<&PluginCompatibilityShim>,
    source_kind: PluginSourceKind,
) -> Option<String> {
    if !is_plugin_backed
        && dialect.is_none()
        && compatibility_mode.is_none()
        && compatibility_shim.is_none()
    {
        return None;
    }

    let Some(compatibility_mode) = compatibility_mode else {
        return Some(
            "plugin-backed provider metadata drifted: missing `plugin_compatibility_mode`"
                .to_owned(),
        );
    };

    let Some(dialect) = dialect else {
        return Some(
            "plugin-backed provider metadata drifted: missing `plugin_dialect`".to_owned(),
        );
    };

    let canonical_dialect = canonical_dialect_for_mode(compatibility_mode, source_kind);
    if dialect != canonical_dialect {
        return Some(format!(
            "plugin compatibility projection drifted: mode `{}` expects canonical dialect `{}` but provider metadata declares `{}`",
            compatibility_mode.as_str(),
            canonical_dialect.as_str(),
            dialect.as_str()
        ));
    }

    let canonical_shim = PluginCompatibilityShim::for_mode(compatibility_mode);
    match (canonical_shim.as_ref(), compatibility_shim) {
        (None, Some(actual)) => {
            return Some(format!(
                "plugin compatibility projection drifted: native mode must not declare compatibility shim `{}` ({})",
                actual.shim_id, actual.family
            ));
        }
        (Some(expected), None) => {
            return Some(format!(
                "plugin compatibility projection drifted: mode `{}` requires canonical compatibility shim `{}` ({})",
                compatibility_mode.as_str(),
                expected.shim_id,
                expected.family
            ));
        }
        (Some(expected), Some(actual)) if actual != expected => {
            return Some(format!(
                "plugin compatibility projection drifted: mode `{}` expects compatibility shim `{}` ({}) but provider metadata declares `{}` ({})",
                compatibility_mode.as_str(),
                expected.shim_id,
                expected.family,
                actual.shim_id,
                actual.family
            ));
        }
        _ => {}
    }

    None
}

fn format_plugin_compatibility_shim(shim: Option<&PluginCompatibilityShim>) -> String {
    shim.map(|shim| format!("`{}` ({})", shim.shim_id, shim.family))
        .unwrap_or_else(|| "none".to_owned())
}

fn format_plugin_contract_compatibility(compatibility: Option<&PluginCompatibility>) -> String {
    compatibility
        .map(|compatibility| {
            format!(
                "host_api={}, host_version_req={}",
                compatibility.host_api.as_deref().unwrap_or("none"),
                compatibility.host_version_req.as_deref().unwrap_or("none")
            )
        })
        .unwrap_or_else(|| "none".to_owned())
}

fn activation_runtime_contract_drift_issue(
    attested: &PluginActivationRuntimeContract,
    current: Option<&PluginActivationRuntimeContract>,
    self_projection_issue: Option<&str>,
) -> Option<String> {
    if let Some(issue) = self_projection_issue {
        return Some(format!(
            "plugin activation contract drifted after registration: {issue}"
        ));
    }

    let Some(current) = current else {
        return Some(
            "plugin activation contract drifted after registration: current provider metadata no longer projects a complete plugin runtime contract"
                .to_owned(),
        );
    };

    if current.plugin_id != attested.plugin_id {
        return Some(format!(
            "plugin activation contract drifted after registration: approved plugin_id `{}` but current projection resolves `{}`",
            attested.plugin_id, current.plugin_id
        ));
    }
    if current.source_path != attested.source_path {
        return Some(format!(
            "plugin activation contract drifted after registration: approved source_path `{}` but current projection resolves `{}`",
            attested.source_path, current.source_path
        ));
    }
    if current.source_kind != attested.source_kind {
        return Some(format!(
            "plugin activation contract drifted after registration: approved source_kind `{}` but current projection resolves `{}`",
            attested.source_kind.as_str(),
            current.source_kind.as_str()
        ));
    }
    if current.dialect != attested.dialect {
        return Some(format!(
            "plugin activation contract drifted after registration: approved dialect `{}` but current projection resolves `{}`",
            attested.dialect.as_str(),
            current.dialect.as_str()
        ));
    }
    if current.dialect_version != attested.dialect_version {
        return Some(format!(
            "plugin activation contract drifted after registration: approved dialect_version `{}` but current projection resolves `{}`",
            attested.dialect_version.as_deref().unwrap_or("none"),
            current.dialect_version.as_deref().unwrap_or("none")
        ));
    }
    if current.compatibility_mode != attested.compatibility_mode {
        return Some(format!(
            "plugin activation contract drifted after registration: approved compatibility_mode `{}` but current projection resolves `{}`",
            attested.compatibility_mode.as_str(),
            current.compatibility_mode.as_str()
        ));
    }
    if current.compatibility_shim != attested.compatibility_shim {
        return Some(format!(
            "plugin activation contract drifted after registration: approved compatibility_shim {} but current projection resolves {}",
            format_plugin_compatibility_shim(attested.compatibility_shim.as_ref()),
            format_plugin_compatibility_shim(current.compatibility_shim.as_ref())
        ));
    }
    if current.bridge_kind != attested.bridge_kind {
        return Some(format!(
            "plugin activation contract drifted after registration: approved bridge_kind `{}` but current projection resolves `{}`",
            attested.bridge_kind.as_str(),
            current.bridge_kind.as_str()
        ));
    }
    if current.adapter_family != attested.adapter_family {
        return Some(format!(
            "plugin activation contract drifted after registration: approved adapter_family `{}` but current projection resolves `{}`",
            attested.adapter_family, current.adapter_family
        ));
    }
    if current.entrypoint_hint != attested.entrypoint_hint {
        return Some(format!(
            "plugin activation contract drifted after registration: approved entrypoint_hint `{}` but current projection resolves `{}`",
            attested.entrypoint_hint, current.entrypoint_hint
        ));
    }
    if current.source_language != attested.source_language {
        return Some(format!(
            "plugin activation contract drifted after registration: approved source_language `{}` but current projection resolves `{}`",
            attested.source_language, current.source_language
        ));
    }
    if current.compatibility != attested.compatibility {
        return Some(format!(
            "plugin activation contract drifted after registration: approved compatibility `{}` but current projection resolves `{}`",
            format_plugin_contract_compatibility(attested.compatibility.as_ref()),
            format_plugin_contract_compatibility(current.compatibility.as_ref())
        ));
    }

    None
}

fn shim_support_profile_mismatch_reasons(
    profile: &PluginCompatibilityShimSupport,
    ir: &PluginIR,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if !profile.supported_dialects.is_empty() && !profile.supported_dialects.contains(&ir.dialect) {
        reasons.push(format!("dialect `{}`", ir.dialect.as_str()));
    }

    if !profile.supported_bridges.is_empty()
        && !profile.supported_bridges.contains(&ir.runtime.bridge_kind)
    {
        reasons.push(format!("bridge kind `{}`", ir.runtime.bridge_kind.as_str()));
    }

    if !profile.supported_adapter_families.is_empty()
        && !profile
            .supported_adapter_families
            .contains(&ir.runtime.adapter_family.trim().to_ascii_lowercase())
    {
        reasons.push(format!("adapter family `{}`", ir.runtime.adapter_family));
    }

    if !profile.supported_source_languages.is_empty()
        && !profile
            .supported_source_languages
            .contains(&ir.runtime.source_language)
    {
        reasons.push(format!("source language `{}`", ir.runtime.source_language));
    }

    reasons
}

fn provider_plugin_host_compatibility_issue(
    compatibility: Option<&PluginCompatibility>,
) -> Option<String> {
    let compatibility = compatibility?;

    if let Some(host_api) = compatibility.host_api.as_deref()
        && host_api != CURRENT_PLUGIN_HOST_API
    {
        return Some(format!(
            "plugin compatibility.host_api `{host_api}` is not supported by current host api `{CURRENT_PLUGIN_HOST_API}`"
        ));
    }

    if let Some(host_version_req) = compatibility.host_version_req.as_deref() {
        let parsed_req = match VersionReq::parse(host_version_req) {
            Ok(parsed_req) => parsed_req,
            Err(error) => {
                return Some(format!(
                    "plugin compatibility.host_version_req `{host_version_req}` is invalid: {error}"
                ));
            }
        };
        let current_version_string = env!("CARGO_PKG_VERSION");
        let current_version = match Version::parse(current_version_string) {
            Ok(current_version) => current_version,
            Err(error) => {
                return Some(format!(
                    "current host version `{current_version_string}` is invalid semver: {error}"
                ));
            }
        };
        if !parsed_req.matches(&current_version) {
            return Some(format!(
                "plugin compatibility.host_version_req `{host_version_req}` does not match current host version `{current_version}`"
            ));
        }
    }

    None
}

fn plugin_ir_from_runtime_contract(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    contract: &PluginActivationRuntimeContract,
) -> PluginIR {
    PluginIR {
        manifest_api_version: provider_metadata_optional_string(
            &provider.metadata,
            "plugin_manifest_api_version",
        ),
        plugin_version: provider_metadata_optional_string(&provider.metadata, "plugin_version")
            .or_else(|| provider_metadata_optional_string(&provider.metadata, "version"))
            .or_else(|| Some(provider.version.clone())),
        dialect: contract.dialect,
        dialect_version: contract.dialect_version.clone(),
        compatibility_mode: contract.compatibility_mode,
        plugin_id: contract.plugin_id.clone(),
        provider_id: provider.provider_id.clone(),
        connector_name: provider.connector_name.clone(),
        channel_id: Some(channel.channel_id.clone()),
        endpoint: Some(channel.endpoint.clone()),
        capabilities: BTreeSet::new(),
        trust_tier: kernel::PluginTrustTier::default(),
        metadata: provider.metadata.clone(),
        source_path: contract.source_path.clone(),
        source_kind: contract.source_kind,
        package_root: provider_metadata_optional_string(&provider.metadata, "plugin_package_root")
            .unwrap_or_else(|| contract.source_path.clone()),
        package_manifest_path: provider_metadata_optional_string(
            &provider.metadata,
            "plugin_package_manifest_path",
        ),
        diagnostic_findings: Vec::new(),
        setup: None,
        slot_claims: Vec::new(),
        compatibility: contract.compatibility.clone(),
        runtime: PluginRuntimeProfile {
            source_language: contract.source_language.clone(),
            bridge_kind: contract.bridge_kind,
            adapter_family: contract.adapter_family.clone(),
            entrypoint_hint: contract.entrypoint_hint.clone(),
        },
    }
}

fn shim_support_profile_payload(profile: &PluginCompatibilityShimSupport) -> Value {
    let mut payload = Map::new();

    if let Some(version) = &profile.version {
        let version = version.clone();

        payload.insert("version".to_owned(), Value::String(version));
    }
    if !profile.supported_dialects.is_empty() {
        let supported_dialects = profile
            .supported_dialects
            .iter()
            .map(|dialect| Value::String(dialect.as_str().to_owned()))
            .collect();

        payload.insert(
            "supported_dialects".to_owned(),
            Value::Array(supported_dialects),
        );
    }
    if !profile.supported_bridges.is_empty() {
        let supported_bridges = profile
            .supported_bridges
            .iter()
            .map(|bridge| Value::String(bridge.as_str().to_owned()))
            .collect();

        payload.insert(
            "supported_bridges".to_owned(),
            Value::Array(supported_bridges),
        );
    }
    if !profile.supported_adapter_families.is_empty() {
        let supported_adapter_families = profile
            .supported_adapter_families
            .iter()
            .map(|family| Value::String(family.clone()))
            .collect();

        payload.insert(
            "supported_adapter_families".to_owned(),
            Value::Array(supported_adapter_families),
        );
    }
    if !profile.supported_source_languages.is_empty() {
        let supported_source_languages = profile
            .supported_source_languages
            .iter()
            .map(|language| Value::String(language.clone()))
            .collect();

        payload.insert(
            "supported_source_languages".to_owned(),
            Value::Array(supported_source_languages),
        );
    }

    Value::Object(payload)
}

fn provider_plugin_compatibility_context(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    bridge_kind: PluginBridgeKind,
    source_kind: PluginSourceKind,
    source_path: &str,
    source_language: &str,
    adapter_family: &str,
    entrypoint: &str,
    runtime_policy: &BridgeRuntimePolicy,
) -> Option<ProviderPluginCompatibilityContext> {
    let current_dialect = provider_plugin_dialect(&provider.metadata);
    let current_dialect_version =
        provider_metadata_optional_string(&provider.metadata, "plugin_dialect_version");
    let current_compatibility_mode = provider_plugin_compatibility_mode(&provider.metadata);
    let current_compatibility_shim =
        provider_plugin_compatibility_shim(&provider.metadata, current_compatibility_mode);
    let current_compatibility = provider_plugin_contract_compatibility(&provider.metadata);
    let attestation_state = provider_activation_runtime_contract_state(&provider.metadata);
    let attested_contract = attestation_state.contract.as_ref();
    let attested_contract_checksum = attestation_state.checksum.clone();
    let attested_contract_computed_checksum = attestation_state.computed_checksum.clone();
    let attestation_integrity_issue = attestation_state.integrity_issue.clone();
    let current_contract = projected_plugin_activation_runtime_contract(
        provider,
        source_path,
        source_kind,
        source_language,
        bridge_kind,
        adapter_family,
        entrypoint,
    );
    let effective_contract = attested_contract.or(current_contract.as_ref());
    let shim_support_profile = effective_contract
        .and_then(|contract| {
            runtime_policy
                .compatibility_matrix
                .compatibility_shim_support_profile(contract.compatibility_shim.as_ref())
        })
        .cloned();
    let is_plugin_backed =
        provider_is_plugin_backed(&provider.metadata) || attestation_state.metadata_present;

    if !is_plugin_backed
        && current_dialect.is_none()
        && current_dialect_version.is_none()
        && current_compatibility_mode.is_none()
        && current_compatibility_shim.is_none()
        && current_compatibility.is_none()
        && shim_support_profile.is_none()
    {
        return None;
    }

    let projection_issue = provider_plugin_compatibility_projection_issue(
        is_plugin_backed,
        current_dialect,
        current_compatibility_mode,
        current_compatibility_shim.as_ref(),
        source_kind,
    );
    let activation_contract_drift_issue = attested_contract.as_ref().and_then(|contract| {
        activation_runtime_contract_drift_issue(
            contract,
            current_contract.as_ref(),
            projection_issue.as_deref(),
        )
    });

    let runtime_ir = effective_contract
        .map(|contract| plugin_ir_from_runtime_contract(provider, channel, contract));

    let shim_support_mismatch_reasons = runtime_ir
        .as_ref()
        .zip(shim_support_profile.as_ref())
        .map(|(ir, profile)| shim_support_profile_mismatch_reasons(profile, ir))
        .unwrap_or_default();

    let blocking_reason = attestation_integrity_issue.clone()
        .or(activation_contract_drift_issue)
        .or_else(|| {
            let contract = effective_contract?;
            let compatibility_mode = contract.compatibility_mode;
            if !runtime_policy
                .compatibility_matrix
                .is_compatibility_mode_supported(compatibility_mode)
            {
                let shim_clause = contract
                    .compatibility_shim
                    .as_ref()
                    .map(|shim| format!(" via shim `{}` ({})", shim.shim_id, shim.family))
                    .unwrap_or_default();
                return Some(format!(
                    "compatibility mode {} requires a host shim that is not enabled in the current runtime matrix{}",
                    compatibility_mode.as_str(),
                    shim_clause
                ));
            }

            if !runtime_policy
                .compatibility_matrix
                .is_compatibility_shim_supported(contract.compatibility_shim.as_ref())
            {
                let shim = match contract.compatibility_shim.as_ref() {
                    Some(shim) => shim,
                    None => {
                        return Some(format!(
                            "compatibility mode {} requires a canonical compatibility shim, but none was resolved in the activation contract",
                            compatibility_mode.as_str()
                        ));
                    }
                };
                return Some(format!(
                    "compatibility mode {} requires compatibility shim `{}` ({}) that is not enabled in the current runtime matrix",
                    compatibility_mode.as_str(),
                    shim.shim_id,
                    shim.family
                ));
            }

            if let Some(reason) =
                provider_plugin_host_compatibility_issue(contract.compatibility.as_ref())
            {
                return Some(reason);
            }

            let ir = runtime_ir.as_ref()?;
            if let Some(reason) = runtime_policy
                .compatibility_matrix
                .compatibility_shim_support_issue(ir, contract.compatibility_shim.as_ref())
            {
                return Some(reason);
            }

            if !runtime_policy
                .compatibility_matrix
                .is_bridge_supported(contract.bridge_kind)
            {
                return Some(format!(
                    "bridge kind {} is not supported by current runtime matrix",
                    contract.bridge_kind.as_str()
                ));
            }

            if !runtime_policy
                .compatibility_matrix
                .is_adapter_family_supported(&contract.adapter_family)
            {
                return Some(format!(
                    "adapter family {} is not supported by current runtime matrix",
                    contract.adapter_family
                ));
            }

            None
        })
        .or_else(|| {
            effective_contract
                .and_then(|contract| provider_plugin_host_compatibility_issue(contract.compatibility.as_ref()))
        })
        .or_else(|| projection_issue.clone());

    let mut payload = Map::new();
    if let Some(contract) = effective_contract {
        let dialect = contract.dialect.as_str().to_owned();

        payload.insert("dialect".to_owned(), Value::String(dialect));
        if let Some(dialect_version) = &contract.dialect_version {
            let dialect_version = dialect_version.clone();

            payload.insert("dialect_version".to_owned(), Value::String(dialect_version));
        }
        let compatibility_mode = contract.compatibility_mode.as_str().to_owned();

        payload.insert("mode".to_owned(), Value::String(compatibility_mode));
        if let Some(compatibility_shim) = &contract.compatibility_shim {
            let mut shim_payload = Map::new();
            let shim_id = compatibility_shim.shim_id.clone();
            let family = compatibility_shim.family.clone();

            shim_payload.insert("shim_id".to_owned(), Value::String(shim_id));
            shim_payload.insert("family".to_owned(), Value::String(family));
            payload.insert("shim".to_owned(), Value::Object(shim_payload));
        }
        if let Some(compatibility) = &contract.compatibility {
            if let Some(host_api) = &compatibility.host_api {
                let host_api = host_api.clone();

                payload.insert("host_api".to_owned(), Value::String(host_api));
            }
            if let Some(host_version_req) = &compatibility.host_version_req {
                let host_version_req = host_version_req.clone();

                payload.insert(
                    "host_version_req".to_owned(),
                    Value::String(host_version_req),
                );
            }
        }
    }
    if let Some(shim_support_profile) = shim_support_profile {
        let shim_support = shim_support_profile_payload(&shim_support_profile);

        payload.insert("shim_support".to_owned(), shim_support);
    }
    if !shim_support_mismatch_reasons.is_empty() {
        let mismatch_reasons = shim_support_mismatch_reasons
            .iter()
            .cloned()
            .map(Value::String)
            .collect();

        payload.insert(
            "shim_support_mismatch_reasons".to_owned(),
            Value::Array(mismatch_reasons),
        );
    }
    if attestation_integrity_issue.is_none()
        && let Some(contract) = attested_contract
    {
        let activation_contract = plugin_activation_runtime_contract_value(contract);

        payload.insert("activation_contract".to_owned(), activation_contract);
    }
    if let Some(checksum) = attested_contract_checksum {
        payload.insert(
            "activation_contract_checksum".to_owned(),
            Value::String(checksum),
        );
    }

    let mut runtime_projection = Map::new();
    let source_path = source_path.to_owned();
    let source_kind = source_kind.as_str().to_owned();
    let source_language = source_language.to_owned();
    let bridge_kind = bridge_kind.as_str().to_owned();
    let adapter_family = adapter_family.to_owned();
    let entrypoint_hint = entrypoint.to_owned();

    runtime_projection.insert("source_path".to_owned(), Value::String(source_path));
    runtime_projection.insert("source_kind".to_owned(), Value::String(source_kind));
    runtime_projection.insert("source_language".to_owned(), Value::String(source_language));
    runtime_projection.insert("bridge_kind".to_owned(), Value::String(bridge_kind));
    runtime_projection.insert("adapter_family".to_owned(), Value::String(adapter_family));
    runtime_projection.insert("entrypoint_hint".to_owned(), Value::String(entrypoint_hint));
    if let Some(dialect) = current_dialect {
        let dialect = dialect.as_str().to_owned();

        runtime_projection.insert("dialect".to_owned(), Value::String(dialect));
    }
    if let Some(dialect_version) = current_dialect_version {
        runtime_projection.insert("dialect_version".to_owned(), Value::String(dialect_version));
    }
    if let Some(compatibility_mode) = current_compatibility_mode {
        let compatibility_mode = compatibility_mode.as_str().to_owned();

        runtime_projection.insert("mode".to_owned(), Value::String(compatibility_mode));
    }
    if let Some(compatibility_shim) = current_compatibility_shim {
        let mut shim_payload = Map::new();
        let shim_id = compatibility_shim.shim_id;
        let family = compatibility_shim.family;

        shim_payload.insert("shim_id".to_owned(), Value::String(shim_id));
        shim_payload.insert("family".to_owned(), Value::String(family));
        runtime_projection.insert("shim".to_owned(), Value::Object(shim_payload));
    }
    payload.insert(
        "runtime_projection".to_owned(),
        Value::Object(runtime_projection),
    );

    let runtime_guard_status = if blocking_reason.is_some() {
        "blocked".to_owned()
    } else {
        "passed".to_owned()
    };
    let activation_contract_attested = attestation_state.metadata_present;
    let activation_contract_verified =
        attested_contract.is_some() && attestation_integrity_issue.is_none();
    let activation_contract_integrity = if !attestation_state.metadata_present {
        "missing".to_owned()
    } else if attestation_integrity_issue.is_some() {
        "invalid".to_owned()
    } else {
        "verified".to_owned()
    };
    let mut runtime_guard = Map::new();

    runtime_guard.insert("status".to_owned(), Value::String(runtime_guard_status));
    runtime_guard.insert(
        "kind".to_owned(),
        Value::String("compatibility_contract".to_owned()),
    );
    runtime_guard.insert(
        "activation_contract_attested".to_owned(),
        Value::Bool(activation_contract_attested),
    );
    runtime_guard.insert(
        "activation_contract_verified".to_owned(),
        Value::Bool(activation_contract_verified),
    );
    runtime_guard.insert(
        "activation_contract_integrity".to_owned(),
        Value::String(activation_contract_integrity),
    );
    if let Some(checksum) = attested_contract_computed_checksum {
        runtime_guard.insert(
            "activation_contract_computed_checksum".to_owned(),
            Value::String(checksum),
        );
    }
    if let Some(issue) = attestation_integrity_issue.as_ref() {
        let issue = issue.clone();

        runtime_guard.insert(
            "activation_contract_integrity_issue".to_owned(),
            Value::String(issue),
        );
    }
    if let Some(reason) = blocking_reason.as_ref() {
        let reason = reason.clone();

        runtime_guard.insert("reason".to_owned(), Value::String(reason));
    }
    payload.insert("runtime_guard".to_owned(), Value::Object(runtime_guard));

    Some(ProviderPluginCompatibilityContext {
        payload: Value::Object(payload),
        blocking_reason,
    })
}

pub async fn maybe_execute_bridge(
    execution: Value,
    bridge_kind: PluginBridgeKind,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    if runtime_policy.execute_http_json && matches!(bridge_kind, PluginBridgeKind::HttpJson) {
        return execute_http_json_bridge(execution, provider, channel, command);
    }

    if runtime_policy.execute_process_stdio && matches!(bridge_kind, PluginBridgeKind::ProcessStdio)
    {
        return execute_process_stdio_bridge(execution, provider, channel, command, runtime_policy)
            .await;
    }

    if runtime_policy.execute_wasm_component
        && matches!(bridge_kind, PluginBridgeKind::WasmComponent)
    {
        return execute_wasm_component_bridge(
            execution,
            provider,
            channel,
            command,
            runtime_policy,
        );
    }

    execution
}
