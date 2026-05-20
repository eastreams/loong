use super::*;
pub struct RuntimeSnapshotCliState {
    pub config: String,
    pub provider: RuntimeSnapshotProviderState,
    pub context_engine: mvp::conversation::ContextEngineRuntimeSnapshot,
    pub compaction_hygiene: RuntimeSnapshotCompactionHygieneState,
    pub memory_system: mvp::memory::MemorySystemRuntimeSnapshot,
    pub acp: mvp::acp::AcpRuntimeSnapshot,
    pub enabled_channel_ids: Vec<String>,
    pub enabled_runtime_backed_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub enabled_plugin_backed_channel_ids: Vec<String>,
    pub enabled_outbound_only_channel_ids: Vec<String>,
    pub channels: mvp::channel::ChannelInventory,
    pub tool_runtime: mvp::tools::runtime_config::ToolRuntimeConfig,
    pub tool_access: RuntimeToolAccessSummary,
    pub visible_tool_names: Vec<String>,
    pub discoverable_tool_summary: mvp::tools::DiscoverableToolSurfaceSummary,
    pub capability_snapshot: String,
    pub capability_snapshot_sha256: String,
    pub tool_calling: RuntimeSnapshotToolCallingState,
    pub runtime_plugins: RuntimeSnapshotRuntimePluginsState,
    pub skills: RuntimeSnapshotSkillsState,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSnapshotInventoryStatus {
    Ok,
    Disabled,
    Error,
}

impl RuntimeSnapshotInventoryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Disabled => "disabled",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotSkillsState {
    pub policy: mvp::tools::runtime_config::SkillsRuntimePolicy,
    pub override_active: bool,
    pub inventory_status: RuntimeSnapshotInventoryStatus,
    pub inventory_error: Option<String>,
    pub inventory: Value,
    pub resolved_skill_count: usize,
    pub shadowed_skill_count: usize,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotRuntimePluginsState {
    pub enabled: bool,
    pub roots: Vec<String>,
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub inventory_status: RuntimeSnapshotInventoryStatus,
    pub inventory_error: Option<String>,
    pub readiness_evaluation: String,
    pub scanned_root_count: usize,
    pub scanned_file_count: usize,
    pub discovered_plugin_count: usize,
    pub translated_plugin_count: usize,
    pub ready_plugin_count: usize,
    pub setup_incomplete_plugin_count: usize,
    pub blocked_plugin_count: usize,
    pub plugins: Vec<RuntimeSnapshotRuntimePluginState>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotRuntimePluginState {
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub source_path: String,
    pub source_kind: String,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: String,
    pub adapter_family: String,
    pub setup_mode: Option<String>,
    pub setup_surface: Option<String>,
    pub slot_claims: Vec<String>,
    pub conflicting_slot_claims: Vec<String>,
    pub status: String,
    pub reason: String,
    pub missing_required_env_vars: Vec<String>,
    pub missing_required_config_keys: Vec<String>,
}

pub(crate) use runtime_access::{
    RUNTIME_TOOL_ACCESS_SEPARATION_NOTE, RuntimeToolAccessSummary, runtime_tool_access_summary,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotArtifactMetadata {
    pub created_at: String,
    pub label: Option<String>,
    pub experiment_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotArtifactLineage {
    pub snapshot_id: String,
    pub created_at: String,
    pub label: Option<String>,
    pub experiment_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshotRestoreSpec {
    pub provider: RuntimeSnapshotRestoreProviderSpec,
    pub conversation: mvp::config::ConversationConfig,
    pub memory: mvp::config::MemoryConfig,
    pub acp: mvp::config::AcpConfig,
    pub tools: mvp::config::ToolConfig,
    pub skills: mvp::config::SkillsConfig,
    #[serde(default)]
    pub runtime_plugins: mvp::config::RuntimePluginsConfig,
    pub managed_skills: RuntimeSnapshotRestoreManagedSkillsSpec,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshotRestoreProviderSpec {
    pub active_provider: Option<String>,
    pub last_provider: Option<String>,
    pub profiles: BTreeMap<String, mvp::config::ProviderProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeSnapshotRestoreManagedSkillsSpec {
    pub skills: Vec<RuntimeSnapshotRestoreManagedSkillSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotRestoreManagedSkillSpec {
    pub skill_id: String,
    pub display_name: String,
    pub summary: String,
    pub source_kind: String,
    pub source_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshotArtifactDocument {
    pub config: String,
    pub schema: RuntimeSnapshotArtifactSchema,
    pub lineage: RuntimeSnapshotArtifactLineage,
    pub provider: Value,
    pub context_engine: Value,
    pub memory_system: Value,
    pub acp: Value,
    pub channels: Value,
    pub tool_runtime: Value,
    pub tools: Value,
    #[serde(default)]
    pub runtime_plugins: Value,
    pub skills: Value,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

pub fn run_runtime_snapshot_cli(
    config_path: Option<&str>,
    as_json: bool,
    output_path: Option<&str>,
    label: Option<&str>,
    experiment_id: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> CliResult<()> {
    let snapshot = collect_runtime_snapshot_cli_state(config_path)?;
    let metadata =
        runtime_snapshot_artifact_metadata_now(label, experiment_id, parent_snapshot_id)?;
    let artifact_payload = build_runtime_snapshot_artifact_json_payload(&snapshot, &metadata)?;

    if let Some(output_path) = output_path {
        persist_json_artifact(output_path, &artifact_payload, "runtime snapshot artifact")?;
    }

    if as_json {
        let pretty = serde_json::to_string_pretty(&artifact_payload).map_err(|error| {
            format!("serialize runtime snapshot artifact output failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_runtime_snapshot_artifact_text(&snapshot, &artifact_payload)
    );
    Ok(())
}

pub fn collect_runtime_snapshot_cli_state(
    config_path: Option<&str>,
) -> CliResult<RuntimeSnapshotCliState> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    collect_runtime_snapshot_cli_state_from_parts(resolved_path.as_path(), &config)
}

pub(crate) fn collect_runtime_snapshot_cli_state_from_loaded_config(
    loaded_config: &supervisor::LoadedSupervisorConfig,
) -> CliResult<RuntimeSnapshotCliState> {
    let resolved_path = loaded_config.resolved_path.as_path();
    let config = &loaded_config.config;
    collect_runtime_snapshot_cli_state_from_parts(resolved_path, config)
}

fn collect_runtime_snapshot_cli_state_from_parts(
    resolved_path: &Path,
    config: &mvp::config::LoongConfig,
) -> CliResult<RuntimeSnapshotCliState> {
    let config_display = resolved_path.display().to_string();
    let provider = collect_runtime_snapshot_provider_state(config);
    let context_engine = mvp::conversation::collect_context_engine_runtime_snapshot(config)?;
    let compaction_hygiene =
        collect_runtime_snapshot_compaction_hygiene_state(config, &context_engine);
    let memory_system = mvp::memory::collect_memory_system_runtime_snapshot(config)?;
    let acp = mvp::acp::collect_acp_runtime_snapshot(config)?;
    let enabled_channel_ids = config.enabled_channel_ids();
    let enabled_runtime_backed_channel_ids = config.enabled_runtime_backed_channel_ids();
    let enabled_service_channel_ids = config.enabled_service_channel_ids();
    let enabled_plugin_backed_channel_ids = config.enabled_plugin_backed_channel_ids();
    let enabled_outbound_only_channel_ids = config.enabled_outbound_only_channel_ids();
    let channels = mvp::channel::channel_inventory(config);
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        config,
        Some(resolved_path),
    );
    let (skills, snapshot_tool_runtime) = collect_runtime_snapshot_skills_state(&tool_runtime);
    let tool_access = runtime_tool_access_summary(config, &snapshot_tool_runtime);
    let tool_view = mvp::tools::runtime_tool_view_for_runtime_config(&snapshot_tool_runtime);
    let visible_tools = tool_view
        .tool_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let discoverable_tool_summary =
        mvp::tools::runtime_discoverable_tool_surface_summary_with_config(
            &snapshot_tool_runtime,
            Some(&tool_view),
        );
    let capability_snapshot = mvp::tools::capability_snapshot_with_config(&snapshot_tool_runtime);
    let capability_snapshot_sha256 =
        runtime_snapshot_tool_digest(&visible_tools, &capability_snapshot)?;
    let tool_calling = collect_runtime_snapshot_tool_calling_state(config, visible_tools.len());
    let runtime_plugins = collect_runtime_snapshot_runtime_plugins_state(config);
    let restore_spec = build_runtime_snapshot_restore_spec(config, &skills);
    Ok(RuntimeSnapshotCliState {
        config: config_display,
        provider,
        context_engine,
        compaction_hygiene,
        memory_system,
        acp,
        enabled_channel_ids,
        enabled_runtime_backed_channel_ids,
        enabled_service_channel_ids,
        enabled_plugin_backed_channel_ids,
        enabled_outbound_only_channel_ids,
        channels,
        tool_runtime: snapshot_tool_runtime,
        tool_access,
        visible_tool_names: visible_tools,
        discoverable_tool_summary,
        capability_snapshot,
        capability_snapshot_sha256,
        tool_calling,
        runtime_plugins,
        skills,
        restore_spec,
    })
}

fn collect_runtime_snapshot_provider_state(
    config: &mvp::config::LoongConfig,
) -> RuntimeSnapshotProviderState {
    let active_profile_id = config
        .active_provider_id()
        .unwrap_or(config.provider.kind.profile().id)
        .to_owned();
    let saved_profile_ids = provider_presentation::saved_provider_profile_ids(config);
    let profiles = if config.providers.is_empty() {
        vec![build_runtime_snapshot_provider_profile_state(
            active_profile_id.as_str(),
            &mvp::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: config.provider.clone(),
            },
            true,
        )]
    } else {
        saved_profile_ids
            .iter()
            .filter_map(|profile_id| {
                config.providers.get(profile_id).map(|profile| {
                    build_runtime_snapshot_provider_profile_state(
                        profile_id,
                        profile,
                        profile_id == &active_profile_id,
                    )
                })
            })
            .collect::<Vec<_>>()
    };

    let transport_metrics = mvp::provider::provider_http_client_runtime_metrics_snapshot();
    let failover_metrics = mvp::provider::provider_failover_metrics_snapshot();
    let transport_runtime = RuntimeSnapshotProviderTransportState {
        http_client_cache_entries: transport_metrics.cache_entry_count,
        http_client_cache_hits: transport_metrics.cache_hit_count,
        http_client_cache_misses: transport_metrics.cache_miss_count,
        built_http_clients: transport_metrics.built_client_count,
        failover_total_events: failover_metrics.total_events,
        failover_continued_events: failover_metrics.continued_events,
        failover_exhausted_events: failover_metrics.exhausted_events,
        failover_by_reason: failover_metrics.by_reason,
        failover_by_stage: failover_metrics.by_stage,
        failover_by_provider: failover_metrics.by_provider,
    };

    RuntimeSnapshotProviderState {
        active_profile_id,
        active_label: provider_presentation::active_provider_detail_label(config),
        last_provider_id: config.last_provider_id().map(str::to_owned),
        saved_profile_ids,
        transport_runtime,
        profiles,
    }
}

fn build_runtime_snapshot_provider_profile_state(
    profile_id: &str,
    profile: &mvp::config::ProviderProfileConfig,
    is_active: bool,
) -> RuntimeSnapshotProviderProfileState {
    let provider = &profile.provider;
    let descriptor = provider.descriptor_document();
    let mut header_names = provider.headers.keys().cloned().collect::<Vec<_>>();
    header_names.sort();

    RuntimeSnapshotProviderProfileState {
        profile_id: profile_id.to_owned(),
        is_active,
        default_for_kind: profile.default_for_kind,
        descriptor,
        kind: provider.kind,
        model: provider.model.clone(),
        wire_api: provider.wire_api,
        base_url: provider.resolved_base_url(),
        endpoint: provider.endpoint(),
        models_endpoint: provider.models_endpoint(),
        protocol_family: provider.kind.profile().protocol_family.as_str(),
        credential_resolved: runtime_snapshot_provider_credentials_resolved(provider),
        auth_env: provider.resolved_auth_env_name(),
        reasoning_effort: provider
            .reasoning_effort
            .map(|value| value.as_str().to_owned()),
        temperature: provider.temperature,
        max_tokens: provider.max_tokens,
        request_timeout_ms: provider.request_timeout_ms,
        retry_max_attempts: provider.retry_max_attempts,
        header_names,
        preferred_models: provider.preferred_models.clone(),
    }
}

fn runtime_snapshot_provider_credentials_resolved(provider: &mvp::config::ProviderConfig) -> bool {
    provider_credential_policy::provider_has_locally_available_credentials(provider)
}

fn collect_runtime_snapshot_skills_state(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> (
    RuntimeSnapshotSkillsState,
    mvp::tools::runtime_config::ToolRuntimeConfig,
) {
    let empty_inventory = json!({
        "skills": [],
        "shadowed_skills": [],
    });

    let (effective_policy, override_active) =
        match runtime_snapshot_effective_skills_policy(tool_runtime) {
            Ok(policy_state) => policy_state,
            Err(error) => {
                return (
                    RuntimeSnapshotSkillsState {
                        policy: tool_runtime.skills.clone(),
                        override_active: false,
                        inventory_status: RuntimeSnapshotInventoryStatus::Error,
                        inventory_error: Some(error.clone()),
                        inventory: json!({
                            "skills": [],
                            "shadowed_skills": [],
                            "error": error,
                        }),
                        resolved_skill_count: 0,
                        shadowed_skill_count: 0,
                    },
                    tool_runtime.clone(),
                );
            }
        };

    let mut effective_tool_runtime = tool_runtime.clone();
    effective_tool_runtime.skills = effective_policy.clone();

    if !effective_policy.enabled {
        return (
            RuntimeSnapshotSkillsState {
                policy: effective_policy,
                override_active,
                inventory_status: RuntimeSnapshotInventoryStatus::Disabled,
                inventory_error: None,
                inventory: empty_inventory,
                resolved_skill_count: 0,
                shadowed_skill_count: 0,
            },
            effective_tool_runtime,
        );
    }

    match mvp::tools::skills_list_with_config(&effective_tool_runtime) {
        Ok(outcome) => (
            RuntimeSnapshotSkillsState {
                policy: effective_policy,
                override_active,
                inventory_status: RuntimeSnapshotInventoryStatus::Ok,
                inventory_error: None,
                resolved_skill_count: json_array_len(outcome.payload.get("skills")),
                shadowed_skill_count: json_array_len(outcome.payload.get("shadowed_skills")),
                inventory: outcome.payload,
            },
            effective_tool_runtime,
        ),
        Err(error) => (
            RuntimeSnapshotSkillsState {
                policy: effective_policy,
                override_active,
                inventory_status: RuntimeSnapshotInventoryStatus::Error,
                inventory_error: Some(error.clone()),
                inventory: json!({
                    "skills": [],
                    "shadowed_skills": [],
                    "error": error,
                }),
                resolved_skill_count: 0,
                shadowed_skill_count: 0,
            },
            effective_tool_runtime,
        ),
    }
}

pub(crate) fn collect_runtime_snapshot_runtime_plugins_state(
    config: &mvp::config::LoongConfig,
) -> RuntimeSnapshotRuntimePluginsState {
    let readiness_evaluation = config
        .runtime_plugins
        .readiness_evaluation_label()
        .to_owned();
    let roots = config
        .runtime_plugins
        .resolved_roots()
        .into_iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>();
    let supported_bridges = config
        .runtime_plugins
        .resolved_supported_bridges()
        .unwrap_or_default()
        .into_iter()
        .map(|bridge_kind| bridge_kind.as_str().to_owned())
        .collect::<Vec<_>>();
    let supported_adapter_families = config
        .runtime_plugins
        .normalized_supported_adapter_families();

    if !config.runtime_plugins.enabled {
        return RuntimeSnapshotRuntimePluginsState {
            enabled: false,
            roots,
            supported_bridges,
            supported_adapter_families,
            inventory_status: RuntimeSnapshotInventoryStatus::Disabled,
            inventory_error: None,
            readiness_evaluation,
            scanned_root_count: 0,
            scanned_file_count: 0,
            discovered_plugin_count: 0,
            translated_plugin_count: 0,
            ready_plugin_count: 0,
            setup_incomplete_plugin_count: 0,
            blocked_plugin_count: 0,
            plugins: Vec::new(),
        };
    }

    let resolved_roots = config.runtime_plugins.resolved_roots();
    if resolved_roots.is_empty() {
        return RuntimeSnapshotRuntimePluginsState {
            enabled: true,
            roots,
            supported_bridges,
            supported_adapter_families,
            inventory_status: RuntimeSnapshotInventoryStatus::Error,
            inventory_error: Some(
                "runtime_plugins.enabled=true but no runtime plugin roots are configured"
                    .to_owned(),
            ),
            readiness_evaluation,
            scanned_root_count: 0,
            scanned_file_count: 0,
            discovered_plugin_count: 0,
            translated_plugin_count: 0,
            ready_plugin_count: 0,
            setup_incomplete_plugin_count: 0,
            blocked_plugin_count: 0,
            plugins: Vec::new(),
        };
    }

    let scanner = PluginScanner::new();
    let mut combined = kernel::PluginScanReport::default();
    for root in &resolved_roots {
        let report = match scanner.scan_path(root) {
            Ok(report) => report,
            Err(error) => {
                return RuntimeSnapshotRuntimePluginsState {
                    enabled: true,
                    roots,
                    supported_bridges,
                    supported_adapter_families,
                    inventory_status: RuntimeSnapshotInventoryStatus::Error,
                    inventory_error: Some(format!(
                        "runtime plugin scan failed for {}: {error}",
                        root.display()
                    )),
                    readiness_evaluation,
                    scanned_root_count: 0,
                    scanned_file_count: 0,
                    discovered_plugin_count: 0,
                    translated_plugin_count: 0,
                    ready_plugin_count: 0,
                    setup_incomplete_plugin_count: 0,
                    blocked_plugin_count: 0,
                    plugins: Vec::new(),
                };
            }
        };
        merge_plugin_scan_report(&mut combined, report);
    }

    let bridge_matrix = match config.runtime_plugins.resolved_bridge_support_matrix() {
        Ok(matrix) => matrix,
        Err(error) => {
            return RuntimeSnapshotRuntimePluginsState {
                enabled: true,
                roots,
                supported_bridges,
                supported_adapter_families,
                inventory_status: RuntimeSnapshotInventoryStatus::Error,
                inventory_error: Some(error),
                readiness_evaluation,
                scanned_root_count: resolved_roots.len(),
                scanned_file_count: combined.scanned_files,
                discovered_plugin_count: combined.matched_plugins,
                translated_plugin_count: 0,
                ready_plugin_count: 0,
                setup_incomplete_plugin_count: 0,
                blocked_plugin_count: 0,
                plugins: Vec::new(),
            };
        }
    };

    let translator = PluginTranslator::new();
    let translation = translator.translate_scan_report(&combined);
    let readiness_context = runtime_plugin_setup_readiness_context(config);
    let activation = translator.plan_activation(&translation, &bridge_matrix, &readiness_context);
    let inventory_entries = activation.inventory_entries(&translation);
    let inventory_by_key = inventory_entries
        .into_iter()
        .map(|entry| ((entry.source_path.clone(), entry.plugin_id.clone()), entry))
        .collect::<BTreeMap<_, _>>();

    let plugins = translation
        .entries
        .iter()
        .map(|entry| {
            let entry_key = (entry.source_path.clone(), entry.plugin_id.clone());
            let inventory_entry = inventory_by_key.get(&entry_key);
            let setup_mode = entry
                .setup
                .as_ref()
                .map(|setup| setup.mode.as_str().to_owned());
            let setup_surface = entry.setup.as_ref().and_then(|setup| setup.surface.clone());
            let setup_requirements = evaluate_plugin_setup_requirements(
                entry
                    .setup
                    .as_ref()
                    .map(|setup| setup.required_env_vars.as_slice())
                    .unwrap_or(&[]),
                entry
                    .setup
                    .as_ref()
                    .map(|setup| setup.required_config_keys.as_slice())
                    .unwrap_or(&[]),
                &readiness_context,
            );
            let activation_status = inventory_entry.and_then(|item| item.activation_status);
            let slot_claims = entry
                .slot_claims
                .iter()
                .map(kernel::PluginSlotClaim::canonical_label)
                .collect::<Vec<_>>();
            let conflicting_slot_claims = if matches!(
                activation_status,
                Some(PluginActivationStatus::BlockedSlotClaimConflict)
            ) {
                slot_claims.clone()
            } else {
                Vec::new()
            };
            let status = activation_status
                .map(runtime_plugin_activation_status)
                .unwrap_or("unknown")
                .to_owned();
            let reason = inventory_entry
                .and_then(|item| item.activation_reason.clone())
                .unwrap_or_else(|| "-".to_owned());
            let missing_required_env_vars = if matches!(
                activation_status,
                Some(PluginActivationStatus::SetupIncomplete)
            ) {
                setup_requirements.missing_required_env_vars
            } else {
                Vec::new()
            };
            let missing_required_config_keys = if matches!(
                activation_status,
                Some(PluginActivationStatus::SetupIncomplete)
            ) {
                setup_requirements.missing_required_config_keys
            } else {
                Vec::new()
            };

            RuntimeSnapshotRuntimePluginState {
                plugin_id: entry.plugin_id.clone(),
                provider_id: entry.provider_id.clone(),
                connector_name: entry.connector_name.clone(),
                source_path: entry.source_path.clone(),
                source_kind: entry.source_kind.as_str().to_owned(),
                package_root: entry.package_root.clone(),
                package_manifest_path: entry.package_manifest_path.clone(),
                bridge_kind: entry.runtime.bridge_kind.as_str().to_owned(),
                adapter_family: entry.runtime.adapter_family.clone(),
                setup_mode,
                setup_surface,
                slot_claims,
                conflicting_slot_claims,
                status,
                reason,
                missing_required_env_vars,
                missing_required_config_keys,
            }
        })
        .collect::<Vec<_>>();

    RuntimeSnapshotRuntimePluginsState {
        enabled: true,
        roots,
        supported_bridges,
        supported_adapter_families,
        inventory_status: RuntimeSnapshotInventoryStatus::Ok,
        inventory_error: None,
        readiness_evaluation,
        scanned_root_count: resolved_roots.len(),
        scanned_file_count: combined.scanned_files,
        discovered_plugin_count: combined.matched_plugins,
        translated_plugin_count: translation.translated_plugins,
        ready_plugin_count: activation.ready_plugins,
        setup_incomplete_plugin_count: activation.setup_incomplete_plugins,
        blocked_plugin_count: activation.blocked_plugins,
        plugins,
    }
}

fn merge_plugin_scan_report(
    combined: &mut kernel::PluginScanReport,
    report: kernel::PluginScanReport,
) {
    let kernel::PluginScanReport {
        scanned_files,
        matched_plugins,
        descriptors,
        diagnostic_findings,
    } = report;

    combined.scanned_files += scanned_files;
    combined.matched_plugins += matched_plugins;
    combined.descriptors.extend(descriptors);
    combined.diagnostic_findings.extend(diagnostic_findings);
}

fn runtime_plugin_setup_readiness_context(
    config: &mvp::config::LoongConfig,
) -> PluginSetupReadinessContext {
    let verified_env_vars = std::env::vars_os()
        .filter_map(|(key, value)| {
            let value_string = value.to_string_lossy();
            let trimmed_value = value_string.trim();
            if trimmed_value.is_empty() {
                return None;
            }

            Some(key.to_string_lossy().to_string())
        })
        .collect();
    let mut verified_config_keys = BTreeSet::new();
    if let Ok(value) = serde_json::to_value(config) {
        collect_config_paths(&value, None, &mut verified_config_keys);
    }

    PluginSetupReadinessContext {
        verified_env_vars,
        verified_config_keys,
    }
}

fn collect_config_paths(value: &Value, prefix: Option<&str>, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next_prefix = match prefix {
                    Some(prefix) => format!("{prefix}.{key}"),
                    None => key.clone(),
                };

                match child {
                    Value::Null => {}
                    Value::Object(_)
                    | Value::Array(_)
                    | Value::Bool(_)
                    | Value::Number(_)
                    | Value::String(_) => {
                        out.insert(next_prefix.clone());
                        collect_config_paths(child, Some(next_prefix.as_str()), out);
                    }
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_config_paths(child, prefix, out);
            }
        }
        Value::Null => {}
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            if let Some(prefix) = prefix {
                out.insert(prefix.to_owned());
            }
        }
    }
}

fn runtime_snapshot_effective_skills_policy(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Result<(mvp::tools::runtime_config::SkillsRuntimePolicy, bool), String> {
    let outcome = mvp::tools::skills_policy_get_with_config(tool_runtime)
        .map_err(|error| format!("resolve effective skills policy failed: {error}"))?;

    let policy = runtime_snapshot_skills_policy_from_payload(&outcome.payload)?;
    let override_active = outcome
        .payload
        .get("override_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((policy, override_active))
}

fn runtime_snapshot_skills_policy_from_payload(
    payload: &Value,
) -> Result<mvp::tools::runtime_config::SkillsRuntimePolicy, String> {
    let policy = payload
        .get("policy")
        .and_then(Value::as_object)
        .ok_or_else(|| "runtime snapshot skills policy payload missing `policy`".to_owned())?;

    Ok(mvp::tools::runtime_config::SkillsRuntimePolicy {
        enabled: policy
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "runtime snapshot skills policy missing `enabled`".to_owned())?,
        require_download_approval: policy
            .get("require_download_approval")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot skills policy missing `require_download_approval`".to_owned()
            })?,
        allowed_domains: json_string_array_to_set(
            policy.get("allowed_domains"),
            "runtime snapshot skills policy.allowed_domains",
        )?,
        blocked_domains: json_string_array_to_set(
            policy.get("blocked_domains"),
            "runtime snapshot skills policy.blocked_domains",
        )?,
        install_root: policy
            .get("install_root")
            .and_then(Value::as_str)
            .map(Path::new)
            .map(Path::to_path_buf),
        auto_expose_installed: policy
            .get("auto_expose_installed")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot skills policy missing `auto_expose_installed`".to_owned()
            })?,
    })
}

fn runtime_snapshot_tool_digest(
    visible_tool_names: &[String],
    capability_snapshot: &str,
) -> CliResult<String> {
    let serialized = serde_json::to_vec(&json!({
        "visible_tool_names": visible_tool_names,
        "capability_snapshot": capability_snapshot,
    }))
    .map_err(|error| format!("serialize runtime snapshot tool digest input failed: {error}"))?;
    Ok(hex::encode(Sha256::digest(serialized)))
}

fn json_array_len(value: Option<&Value>) -> usize {
    value.and_then(Value::as_array).map_or(0, Vec::len)
}

fn runtime_plugin_activation_status(status: PluginActivationStatus) -> &'static str {
    status.as_str()
}

fn json_string_array_to_set(
    value: Option<&Value>,
    context: &str,
) -> Result<BTreeSet<String>, String> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context} must be an array"))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{context} must contain only strings"))
        })
        .collect()
}

