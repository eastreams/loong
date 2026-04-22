use serde_json::Value;

use super::*;

pub(super) async fn abilities_personalization(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<AbilitiesPersonalizationPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let payload = build_personalization_payload(&snapshot.config);

    Ok(Json(ApiEnvelope {
        ok: true,
        data: payload,
    }))
}

pub(super) async fn abilities_personalization_save(
    State(state): State<Arc<WebApiState>>,
    Json(request): Json<AbilitiesPersonalizationWriteRequest>,
) -> Result<Json<ApiEnvelope<AbilitiesPersonalizationPayload>>, WebApiError> {
    let config_path = resolve_web_config_path(state.as_ref());
    let mut config = if config_path.is_file() {
        let (_, loaded) = mvp::config::load(state.config_path.as_deref()).map_err(|error| {
            WebApiError::bad_request(format!("local config could not be loaded: {error}"))
        })?;
        loaded
    } else {
        mvp::config::LoongConfig::default()
    };

    let existing_personalization = config.memory.trimmed_personalization();
    let default_personalization = mvp::config::PersonalizationConfig::default();
    let prompt_state =
        parse_personalization_prompt_state(request.prompt_state.as_deref().unwrap_or("pending"))?;
    let updated_at_epoch_seconds = u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).ok();

    let personalization = mvp::config::PersonalizationConfig {
        preferred_name: normalize_optional_text(request.preferred_name.as_deref()),
        response_density: parse_response_density(request.response_density.as_deref())?,
        initiative_level: parse_initiative_level(request.initiative_level.as_deref())?,
        standing_boundaries: normalize_optional_text(request.standing_boundaries.as_deref()),
        timezone: normalize_optional_text(request.timezone.as_deref()),
        locale: normalize_optional_text(request.locale.as_deref()),
        prompt_state,
        schema_version: existing_personalization
            .as_ref()
            .map(|value| value.schema_version)
            .unwrap_or(default_personalization.schema_version),
        updated_at_epoch_seconds,
    }
    .normalized();

    config.memory.personalization = personalization;

    let path_string = config_path.display().to_string();
    mvp::config::write(Some(path_string.as_str()), &config, true).map_err(WebApiError::internal)?;

    let payload = build_personalization_payload(&config);
    let state_label = payload.prompt_state;
    record_debug_operation(
        &state,
        "abilities_personalization",
        format!(
            "{} personalization updated",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
        ),
        vec![
            format!(
                "preferred_name={}",
                payload.preferred_name.as_deref().unwrap_or("empty")
            ),
            format!(
                "response_density={}",
                payload.response_density.unwrap_or("unset")
            ),
            format!(
                "initiative_level={}",
                payload.initiative_level.unwrap_or("unset")
            ),
            format!("prompt_state={state_label}"),
        ],
    );

    Ok(Json(ApiEnvelope {
        ok: true,
        data: payload,
    }))
}

