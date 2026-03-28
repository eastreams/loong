use std::collections::BTreeSet;

use loongclaw_app as mvp;

use super::types::{
    ChannelCandidate, ChannelCredentialState, ChannelImportReadiness, ImportSurface,
    ImportSurfaceLevel, PreviewStatus,
};

mod feishu;
mod matrix;
mod telegram;
mod wecom;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPreview {
    pub candidate: ChannelCandidate,
    pub surface: ImportSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelCheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPreflightCheck {
    pub name: &'static str,
    pub level: ChannelCheckLevel,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDoctorCheck {
    pub name: &'static str,
    pub level: ChannelCheckLevel,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelNextAction {
    pub id: &'static str,
    pub label: &'static str,
    pub command: String,
}

pub(crate) const CHANNEL_CATALOG_ACTION_ID: &str = "channel_catalog";
const CHANNEL_CATALOG_ACTION_LABEL: &str = "channels";

struct ChannelAdapter {
    id: &'static str,
    collect_preview:
        fn(&mvp::config::LoongClawConfig, &ChannelImportReadiness, &str) -> Option<ChannelPreview>,
    apply: fn(&mut mvp::config::LoongClawConfig, &mvp::config::LoongClawConfig) -> bool,
    readiness_state: fn(&mvp::config::LoongClawConfig) -> ChannelCredentialState,
    apply_import_readiness: fn(&mut mvp::config::LoongClawConfig, ChannelCredentialState),
    collect_preflight_checks: fn(&mvp::config::LoongClawConfig) -> Vec<ChannelPreflightCheck>,
    collect_doctor_checks: fn(&mvp::config::LoongClawConfig) -> Vec<ChannelDoctorCheck>,
    apply_default_env_bindings: fn(&mut mvp::config::LoongClawConfig) -> Vec<String>,
}

const REGISTRY: [ChannelAdapter; 4] = [
    ChannelAdapter {
        id: telegram::ID,
        collect_preview: telegram::collect_preview,
        apply: telegram::apply,
        readiness_state: telegram::readiness_state,
        apply_import_readiness: telegram::apply_import_readiness,
        collect_preflight_checks: telegram::collect_preflight_checks,
        collect_doctor_checks: telegram::collect_doctor_checks,
        apply_default_env_bindings: telegram::apply_default_env_bindings,
    },
    ChannelAdapter {
        id: feishu::ID,
        collect_preview: feishu::collect_preview,
        apply: feishu::apply,
        readiness_state: feishu::readiness_state,
        apply_import_readiness: feishu::apply_import_readiness,
        collect_preflight_checks: feishu::collect_preflight_checks,
        collect_doctor_checks: feishu::collect_doctor_checks,
        apply_default_env_bindings: feishu::apply_default_env_bindings,
    },
    ChannelAdapter {
        id: matrix::ID,
        collect_preview: matrix::collect_preview,
        apply: matrix::apply,
        readiness_state: matrix::readiness_state,
        apply_import_readiness: matrix::apply_import_readiness,
        collect_preflight_checks: matrix::collect_preflight_checks,
        collect_doctor_checks: matrix::collect_doctor_checks,
        apply_default_env_bindings: matrix::apply_default_env_bindings,
    },
    ChannelAdapter {
        id: wecom::ID,
        collect_preview: wecom::collect_preview,
        apply: wecom::apply,
        readiness_state: wecom::readiness_state,
        apply_import_readiness: wecom::apply_import_readiness,
        collect_preflight_checks: wecom::collect_preflight_checks,
        collect_doctor_checks: wecom::collect_doctor_checks,
        apply_default_env_bindings: wecom::apply_default_env_bindings,
    },
];

pub fn registered_channel_ids() -> Vec<&'static str> {
    mvp::config::service_channel_descriptors()
        .into_iter()
        .filter_map(|descriptor| find_adapter(descriptor.id).map(|_| descriptor.id))
        .collect()
}

pub fn registered_enabled_channel_ids(config: &mvp::config::LoongClawConfig) -> Vec<&'static str> {
    enabled_channel_adapters(config)
        .into_iter()
        .map(|adapter| adapter.id)
        .collect()
}

pub fn collect_channel_previews(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Vec<ChannelPreview> {
    ordered_channel_adapters()
        .into_iter()
        .filter_map(|adapter| (adapter.collect_preview)(config, readiness, source))
        .collect()
}

pub fn resolve_import_readiness(config: &mvp::config::LoongClawConfig) -> ChannelImportReadiness {
    let mut readiness = ChannelImportReadiness::default();
    for adapter in ordered_channel_adapters() {
        readiness.set_state(adapter.id, (adapter.readiness_state)(config));
    }
    readiness
}

pub fn enabled_channels_have_blockers(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
) -> bool {
    registered_enabled_channel_ids(config)
        .into_iter()
        .any(|channel_id| !readiness.is_ready(channel_id))
}

pub fn apply_detected_import_readiness(
    config: &mut mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
) {
    for adapter in ordered_channel_adapters() {
        (adapter.apply_import_readiness)(config, readiness.state(adapter.id));
    }
}

pub fn apply_selected_channels(
    target: &mut mvp::config::LoongClawConfig,
    source: &mvp::config::LoongClawConfig,
    channel_ids: &[&str],
) -> bool {
    let mut changed = false;
    for channel_id in channel_ids {
        if let Some(adapter) = find_adapter(channel_id) {
            changed |= (adapter.apply)(target, source);
        }
    }
    changed
}

pub fn collect_channel_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    enabled_channel_adapters(config)
        .into_iter()
        .flat_map(|adapter| (adapter.collect_preflight_checks)(config))
        .collect()
}

pub fn collect_channel_doctor_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelDoctorCheck> {
    enabled_channel_adapters(config)
        .into_iter()
        .flat_map(|adapter| (adapter.collect_doctor_checks)(config))
        .collect()
}

pub fn apply_default_channel_env_bindings(
    config: &mut mvp::config::LoongClawConfig,
) -> Vec<String> {
    ordered_channel_adapters()
        .into_iter()
        .flat_map(|adapter| (adapter.apply_default_env_bindings)(config))
        .collect()
}

pub fn collect_channel_next_actions(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
) -> Vec<ChannelNextAction> {
    let configured_actions = collect_configured_runtime_channel_next_actions(config, config_path);
    if !configured_actions.is_empty() {
        return configured_actions;
    }

    vec![build_channel_catalog_next_action(config_path)]
}

fn collect_configured_runtime_channel_next_actions(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
) -> Vec<ChannelNextAction> {
    let enabled_channel_ids = collect_enabled_service_channel_ids(config);
    if enabled_channel_ids.is_empty() {
        return Vec::new();
    }

    let inventory = mvp::channel::channel_inventory(config);
    inventory
        .channel_surfaces
        .into_iter()
        .filter(|surface| enabled_channel_ids.contains(surface.catalog.id))
        .filter_map(|surface| {
            let serve_operation = surface
                .catalog
                .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)?;
            if serve_operation.availability
                != mvp::channel::ChannelCatalogOperationAvailability::Implemented
            {
                return None;
            }

            Some(ChannelNextAction {
                id: surface.catalog.id,
                label: surface.catalog.label,
                command: crate::cli_handoff::format_subcommand_with_config(
                    serve_operation.command,
                    config_path,
                ),
            })
        })
        .collect()
}