fn build_runtime_snapshot_restore_spec(
    config: &mvp::config::LoongConfig,
    skills: &RuntimeSnapshotSkillsState,
) -> RuntimeSnapshotRestoreSpec {
    let mut warnings = Vec::new();
    let mut profiles = runtime_snapshot_restore_provider_profiles(config);
    for (profile_id, profile) in &mut profiles {
        normalize_runtime_snapshot_restore_provider_profile(profile_id, profile, &mut warnings);
    }

    RuntimeSnapshotRestoreSpec {
        provider: RuntimeSnapshotRestoreProviderSpec {
            active_provider: config.active_provider_id().map(str::to_owned),
            last_provider: config.last_provider_id().map(str::to_owned),
            profiles,
        },
        conversation: config.conversation.clone(),
        memory: config.memory.clone(),
        acp: config.acp.clone(),
        tools: config.tools.clone(),
        skills: config.skills.clone(),
        runtime_plugins: config.runtime_plugins.clone(),
        managed_skills: build_runtime_snapshot_restore_managed_skills_spec(skills, &mut warnings),
        warnings,
    }
}

fn runtime_snapshot_restore_provider_profiles(
    config: &mvp::config::LoongConfig,
) -> BTreeMap<String, mvp::config::ProviderProfileConfig> {
    if !config.providers.is_empty() {
        return config.providers.clone();
    }

    let profile_id = config
        .active_provider_id()
        .unwrap_or(config.provider.kind.profile().id)
        .to_owned();
    BTreeMap::from([(
        profile_id,
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: config.provider.clone(),
        },
    )])
}