pub(super) async fn abilities_channels(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<AbilitiesChannelsPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let runtime_snapshot = collect_runtime_snapshot(snapshot.resolved_path.as_path())?;
    let inventory = &runtime_snapshot.channels.inventory;
    let enabled_service_channel_ids = &runtime_snapshot.channels.enabled_service_channel_ids;

    let surfaces = inventory
        .channel_surfaces
        .iter()
        .map(|surface| {
            let channel_id = surface.surface.catalog.id.to_owned();
            let service_enabled = enabled_service_channel_ids.contains(&channel_id);
            let ready_send_account_count = surface
                .surface
                .configured_accounts
                .iter()
                .filter(|account| {
                    channel_account_operation_is_ready(
                        account,
                        mvp::channel::CHANNEL_OPERATION_SEND_ID,
                    )
                })
                .count();
            let ready_serve_account_count = surface
                .surface
                .configured_accounts
                .iter()
                .filter(|account| {
                    channel_account_operation_is_ready(
                        account,
                        mvp::channel::CHANNEL_OPERATION_SERVE_ID,
                    )
                })
                .count();

            AbilitiesChannelSurfacePayload {
                id: channel_id,
                label: surface.surface.catalog.label.to_owned(),
                source: surface.surface.catalog.implementation_status.as_str(),
                configured_account_count: surface.surface.configured_accounts.len(),
                enabled_account_count: surface
                    .surface
                    .configured_accounts
                    .iter()
                    .filter(|account| account.enabled)
                    .count(),
                misconfigured_account_count: surface
                    .surface
                    .configured_accounts
                    .iter()
                    .filter(|account| channel_account_is_misconfigured(account))
                    .count(),
                ready_send_account_count,
                ready_serve_account_count,
                default_configured_account_id: surface
                    .surface
                    .default_configured_account_id
                    .clone(),
                service_enabled,
                service_ready: service_enabled && ready_serve_account_count > 0,
            }
        })
        .collect::<Vec<_>>();

    Ok(Json(ApiEnvelope {
        ok: true,
        data: AbilitiesChannelsPayload {
            catalog_channel_count: inventory.channel_catalog.len(),
            configured_channel_count: inventory
                .channel_surfaces
                .iter()
                .filter(|surface| !surface.surface.configured_accounts.is_empty())
                .count(),
            configured_account_count: inventory.channels.len(),
            enabled_account_count: inventory
                .channels
                .iter()
                .filter(|account| account.enabled)
                .count(),
            misconfigured_account_count: inventory
                .channels
                .iter()
                .filter(|account| channel_account_is_misconfigured(account))
                .count(),
            runtime_backed_channel_count: inventory
                .channel_catalog
                .iter()
                .filter(|channel| {
                    channel.implementation_status
                        == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
                })
                .count(),
            enabled_service_channel_count: enabled_service_channel_ids.len(),
            ready_service_channel_count: surfaces
                .iter()
                .filter(|surface| surface.service_ready)
                .count(),
            surfaces,
        },
    }))
}

pub(super) async fn abilities_skills(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<AbilitiesSkillsPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    let runtime_snapshot = collect_runtime_snapshot(snapshot.resolved_path.as_path())?;

    let browser_companion = json_object_field(&runtime_snapshot.tool_runtime, "browser_companion");
    let external_skills = json_object_field(&runtime_snapshot.external_skills, "policy");
    let visible_runtime_catalog = runtime_snapshot
        .tools
        .visible_tool_names
        .iter()
        .map(|tool_name| build_visible_tool_payload(tool_name))
        .collect::<Vec<_>>();
    let hidden_tool_surfaces = runtime_snapshot
        .tools
        .hidden_tool_surfaces
        .iter()
        .map(|surface| AbilitiesHiddenToolSurfacePayload {
            surface_id: surface.surface_id.clone(),
            tool_count: surface.tool_count,
            visible_tool_names: surface.visible_tool_names.clone(),
            usage_guidance: surface.usage_guidance.clone(),
        })
        .collect::<Vec<_>>();

    Ok(Json(ApiEnvelope {
        ok: true,
        data: AbilitiesSkillsPayload {
            visible_runtime_tool_count: runtime_snapshot.tools.visible_tool_count,
            visible_runtime_direct_tool_count: runtime_snapshot.tools.visible_direct_tool_names.len(),
            hidden_tool_count: runtime_snapshot.tools.hidden_tool_count,
            visible_runtime_tools: runtime_snapshot.tools.visible_tool_names.clone(),
            visible_runtime_catalog,
            hidden_tool_surfaces,
            approval_mode: approval_mode_label(snapshot.config.tools.approval.mode).to_owned(),
            autonomy_profile: snapshot.config.tools.autonomy_profile.as_str().to_owned(),
            consent_default_mode: snapshot.config.tools.consent.default_mode.as_str().to_owned(),
            sessions_allow_mutation: snapshot.config.tools.sessions.allow_mutation,
            browser_companion: AbilitiesBrowserCompanionPayload {
                enabled: json_bool_field(browser_companion, "enabled"),
                ready: json_bool_field(browser_companion, "ready"),
                command_configured: json_string_option_field(browser_companion, "command")
                    .is_some(),
                expected_version: json_string_option_field(browser_companion, "expected_version"),
                execution_tier: json_string_field(browser_companion, "execution_tier")
                    .unwrap_or("unknown")
                    .to_owned(),
                timeout_seconds: json_u64_field(browser_companion, "timeout_seconds").unwrap_or(0),
            },
            external_skills: AbilitiesExternalSkillsPayload {
                enabled: json_bool_field(external_skills, "enabled"),
                override_active: json_bool_field(
                    &runtime_snapshot.external_skills,
                    "override_active",
                ),
                inventory_status: json_string_field(
                    &runtime_snapshot.external_skills,
                    "inventory_status",
                )
                .unwrap_or("unknown")
                .to_owned(),
                inventory_error: json_string_option_field(
                    &runtime_snapshot.external_skills,
                    "inventory_error",
                ),
                require_download_approval: json_bool_field(
                    external_skills,
                    "require_download_approval",
                ),
                auto_expose_installed: json_bool_field(external_skills, "auto_expose_installed"),
                install_root: json_string_option_field(external_skills, "install_root"),
                allowed_domain_count: json_array_len(external_skills, "allowed_domains"),
                blocked_domain_count: json_array_len(external_skills, "blocked_domains"),
                resolved_skill_count: json_usize_field(
                    &runtime_snapshot.external_skills,
                    "resolved_skill_count",
                )
                .unwrap_or(0),
                shadowed_skill_count: json_usize_field(
                    &runtime_snapshot.external_skills,
                    "shadowed_skill_count",
                )
                .unwrap_or(0),
            },
        },
    }))
}

