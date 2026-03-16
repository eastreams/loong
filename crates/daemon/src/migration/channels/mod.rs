use std::collections::BTreeSet;

use loongclaw_app as mvp;

use super::types::{
    ChannelCandidate, ChannelCredentialState, ChannelImportReadiness, ImportSurface,
    ImportSurfaceLevel, PreviewStatus,
};

mod feishu;
mod telegram;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelPreview {
    pub(crate) candidate: ChannelCandidate,
    pub(crate) surface: ImportSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelCheckLevel {
    Pass,
    Warn,
    #[cfg(test)]
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelPreflightCheck {
    pub(crate) name: &'static str,
    pub(crate) level: ChannelCheckLevel,
    pub(crate) detail: String,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelDoctorCheck {
    pub(crate) name: &'static str,
    pub(crate) level: ChannelCheckLevel,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelNextAction {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) command: String,
}

struct ChannelAdapter {
    id: &'static str,
    collect_preview:
        fn(&mvp::config::LoongClawConfig, &ChannelImportReadiness, &str) -> Option<ChannelPreview>,
    apply: fn(&mut mvp::config::LoongClawConfig, &mvp::config::LoongClawConfig) -> bool,
    readiness_state: fn(&mvp::config::LoongClawConfig) -> ChannelCredentialState,
    apply_import_readiness: fn(&mut mvp::config::LoongClawConfig, ChannelCredentialState),
    collect_preflight_checks: fn(&mvp::config::LoongClawConfig) -> Vec<ChannelPreflightCheck>,
    #[cfg(test)]
    collect_doctor_checks: fn(&mvp::config::LoongClawConfig) -> Vec<ChannelDoctorCheck>,
    apply_default_env_bindings: fn(&mut mvp::config::LoongClawConfig) -> Vec<String>,
}

const REGISTRY: [ChannelAdapter; 2] = [
    ChannelAdapter {
        id: telegram::ID,
        collect_preview: telegram::collect_preview,
        apply: telegram::apply,
        readiness_state: telegram::readiness_state,
        apply_import_readiness: telegram::apply_import_readiness,
        collect_preflight_checks: telegram::collect_preflight_checks,
        #[cfg(test)]
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
        #[cfg(test)]
        collect_doctor_checks: feishu::collect_doctor_checks,
        apply_default_env_bindings: feishu::apply_default_env_bindings,
    },
];

pub(crate) fn registered_channel_ids() -> Vec<&'static str> {
    mvp::config::service_channel_descriptors()
        .into_iter()
        .filter_map(|descriptor| find_adapter(descriptor.id).map(|_| descriptor.id))
        .collect()
}

pub(crate) fn registered_enabled_channel_ids(
    config: &mvp::config::LoongClawConfig,
) -> Vec<&'static str> {
    enabled_channel_adapters(config)
        .into_iter()
        .map(|adapter| adapter.id)
        .collect()
}

pub(crate) fn collect_channel_previews(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Vec<ChannelPreview> {
    ordered_channel_adapters()
        .into_iter()
        .filter_map(|adapter| (adapter.collect_preview)(config, readiness, source))
        .collect()
}

pub(crate) fn resolve_import_readiness(
    config: &mvp::config::LoongClawConfig,
) -> ChannelImportReadiness {
    let mut readiness = ChannelImportReadiness::default();
    for adapter in ordered_channel_adapters() {
        readiness.set_state(adapter.id, (adapter.readiness_state)(config));
    }
    readiness
}

pub(crate) fn enabled_channels_have_blockers(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
) -> bool {
    registered_enabled_channel_ids(config)
        .into_iter()
        .any(|channel_id| !readiness.is_ready(channel_id))
}

pub(crate) fn apply_detected_import_readiness(
    config: &mut mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
) {
    for adapter in ordered_channel_adapters() {
        (adapter.apply_import_readiness)(config, readiness.state(adapter.id));
    }
}

pub(crate) fn apply_selected_channels(
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

pub(crate) fn collect_channel_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    enabled_channel_adapters(config)
        .into_iter()
        .flat_map(|adapter| (adapter.collect_preflight_checks)(config))
        .collect()
}

#[cfg(test)]
pub(crate) fn collect_channel_doctor_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelDoctorCheck> {
    enabled_channel_adapters(config)
        .into_iter()
        .flat_map(|adapter| (adapter.collect_doctor_checks)(config))
        .collect()
}

pub(crate) fn apply_default_channel_env_bindings(
    config: &mut mvp::config::LoongClawConfig,
) -> Vec<String> {
    ordered_channel_adapters()
        .into_iter()
        .flat_map(|adapter| (adapter.apply_default_env_bindings)(config))
        .collect()
}

pub(crate) fn collect_channel_next_actions(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
) -> Vec<ChannelNextAction> {
    enabled_channel_adapters(config)
        .into_iter()
        .filter_map(|adapter| {
            mvp::config::channel_descriptor(adapter.id).and_then(|descriptor| {
                descriptor
                    .serve_subcommand
                    .map(|subcommand| ChannelNextAction {
                        id: adapter.id,
                        label: descriptor.label,
                        command: format!(
                            "{} {} --config '{}'",
                            mvp::config::CLI_COMMAND_NAME,
                            subcommand,
                            config_path
                        ),
                    })
            })
        })
        .collect()
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