fn normalize_runtime_snapshot_restore_provider_profile(
    profile_id: &str,
    profile: &mut mvp::config::ProviderProfileConfig,
    warnings: &mut Vec<String>,
) {
    runtime_snapshot_migrate_provider_env_reference(
        &mut profile.provider.api_key,
        &mut profile.provider.api_key_env,
    );
    runtime_snapshot_migrate_provider_env_reference(
        &mut profile.provider.oauth_access_token,
        &mut profile.provider.oauth_access_token_env,
    );

    if runtime_snapshot_redact_provider_secret_field(
        profile.provider.api_key.as_mut(),
        profile_id,
        "api_key",
        warnings,
    ) {
        profile.provider.api_key = None;
    }
    if runtime_snapshot_redact_provider_secret_field(
        profile.provider.oauth_access_token.as_mut(),
        profile_id,
        "oauth_access_token",
        warnings,
    ) {
        profile.provider.oauth_access_token = None;
    }

    let header_keys_to_remove = profile
        .provider
        .headers
        .iter()
        .filter(|(header_name, header_value)| {
            !runtime_snapshot_provider_header_is_safe_to_persist(
                profile.provider.kind,
                header_name,
                header_value,
            )
        })
        .map(|(header_name, _)| header_name.clone())
        .collect::<Vec<_>>();
    for header_name in header_keys_to_remove {
        profile.provider.headers.remove(&header_name);
        warnings.push(format!(
            "restore spec redacted inline provider header `{header_name}` for profile `{profile_id}`"
        ));
    }
}