fn collect_enabled_service_channel_ids(config: &mvp::config::LoongClawConfig) -> BTreeSet<String> {
    config.enabled_service_channel_ids().into_iter().collect()
}

fn build_channel_catalog_next_action(config_path: &str) -> ChannelNextAction {
    ChannelNextAction {
        id: CHANNEL_CATALOG_ACTION_ID,
        label: CHANNEL_CATALOG_ACTION_LABEL,
        command: crate::cli_handoff::format_subcommand_with_config("channels", config_path),
    }
}

fn enabled_channel_adapters(config: &mvp::config::LoongClawConfig) -> Vec<&'static ChannelAdapter> {
    let enabled_ids = config
        .enabled_service_channel_ids()
        .into_iter()
        .collect::<BTreeSet<_>>();
    registered_channel_ids()
        .into_iter()
        .filter(|channel_id| enabled_ids.contains(*channel_id))
        .filter_map(find_adapter)
        .collect()
}

fn ordered_channel_adapters() -> Vec<&'static ChannelAdapter> {
    registered_channel_ids()
        .into_iter()
        .filter_map(find_adapter)
        .collect()
}

fn find_adapter(channel_id: &str) -> Option<&'static ChannelAdapter> {
    REGISTRY.iter().find(|adapter| adapter.id == channel_id)
}

fn preview_status_from_surface_level(level: ImportSurfaceLevel) -> PreviewStatus {
    match level {
        ImportSurfaceLevel::Ready => PreviewStatus::Ready,
        ImportSurfaceLevel::Review => PreviewStatus::NeedsReview,
        ImportSurfaceLevel::Blocked => PreviewStatus::Unavailable,
    }
}

fn build_channel_preview(
    id: &'static str,
    label: &'static str,
    surface_name: &'static str,
    source: String,
    level: ImportSurfaceLevel,
    detail: String,
) -> ChannelPreview {
    ChannelPreview {
        candidate: ChannelCandidate {
            id,
            label,
            status: preview_status_from_surface_level(level),
            source,
            summary: detail.clone(),
        },
        surface: ImportSurface {
            name: surface_name,
            domain: super::types::SetupDomainKind::Channels,
            level,
            detail,
        },
    }
}

pub(super) fn ensure_default_env_binding(
    slot: &mut Option<String>,
    default_key: Option<&str>,
    label: &'static str,
    fixes: &mut Vec<String>,
) {
    if slot
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return;
    }
    let Some(default_key) = default_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    *slot = Some(default_key.to_owned());
    fixes.push(format!("{label}={default_key}"));
}