fn build_visible_tool_payload(tool_name: &str) -> AbilitiesVisibleToolPayload {
    match mvp::tools::tool_catalog().resolve(tool_name) {
        Some(entry) => AbilitiesVisibleToolPayload {
            visible_name: tool_name.to_owned(),
            canonical_name: entry.name.to_owned(),
            display_name: humanize_tool_name(tool_name),
            summary: entry.description.to_owned(),
            surface_id: entry.surface_id().map(str::to_owned),
            exposure: format!("{:?}", entry.exposure).to_ascii_lowercase(),
            execution_kind: tool_execution_kind_label(entry.execution_kind).to_owned(),
            capability_action_class: entry.capability_action_class().as_str().to_owned(),
            usage_guidance: entry.usage_guidance().map(str::to_owned),
        },
        None => AbilitiesVisibleToolPayload {
            visible_name: tool_name.to_owned(),
            canonical_name: tool_name.to_owned(),
            display_name: humanize_tool_name(tool_name),
            summary: format!("Runtime capability surfaced as {tool_name}."),
            surface_id: None,
            exposure: "unknown".to_owned(),
            execution_kind: "unknown".to_owned(),
            capability_action_class: "unknown".to_owned(),
            usage_guidance: None,
        },
    }
}

fn tool_execution_kind_label(
    execution_kind: mvp::tools::ToolExecutionKind,
) -> &'static str {
    match execution_kind {
        mvp::tools::ToolExecutionKind::Core => "core",
        mvp::tools::ToolExecutionKind::App => "app",
    }
}