fn runtime_snapshot_redact_provider_secret_field(
    raw: Option<&mut SecretRef>,
    profile_id: &str,
    field_name: &str,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    if raw.inline_literal_value().is_none() {
        return false;
    }
    warnings.push(format!(
        "restore spec redacted inline provider credential `{field_name}` for profile `{profile_id}`"
    ));
    true
}

fn runtime_snapshot_provider_header_is_safe_to_persist(
    provider_kind: mvp::config::ProviderKind,
    header_name: &str,
    header_value: &str,
) -> bool {
    if header_value.trim().is_empty() || runtime_snapshot_is_env_reference_literal(header_value) {
        return true;
    }

    let normalized = header_name.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "accept"
            | "accept-charset"
            | "accept-encoding"
            | "accept-language"
            | "anthropic-version"
            | "cache-control"
            | "content-language"
            | "content-type"
            | "pragma"
            | "user-agent"
            | "anthropic-beta"
            | "openai-beta"
    ) || provider_kind
        .default_headers()
        .iter()
        .any(|(default_name, _)| default_name.eq_ignore_ascii_case(&normalized))
}

fn runtime_snapshot_migrate_provider_env_reference(
    inline_secret: &mut Option<SecretRef>,
    env_name: &mut Option<String>,
) {
    let explicit_env_name = inline_secret
        .as_ref()
        .and_then(SecretRef::explicit_env_name);
    if let Some(explicit_env_name) = explicit_env_name {
        *inline_secret = Some(SecretRef::Env {
            env: explicit_env_name,
        });
        *env_name = None;
        return;
    }

    if inline_secret.as_ref().is_some_and(SecretRef::is_configured) {
        *env_name = None;
        return;
    }

    let configured_env_name = env_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(configured_env_name) = configured_env_name {
        *inline_secret = Some(SecretRef::Env {
            env: configured_env_name,
        });
    }
    *env_name = None;
}

