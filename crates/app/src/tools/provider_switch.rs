use std::path::PathBuf;

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Map, Value, json};

use crate::config::{self, ProviderProfileConfig};

pub(super) fn execute_provider_switch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "provider.switch payload must be an object".to_owned())?;
    let config_path = resolve_provider_switch_config_path(Some(payload), config)?;
    let config_path_str = config_path.to_str().ok_or_else(|| {
        format!(
            "provider.switch config path {} is not valid UTF-8",
            config_path.display()
        )
    })?;
    let (_, mut loaded) = config::load(Some(config_path_str))?;
    let previous_active_provider = loaded.active_provider_id().map(str::to_owned);
    let selector = payload
        .get("selector")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let active_provider = if let Some(selector) = selector.as_deref() {
        loaded.switch_active_provider(selector)?;
        config::write(Some(config_path_str), &loaded, true)?;
        loaded.active_provider_id().unwrap_or(selector).to_owned()
    } else {
        loaded.active_provider_id().unwrap_or_default().to_owned()
    };
    let changed = previous_active_provider.as_deref() != Some(active_provider.as_str());
    let profiles = provider_profile_payloads(&loaded, active_provider.as_str());

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "changed": changed,
            "selector": selector,
            "config_path": config_path.display().to_string(),
            "previous_active_provider": previous_active_provider,
            "active_provider": active_provider,
            "last_provider": loaded.last_provider_id(),
            "profiles": profiles,
        }),
    })
}

pub(super) fn resolve_provider_switch_config_path(
    payload: Option<&Map<String, Value>>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    if let Some(raw) = payload
        .and_then(|payload| payload.get("config_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return super::file::resolve_safe_file_path_with_config(raw, config);
    }
    config
        .config_path
        .clone()
        .ok_or_else(|| {
            format!(
                "provider.switch requires a resolved runtime config path; start from `{} chat` / channel runtime or pass payload.config_path",
                config::active_cli_command_name()
            )
        })
}

fn provider_profile_payloads(config: &config::LoongConfig, active_provider: &str) -> Vec<Value> {
    config
        .providers
        .iter()
        .map(|(profile_id, profile)| {
            provider_profile_payload(config, profile_id, profile, active_provider)
        })
        .collect()
}

fn provider_profile_payload(
    config: &config::LoongConfig,
    profile_id: &str,
    profile: &ProviderProfileConfig,
    active_provider: &str,
) -> Value {
    let transport = profile.provider.transport_policy();
    json!({
        "profile_id": profile_id,
        "active": profile_id == active_provider,
        "accepted_selectors": config.accepted_provider_selectors(profile_id),
        "default_for_kind": profile.default_for_kind,
        "kind": profile.provider.kind.as_str(),
        "display_name": profile.provider.kind.display_name(),
        "model": profile.provider.model,
        "wire_api": profile.provider.wire_api.as_str(),
        "base_url": profile.provider.base_url,
        "endpoint": transport.request_endpoint,
        "models_endpoint": transport.models_endpoint,
        "transport_readiness": {
            "level": match transport.readiness.level {
                crate::config::ProviderTransportReadinessLevel::Ready => "ready",
                crate::config::ProviderTransportReadinessLevel::Review => "review",
                crate::config::ProviderTransportReadinessLevel::Unsupported => "unsupported",
            },
            "summary": transport.readiness.summary,
            "detail": transport.readiness.detail,
            "auto_fallback_to_chat_completions": transport.readiness.auto_fallback_to_chat_completions,
        }
    })
}
