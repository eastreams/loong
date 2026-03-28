use super::*;

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotCliState {
    pub config: String,
    pub provider: RuntimeSnapshotProviderState,
    pub context_engine: mvp::conversation::ContextEngineRuntimeSnapshot,
    pub memory_system: mvp::memory::MemorySystemRuntimeSnapshot,
    pub acp: mvp::acp::AcpRuntimeSnapshot,
    pub enabled_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub channels: mvp::channel::ChannelInventory,
    pub tool_runtime: mvp::tools::runtime_config::ToolRuntimeConfig,
    pub visible_tool_names: Vec<String>,
    pub capability_snapshot: String,
    pub capability_snapshot_sha256: String,
    pub external_skills: RuntimeSnapshotExternalSkillsState,
    pub restore_spec: RuntimeSnapshotRestoreSpec,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderState {
    pub active_profile_id: String,
    pub active_label: String,
    pub last_provider_id: Option<String>,
    pub saved_profile_ids: Vec<String>,
    pub profiles: Vec<RuntimeSnapshotProviderProfileState>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderProfileState {
    pub profile_id: String,
    pub is_active: bool,
    pub default_for_kind: bool,
    pub kind: mvp::config::ProviderKind,
    pub model: String,
    pub wire_api: mvp::config::ProviderWireApi,
    pub base_url: String,
    pub endpoint: String,
    pub models_endpoint: String,
    pub protocol_family: &'static str,
    pub credential_resolved: bool,
    pub auth_env: Option<String>,
    pub reasoning_effort: Option<String>,
    pub temperature: f64,
    pub max_tokens: Option<u32>,
    pub request_timeout_ms: u64,
    pub retry_max_attempts: usize,
    pub header_names: Vec<String>,
    pub preferred_models: Vec<String>,
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
pub struct RuntimeSnapshotExternalSkillsState {
    pub policy: mvp::tools::runtime_config::ExternalSkillsRuntimePolicy,
    pub override_active: bool,
    pub inventory_status: RuntimeSnapshotInventoryStatus,
    pub inventory_error: Option<String>,
    pub inventory: Value,
    pub resolved_skill_count: usize,
    pub shadowed_skill_count: usize,
}

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
    pub external_skills: mvp::config::ExternalSkillsConfig,
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
    pub external_skills: Value,
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
        persist_runtime_snapshot_artifact(output_path, &artifact_payload)?;
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
    let config_display = resolved_path.display().to_string();
    let provider = collect_runtime_snapshot_provider_state(&config);
    let context_engine = mvp::conversation::collect_context_engine_runtime_snapshot(&config)?;
    let memory_system = mvp::memory::collect_memory_system_runtime_snapshot(&config)?;
    let acp = mvp::acp::collect_acp_runtime_snapshot(&config)?;
    let enabled_channel_ids = config.enabled_channel_ids();
    let enabled_service_channel_ids = config.enabled_service_channel_ids();
    let channels = mvp::channel::channel_inventory(&config);
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        &config,
        Some(resolved_path.as_path()),
    );
    let (external_skills, snapshot_tool_runtime) =
        collect_runtime_snapshot_external_skills_state(&tool_runtime);
    let tool_view = mvp::tools::runtime_tool_view_for_runtime_config(&snapshot_tool_runtime);
    let visible_tool_names = tool_view
        .tool_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let capability_snapshot = mvp::tools::capability_snapshot_with_config(&snapshot_tool_runtime);
    let capability_snapshot_sha256 =
        runtime_snapshot_tool_digest(&visible_tool_names, &capability_snapshot)?;
    let restore_spec = build_runtime_snapshot_restore_spec(&config, &external_skills);

    Ok(RuntimeSnapshotCliState {
        config: config_display,
        provider,
        context_engine,
        memory_system,
        acp,
        enabled_channel_ids,
        enabled_service_channel_ids,
        channels,
        tool_runtime: snapshot_tool_runtime,
        visible_tool_names,
        capability_snapshot,
        capability_snapshot_sha256,
        external_skills,
        restore_spec,
    })
}

fn collect_runtime_snapshot_provider_state(
    config: &mvp::config::LoongClawConfig,
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

    RuntimeSnapshotProviderState {
        active_profile_id,
        active_label: provider_presentation::active_provider_detail_label(config),
        last_provider_id: config.last_provider_id().map(str::to_owned),
        saved_profile_ids,
        profiles,
    }
}

fn build_runtime_snapshot_provider_profile_state(
    profile_id: &str,
    profile: &mvp::config::ProviderProfileConfig,
    is_active: bool,
) -> RuntimeSnapshotProviderProfileState {
    let provider = &profile.provider;
    let mut header_names = provider.headers.keys().cloned().collect::<Vec<_>>();
    header_names.sort();

    RuntimeSnapshotProviderProfileState {
        profile_id: profile_id.to_owned(),
        is_active,
        default_for_kind: profile.default_for_kind,
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
    if provider.resolved_auth_secret().is_some() {
        return true;
    }

    ["authorization", "x-api-key"].iter().any(|header_name| {
        provider
            .header_value(header_name)
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn collect_runtime_snapshot_external_skills_state(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> (
    RuntimeSnapshotExternalSkillsState,
    mvp::tools::runtime_config::ToolRuntimeConfig,
) {
    let empty_inventory = json!({
        "skills": [],
        "shadowed_skills": [],
    });

    let (effective_policy, override_active) =
        match runtime_snapshot_effective_external_skills_policy(tool_runtime) {
            Ok(policy_state) => policy_state,
            Err(error) => {
                return (
                    RuntimeSnapshotExternalSkillsState {
                        policy: tool_runtime.external_skills.clone(),
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
    effective_tool_runtime.external_skills = effective_policy.clone();

    if !effective_policy.enabled {
        return (
            RuntimeSnapshotExternalSkillsState {
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

    match mvp::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.list".to_owned(),
            payload: json!({}),
        },
        &effective_tool_runtime,
    ) {
        Ok(outcome) => (
            RuntimeSnapshotExternalSkillsState {
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
            RuntimeSnapshotExternalSkillsState {
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

fn runtime_snapshot_effective_external_skills_policy(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Result<
    (
        mvp::tools::runtime_config::ExternalSkillsRuntimePolicy,
        bool,
    ),
    String,
> {
    let outcome = mvp::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.policy".to_owned(),
            payload: json!({
                "action": "get",
            }),
        },
        tool_runtime,
    )
    .map_err(|error| format!("resolve effective external skills policy failed: {error}"))?;

    let policy = runtime_snapshot_external_skills_policy_from_payload(&outcome.payload)?;
    let override_active = outcome
        .payload
        .get("override_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((policy, override_active))
}

fn runtime_snapshot_external_skills_policy_from_payload(
    payload: &Value,
) -> Result<mvp::tools::runtime_config::ExternalSkillsRuntimePolicy, String> {
    let policy = payload
        .get("policy")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "runtime snapshot external skills policy payload missing `policy`".to_owned()
        })?;

    Ok(mvp::tools::runtime_config::ExternalSkillsRuntimePolicy {
        enabled: policy
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot external skills policy missing `enabled`".to_owned()
            })?,
        require_download_approval: policy
            .get("require_download_approval")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "runtime snapshot external skills policy missing `require_download_approval`"
                    .to_owned()
            })?,
        allowed_domains: json_string_array_to_set(
            policy.get("allowed_domains"),
            "runtime snapshot external skills policy.allowed_domains",
        )?,
        blocked_domains: json_string_array_to_set(
            policy.get("blocked_domains"),
            "runtime snapshot external skills policy.blocked_domains",
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
                "runtime snapshot external skills policy missing `auto_expose_installed`".to_owned()
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
    Ok(format!("{:x}", Sha256::digest(serialized)))
}

fn json_array_len(value: Option<&Value>) -> usize {
    value.and_then(Value::as_array).map_or(0, Vec::len)
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
    config: &mvp::config::LoongClawConfig,
    external_skills: &RuntimeSnapshotExternalSkillsState,
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
        external_skills: config.external_skills.clone(),
        managed_skills: build_runtime_snapshot_restore_managed_skills_spec(
            external_skills,
            &mut warnings,
        ),
        warnings,
    }
}

fn runtime_snapshot_restore_provider_profiles(
    config: &mvp::config::LoongClawConfig,
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
    external_skills: &RuntimeSnapshotExternalSkillsState,
    warnings: &mut Vec<String>,
) -> RuntimeSnapshotRestoreManagedSkillsSpec {
    match external_skills.inventory_status {
        RuntimeSnapshotInventoryStatus::Disabled => {
            warnings.push(
                "restore spec could not enumerate managed external skills because runtime inventory is disabled"
                    .to_owned(),
            );
            return RuntimeSnapshotRestoreManagedSkillsSpec::default();
        }
        RuntimeSnapshotInventoryStatus::Error => {
            warnings.push(
                "restore spec could not enumerate managed external skills because runtime inventory collection failed"
                    .to_owned(),
            );
            return RuntimeSnapshotRestoreManagedSkillsSpec::default();
        }
        RuntimeSnapshotInventoryStatus::Ok => {}
    }

    let Some(skills) = external_skills
        .inventory
        .get("skills")
        .and_then(Value::as_array)
    else {
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

fn persist_runtime_snapshot_artifact(output_path: &str, payload: &Value) -> CliResult<()> {
    let output_path = PathBuf::from(output_path);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime snapshot artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(payload)
        .map_err(|error| format!("serialize runtime snapshot artifact failed: {error}"))?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "write runtime snapshot artifact {} failed: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}

pub fn build_runtime_snapshot_artifact_json_payload(
    snapshot: &RuntimeSnapshotCliState,
    metadata: &RuntimeSnapshotArtifactMetadata,
) -> CliResult<Value> {
    let base_payload = build_runtime_snapshot_cli_json_payload(snapshot)?;
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
        external_skills: base_payload
            .get("external_skills")
            .cloned()
            .unwrap_or(Value::Null),
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
        snapshot_id: format!("{:x}", Sha256::digest(serialized)),
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

#[cfg(test)]
mod runtime_snapshot_restore_spec_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_snapshot_restore_managed_skills_keeps_entries_without_display_metadata() {
        let mut warnings = Vec::new();
        let spec = build_runtime_snapshot_restore_managed_skills_spec(
            &RuntimeSnapshotExternalSkillsState {
                policy: mvp::tools::runtime_config::ExternalSkillsRuntimePolicy::default(),
                override_active: false,
                inventory_status: RuntimeSnapshotInventoryStatus::Ok,
                inventory_error: None,
                inventory: json!({
                    "skills": [{
                        "scope": "managed",
                        "skill_id": "demo-skill",
                        "source_kind": "directory",
                        "source_path": "/tmp/demo-skill",
                        "sha256": "deadbeef"
                    }]
                }),
                resolved_skill_count: 1,
                shadowed_skill_count: 0,
            },
            &mut warnings,
        );

        assert!(warnings.is_empty());
        assert_eq!(spec.skills.len(), 1);
        assert_eq!(spec.skills[0].skill_id, "demo-skill");
        assert!(spec.skills[0].display_name.is_empty());
        assert!(spec.skills[0].summary.is_empty());
    }

    #[test]
    fn runtime_snapshot_provider_header_safety_uses_explicit_safe_names_only() {
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Anthropic,
            "anthropic-version",
            "2023-06-01",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "anthropic-version",
            "2023-06-01",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Anthropic,
            "anthropic-beta",
            "prompt-caching-2024-07-31",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Openai,
            "openai-beta",
            "assistants=v2",
        ));
        assert!(runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "x-goog-api-key",
            "${GOOGLE_API_KEY}",
        ));
        assert!(!runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "x-secret-beta",
            "literal-secret",
        ));
        assert!(!runtime_snapshot_provider_header_is_safe_to_persist(
            mvp::config::ProviderKind::Deepseek,
            "x-secret-version",
            "literal-secret",
        ));
    }

    #[test]
    fn runtime_snapshot_restore_normalization_moves_provider_env_name_fields_into_secret_refs() {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
                oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_restore_normalization_canonicalizes_matching_explicit_env_reference() {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key: Some(SecretRef::Inline("${INLINE_OPENAI_API_KEY}".to_owned())),
                api_key_env: Some(" INLINE_OPENAI_API_KEY ".to_owned()),
                oauth_access_token: Some(SecretRef::Inline(
                    "$INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
                )),
                oauth_access_token_env: Some("INLINE_OPENAI_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_restore_normalization_prefers_explicit_env_reference_over_legacy_env_field()
    {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key: Some(SecretRef::Inline("${INLINE_OPENAI_API_KEY}".to_owned())),
                api_key_env: Some("CONFIGURED_OPENAI_API_KEY".to_owned()),
                oauth_access_token: Some(SecretRef::Inline(
                    "$INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
                )),
                oauth_access_token_env: Some("CONFIGURED_OPENAI_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_restore_normalization_treats_blank_inline_secret_as_absent() {
        let mut warnings = Vec::new();
        let mut profile = mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "openai/gpt-5.1-codex".to_owned(),
                api_key: Some(SecretRef::Inline("   ".to_owned())),
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
                oauth_access_token: Some(SecretRef::Inline("   ".to_owned())),
                oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
                ..Default::default()
            },
        };

        normalize_runtime_snapshot_restore_provider_profile(
            "openai-main",
            &mut profile,
            &mut warnings,
        );

        assert_eq!(
            profile.provider.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(profile.provider.api_key_env, None);
        assert_eq!(
            profile.provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(profile.provider.oauth_access_token_env, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn runtime_snapshot_tool_runtime_json_reports_browser_execution_tiers() {
        let mut runtime = mvp::tools::runtime_config::ToolRuntimeConfig::default();
        runtime.browser_companion.enabled = true;
        runtime.browser_companion.ready = true;
        runtime.browser_companion.command = Some("browser-companion".to_owned());

        let json = runtime_snapshot_tool_runtime_json(&runtime);

        assert_eq!(json["browser"]["execution_tier"], json!("restricted"));
        assert_eq!(
            json["browser_companion"]["execution_tier"],
            json!("balanced")
        );
    }
}