fn runtime_snapshot_is_env_reference_literal(raw: &str) -> bool {
    runtime_snapshot_parse_env_reference(raw).is_some()
}

fn runtime_snapshot_parse_env_reference(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(inner) = trimmed
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    if let Some(inner) = trimmed.strip_prefix('$') {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    if let Some(inner) = trimmed.strip_prefix("env:") {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    if let Some(inner) = trimmed
        .strip_prefix('%')
        .and_then(|value| value.strip_suffix('%'))
    {
        return runtime_snapshot_is_valid_env_name(inner).then_some(inner);
    }

    None
}

fn runtime_snapshot_is_valid_env_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn build_runtime_snapshot_restore_managed_skills_spec(
    skills: &RuntimeSnapshotSkillsState,
    warnings: &mut Vec<String>,
) -> RuntimeSnapshotRestoreManagedSkillsSpec {
    match skills.inventory_status {
        RuntimeSnapshotInventoryStatus::Disabled => {
            warnings.push(
                "restore spec could not enumerate managed skills because runtime inventory is disabled"
                    .to_owned(),
            );
            return RuntimeSnapshotRestoreManagedSkillsSpec::default();
        }
        RuntimeSnapshotInventoryStatus::Error => {
            warnings.push(
                "restore spec could not enumerate managed skills because runtime inventory collection failed"
                    .to_owned(),
            );
            return RuntimeSnapshotRestoreManagedSkillsSpec::default();
        }
        RuntimeSnapshotInventoryStatus::Ok => {}
    }

    let Some(skills) = skills.inventory.get("skills").and_then(Value::as_array) else {
        return RuntimeSnapshotRestoreManagedSkillsSpec::default();
    };

    let mut managed_skills = skills
        .iter()
        .filter(|skill| skill.get("scope").and_then(Value::as_str) == Some("managed"))
        .filter_map(|skill| {
            let skill_id = skill.get("skill_id").and_then(Value::as_str)?;
            let display_name = skill
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let summary = skill
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let source_kind = skill.get("source_kind").and_then(Value::as_str)?;
            let source_path = skill.get("source_path").and_then(Value::as_str)?;
            let sha256 = skill.get("sha256").and_then(Value::as_str)?;
            Some(RuntimeSnapshotRestoreManagedSkillSpec {
                skill_id: skill_id.to_owned(),
                display_name: display_name.to_owned(),
                summary: summary.to_owned(),
                source_kind: source_kind.to_owned(),
                source_path: source_path.to_owned(),
                sha256: sha256.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    managed_skills.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    RuntimeSnapshotRestoreManagedSkillsSpec {
        skills: managed_skills,
    }
}

#[cfg(test)]
#[path = "lib_runtime_snapshot_restore_spec_tests.rs"]
mod runtime_snapshot_restore_spec_tests;

fn runtime_snapshot_artifact_metadata_now(
    label: Option<&str>,
    experiment_id: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> CliResult<RuntimeSnapshotArtifactMetadata> {
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format runtime snapshot artifact timestamp failed: {error}"))?;
    Ok(RuntimeSnapshotArtifactMetadata {
        created_at,
        label: runtime_snapshot_optional_arg(label),
        experiment_id: runtime_snapshot_optional_arg(experiment_id),
        parent_snapshot_id: runtime_snapshot_optional_arg(parent_snapshot_id),
    })
}

fn runtime_snapshot_optional_arg(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn persist_json_artifact(
    output_path: &str,
    payload: &Value,
    artifact_label: &str,
) -> CliResult<()> {
    let output_path = PathBuf::from(output_path);
    let parent_path = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&parent_path).map_err(|error| {
        format!(
            "create {artifact_label} directory {} failed: {error}",
            parent_path.display()
        )
    })?;
    let encoded = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("serialize {artifact_label} failed: {error}"))?;
    let file_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let process_id = process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("build {artifact_label} temp path failed: {error}"))?
        .as_nanos();
    let temp_file_name = format!(".{file_name}.{process_id}.{timestamp}.tmp");
    let temp_path = parent_path.join(temp_file_name);

    let open_result = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path);
    let mut temp_file = open_result.map_err(|error| {
        format!(
            "create {artifact_label} temp file {} failed: {error}",
            temp_path.display()
        )
    })?;
    temp_file.write_all(encoded.as_bytes()).map_err(|error| {
        format!(
            "write {artifact_label} temp file {} failed: {error}",
            temp_path.display()
        )
    })?;
    temp_file.sync_all().map_err(|error| {
        format!(
            "sync {artifact_label} temp file {} failed: {error}",
            temp_path.display()
        )
    })?;
    drop(temp_file);

    let rename_result = fs::rename(&temp_path, &output_path);
    if let Err(error) = rename_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "replace {artifact_label} {} failed: {error}",
            output_path.display()
        ));
    }
    Ok(())
}

pub fn build_runtime_snapshot_artifact_json_payload(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
) -> CliResult<Value> {
    let base_payload = cli_json::build_runtime_snapshot_cli_json_payload(snapshot)?;
    let lineage = runtime_snapshot_artifact_lineage(snapshot, metadata)?;
    let document = RuntimeSnapshotArtifactDocument {
        config: snapshot.config.clone(),
        schema: RuntimeSnapshotArtifactSchema {
            version: RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: "runtime_snapshot".to_owned(),
            purpose: "experiment_reproducibility".to_owned(),
        },
        lineage,
        provider: base_payload.get("provider").cloned().unwrap_or(Value::Null),
        context_engine: base_payload
            .get("context_engine")
            .cloned()
            .unwrap_or(Value::Null),
        memory_system: base_payload
            .get("memory_system")
            .cloned()
            .unwrap_or(Value::Null),
        acp: base_payload.get("acp").cloned().unwrap_or(Value::Null),
        channels: base_payload.get("channels").cloned().unwrap_or(Value::Null),
        tool_runtime: base_payload
            .get("tool_runtime")
            .cloned()
            .unwrap_or(Value::Null),
        tools: base_payload.get("tools").cloned().unwrap_or(Value::Null),
        runtime_plugins: base_payload
            .get("runtime_plugins")
            .cloned()
            .unwrap_or(Value::Null),
        skills: base_payload.get("skills").cloned().unwrap_or(Value::Null),
        restore_spec: snapshot.restore_spec.clone(),
    };
    serde_json::to_value(document)
        .map_err(|error| format!("serialize runtime snapshot artifact payload failed: {error}"))
}

fn runtime_snapshot_artifact_lineage(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
) -> CliResult<RuntimeSnapshotArtifactLineage> {
    let serialized = serde_json::to_vec(&json!({
        "config": snapshot.config,
        "created_at": metadata.created_at,
        "label": metadata.label,
        "experiment_id": metadata.experiment_id,
        "parent_snapshot_id": metadata.parent_snapshot_id,
        "capability_snapshot_sha256": snapshot.capability_snapshot_sha256,
        "active_provider": snapshot.provider.active_profile_id,
    }))
    .map_err(|error| format!("serialize runtime snapshot lineage input failed: {error}"))?;
    Ok(RuntimeSnapshotArtifactLineage {
        snapshot_id: hex::encode(Sha256::digest(serialized)),
        created_at: metadata.created_at.clone(),
        label: metadata.label.clone(),
        experiment_id: metadata.experiment_id.clone(),
        parent_snapshot_id: metadata.parent_snapshot_id.clone(),
    })
}

fn render_runtime_snapshot_artifact_text(
    snapshot: &RuntimeSnapshotCliState,
    artifact_payload: &Value,
) -> String {
    let lineage = artifact_payload
        .get("lineage")
        .cloned()
        .unwrap_or(Value::Null);
    let schema_version = artifact_payload
        .get("schema")
        .and_then(|schema| schema.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION));

    [
        format!("schema.version={schema_version}"),
        format!("snapshot_id={}", json_string_field(&lineage, "snapshot_id")),
        format!("created_at={}", json_string_field(&lineage, "created_at")),
        format!("label={}", json_string_field(&lineage, "label")),
        format!(
            "experiment_id={}",
            json_string_field(&lineage, "experiment_id")
        ),
        format!(
            "parent_snapshot_id={}",
            json_string_field(&lineage, "parent_snapshot_id")
        ),
        format!("restore_warnings={}", snapshot.restore_spec.warnings.len()),
        render_runtime_snapshot_text(snapshot),
    ]
    .join("\n")
}