fn humanize_tool_name(raw: &str) -> String {
    raw.split(['.', '_'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_runtime_snapshot(
    resolved_path: &FsPath,
) -> Result<crate::gateway::read_models::GatewayRuntimeSnapshotReadModel, WebApiError> {
    let path_string = resolved_path.display().to_string();
    let snapshot = crate::collect_runtime_snapshot_cli_state(Some(path_string.as_str()))
        .map_err(WebApiError::internal)?;
    Ok(crate::gateway::read_models::build_runtime_snapshot_read_model(&snapshot))
}

fn build_personalization_payload(
    config: &mvp::config::LoongConfig,
) -> AbilitiesPersonalizationPayload {
    match config.memory.trimmed_personalization() {
        Some(personalization) => AbilitiesPersonalizationPayload {
            configured: true,
            has_operator_preferences: personalization.has_operator_preferences(),
            suppressed: personalization.suppresses_suggestions(),
            prompt_state: match personalization.prompt_state {
                mvp::config::PersonalizationPromptState::Pending => "pending",
                mvp::config::PersonalizationPromptState::Deferred => "deferred",
                mvp::config::PersonalizationPromptState::Suppressed => "suppressed",
                mvp::config::PersonalizationPromptState::Configured => "configured",
            },
            updated_at: personalization
                .updated_at_epoch_seconds
                .map(|value| format_timestamp(value as i64)),
            preferred_name: personalization.preferred_name,
            response_density: personalization
                .response_density
                .map(mvp::config::ResponseDensity::as_str),
            initiative_level: personalization
                .initiative_level
                .map(mvp::config::InitiativeLevel::as_str),
            standing_boundaries: personalization.standing_boundaries,
            locale: personalization.locale,
            timezone: personalization.timezone,
        },
        None => AbilitiesPersonalizationPayload {
            configured: false,
            has_operator_preferences: false,
            suppressed: false,
            prompt_state: "pending",
            updated_at: None,
            preferred_name: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            locale: None,
            timezone: None,
        },
    }
}

fn normalize_optional_text(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_response_density(
    raw: Option<&str>,
) -> Result<Option<mvp::config::ResponseDensity>, WebApiError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some("concise") => Ok(Some(mvp::config::ResponseDensity::Concise)),
        Some("balanced") => Ok(Some(mvp::config::ResponseDensity::Balanced)),
        Some("thorough") => Ok(Some(mvp::config::ResponseDensity::Thorough)),
        Some(other) => Err(WebApiError::bad_request(format!(
            "unknown response density `{other}`"
        ))),
        None => Ok(None),
    }
}

fn parse_initiative_level(
    raw: Option<&str>,
) -> Result<Option<mvp::config::InitiativeLevel>, WebApiError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some("ask_before_acting") => Ok(Some(mvp::config::InitiativeLevel::AskBeforeActing)),
        Some("balanced") => Ok(Some(mvp::config::InitiativeLevel::Balanced)),
        Some("high_initiative") => Ok(Some(mvp::config::InitiativeLevel::HighInitiative)),
        Some(other) => Err(WebApiError::bad_request(format!(
            "unknown initiative level `{other}`"
        ))),
        None => Ok(None),
    }
}

fn parse_personalization_prompt_state(
    raw: &str,
) -> Result<mvp::config::PersonalizationPromptState, WebApiError> {
    match raw.trim() {
        "pending" => Ok(mvp::config::PersonalizationPromptState::Pending),
        "deferred" => Ok(mvp::config::PersonalizationPromptState::Deferred),
        "suppressed" => Ok(mvp::config::PersonalizationPromptState::Suppressed),
        "configured" => Ok(mvp::config::PersonalizationPromptState::Configured),
        other => Err(WebApiError::bad_request(format!(
            "unknown prompt state `{other}`"
        ))),
    }
}

fn json_object_field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn json_bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn json_string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn json_string_option_field(value: &Value, key: &str) -> Option<String> {
    json_string_field(value, key).map(ToOwned::to_owned)
}

fn json_u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn json_usize_field(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|raw| raw as usize)
}

fn json_array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn channel_account_is_misconfigured(account: &mvp::channel::ChannelStatusSnapshot) -> bool {
    account
        .operations
        .iter()
        .any(|operation| operation.health == mvp::channel::ChannelOperationHealth::Misconfigured)
}

fn channel_account_operation_is_ready(
    account: &mvp::channel::ChannelStatusSnapshot,
    operation_id: &str,
) -> bool {
    account
        .operation(operation_id)
        .is_some_and(|operation| operation.health == mvp::channel::ChannelOperationHealth::Ready)
}
