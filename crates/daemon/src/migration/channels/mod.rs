use std::collections::BTreeSet;

use loong_app as mvp;
use serde_json::Value;

use super::types::{
    ChannelCandidate, ChannelCredentialState, ChannelImportReadiness, ImportSurface,
    ImportSurfaceLevel, PreviewStatus,
};

mod feishu;
mod line;
mod matrix;
mod outbound;
mod plugin_bridge;
mod telegram;
mod webhook;
mod wecom;
mod whatsapp;

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
pub(crate) const CONFIGURED_CHANNELS_ACTION_ID: &str = "configured_channels";
const CHANNEL_CATALOG_ACTION_LABEL: &str = "channels";

struct ChannelAdapter {
    id: &'static str,
    collect_preview:
        fn(&mvp::config::LoongConfig, &ChannelImportReadiness, &str) -> Option<ChannelPreview>,
    apply: fn(&mut mvp::config::LoongConfig, &mvp::config::LoongConfig) -> bool,
    readiness_state: fn(&mvp::config::LoongConfig) -> ChannelCredentialState,
    apply_import_readiness: fn(&mut mvp::config::LoongConfig, ChannelCredentialState),
    collect_preflight_checks: fn(&mvp::config::LoongConfig) -> Vec<ChannelPreflightCheck>,
    collect_doctor_checks: fn(&mvp::config::LoongConfig) -> Vec<ChannelDoctorCheck>,
    apply_default_env_bindings: fn(&mut mvp::config::LoongConfig) -> Vec<String>,
}

const REGISTRY: [ChannelAdapter; 7] = [
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
        id: line::ID,
        collect_preview: line::collect_preview,
        apply: line::apply,
        readiness_state: line::readiness_state,
        apply_import_readiness: line::apply_import_readiness,
        collect_preflight_checks: line::collect_preflight_checks,
        collect_doctor_checks: line::collect_doctor_checks,
        apply_default_env_bindings: line::apply_default_env_bindings,
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
    ChannelAdapter {
        id: whatsapp::ID,
        collect_preview: whatsapp::collect_preview,
        apply: whatsapp::apply,
        readiness_state: whatsapp::readiness_state,
        apply_import_readiness: whatsapp::apply_import_readiness,
        collect_preflight_checks: whatsapp::collect_preflight_checks,
        collect_doctor_checks: whatsapp::collect_doctor_checks,
        apply_default_env_bindings: whatsapp::apply_default_env_bindings,
    },
    ChannelAdapter {
        id: webhook::ID,
        collect_preview: webhook::collect_preview,
        apply: webhook::apply,
        readiness_state: webhook::readiness_state,
        apply_import_readiness: webhook::apply_import_readiness,
        collect_preflight_checks: webhook::collect_preflight_checks,
        collect_doctor_checks: webhook::collect_doctor_checks,
        apply_default_env_bindings: webhook::apply_default_env_bindings,
    },
];

pub fn registered_channel_ids() -> Vec<&'static str> {
    mvp::config::service_channel_descriptors()
        .into_iter()
        .filter_map(|descriptor| find_adapter(descriptor.id).map(|_| descriptor.id))
        .collect()
}

pub fn registered_enabled_channel_ids(config: &mvp::config::LoongConfig) -> Vec<&'static str> {
    enabled_channel_adapters(config)
        .into_iter()
        .map(|adapter| adapter.id)
        .collect()
}

pub fn collect_channel_previews(
    config: &mvp::config::LoongConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Vec<ChannelPreview> {
    let mut previews = ordered_channel_adapters()
        .into_iter()
        .filter_map(|adapter| (adapter.collect_preview)(config, readiness, source))
        .collect::<Vec<_>>();
    let plugin_bridge_previews = plugin_bridge::collect_previews(config, source);
    let outbound_previews = outbound::collect_previews(config, source);

    previews.extend(plugin_bridge_previews);
    previews.extend(outbound_previews);

    previews
}

pub fn resolve_import_readiness(config: &mvp::config::LoongConfig) -> ChannelImportReadiness {
    let mut readiness = ChannelImportReadiness::default();
    for adapter in ordered_channel_adapters() {
        readiness.set_state(adapter.id, (adapter.readiness_state)(config));
    }
    readiness
}

pub fn enabled_channels_have_blockers(
    config: &mvp::config::LoongConfig,
    readiness: &ChannelImportReadiness,
) -> bool {
    let service_channel_blockers = registered_enabled_channel_ids(config)
        .into_iter()
        .any(|channel_id| !readiness.is_ready(channel_id));

    if service_channel_blockers {
        return true;
    }

    let plugin_bridge_blockers = plugin_bridge::enabled_channels_have_blockers(config);

    if plugin_bridge_blockers {
        return true;
    }

    outbound::enabled_channels_have_blockers(config)
}

pub fn apply_detected_import_readiness(
    config: &mut mvp::config::LoongConfig,
    readiness: &ChannelImportReadiness,
) {
    for adapter in ordered_channel_adapters() {
        (adapter.apply_import_readiness)(config, readiness.state(adapter.id));
    }
}

pub fn apply_selected_channels(
    target: &mut mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
    channel_ids: &[&str],
) -> bool {
    let report = apply_selected_channels_with_report(target, source, channel_ids);

    report.changed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelApplyReport {
    pub changed: bool,
    pub conflicts: Vec<ChannelApplyConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelApplyConflict {
    pub channel_id: String,
    pub kind: ChannelApplyConflictKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelApplyConflictKind {
    PluginBridgeInstallRoot {
        existing_install_root: String,
        source_install_root: String,
    },
}

pub fn apply_selected_channels_with_report(
    target: &mut mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
    channel_ids: &[&str],
) -> ChannelApplyReport {
    let mut changed = false;
    let mut conflicts = Vec::new();

    for channel_id in channel_ids {
        if let Some(adapter) = find_adapter(channel_id) {
            changed |= (adapter.apply)(target, source);
            continue;
        }

        let outcome = apply_fallback_channel_section(target, source, channel_id);

        changed |= outcome.changed;

        if let Some(conflict) = outcome.conflict {
            conflicts.push(conflict);
        }
    }

    ChannelApplyReport { changed, conflicts }
}

pub fn summarize_channel_apply_conflict(conflict: &ChannelApplyConflict) -> String {
    match &conflict.kind {
        ChannelApplyConflictKind::PluginBridgeInstallRoot {
            existing_install_root,
            source_install_root,
        } => {
            format!(
                "managed bridge install_root conflict: existing root {existing_install_root} does not match detected root {source_install_root}"
            )
        }
    }
}

pub fn collect_channel_preflight_checks(
    config: &mvp::config::LoongConfig,
) -> Vec<ChannelPreflightCheck> {
    let mut checks = enabled_channel_adapters(config)
        .into_iter()
        .flat_map(|adapter| (adapter.collect_preflight_checks)(config))
        .collect::<Vec<_>>();
    let plugin_bridge_checks = plugin_bridge::collect_preflight_checks(config);
    let outbound_checks = outbound::collect_preflight_checks(config);

    checks.extend(plugin_bridge_checks);
    checks.extend(outbound_checks);

    checks
}

pub fn collect_channel_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<ChannelDoctorCheck> {
    let mut checks = enabled_channel_adapters(config)
        .into_iter()
        .flat_map(|adapter| (adapter.collect_doctor_checks)(config))
        .collect::<Vec<_>>();
    let plugin_bridge_checks = plugin_bridge::collect_doctor_checks(config);
    let outbound_checks = outbound::collect_doctor_checks(config);

    checks.extend(plugin_bridge_checks);
    checks.extend(outbound_checks);

    checks
}

pub fn apply_default_channel_env_bindings(config: &mut mvp::config::LoongConfig) -> Vec<String> {
    ordered_channel_adapters()
        .into_iter()
        .flat_map(|adapter| (adapter.apply_default_env_bindings)(config))
        .collect()
}

pub fn collect_channel_next_actions(
    config: &mvp::config::LoongConfig,
    config_path: &str,
) -> Vec<ChannelNextAction> {
    let mut actions = collect_configured_runtime_channel_next_actions(config, config_path);
    let inspection_actions =
        collect_configured_non_runtime_channel_next_actions(config, config_path);
    actions.extend(inspection_actions);

    if !actions.is_empty() {
        return actions;
    }

    vec![build_channel_catalog_next_action(config_path)]
}

fn collect_configured_runtime_channel_next_actions(
    config: &mvp::config::LoongConfig,
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

fn collect_enabled_service_channel_ids(config: &mvp::config::LoongConfig) -> BTreeSet<String> {
    config.enabled_service_channel_ids().into_iter().collect()
}

fn collect_configured_non_runtime_channel_next_actions(
    config: &mvp::config::LoongConfig,
    config_path: &str,
) -> Vec<ChannelNextAction> {
    let enabled_channel_ids = config
        .enabled_channel_ids()
        .into_iter()
        .filter(|channel_id| {
            channel_id
                != mvp::config::channel_descriptor("cli")
                    .map(|descriptor| descriptor.id)
                    .unwrap_or("cli")
        })
        .collect::<BTreeSet<_>>();
    if enabled_channel_ids.is_empty() {
        return Vec::new();
    }

    let inventory = mvp::channel::channel_inventory(config);
    let surfaces = inventory
        .channel_surfaces
        .into_iter()
        .filter(|surface| enabled_channel_ids.contains(surface.catalog.id))
        .filter(|surface| {
            surface.catalog.implementation_status
                != mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .collect::<Vec<_>>();

    if surfaces.is_empty() {
        return Vec::new();
    }

    vec![build_configured_channel_inspection_action(
        config_path,
        &surfaces,
    )]
}

fn build_channel_catalog_next_action(config_path: &str) -> ChannelNextAction {
    ChannelNextAction {
        id: CHANNEL_CATALOG_ACTION_ID,
        label: CHANNEL_CATALOG_ACTION_LABEL,
        command: crate::cli_handoff::format_subcommand_with_config("channels", config_path),
    }
}

fn build_configured_channel_inspection_action(
    config_path: &str,
    surfaces: &[mvp::channel::ChannelSurface],
) -> ChannelNextAction {
    let label = configured_channel_inspection_label(surfaces);

    ChannelNextAction {
        id: CONFIGURED_CHANNELS_ACTION_ID,
        label: Box::leak(label.into_boxed_str()),
        command: crate::cli_handoff::format_subcommand_with_config("channels", config_path),
    }
}

fn configured_channel_inspection_label(surfaces: &[mvp::channel::ChannelSurface]) -> String {
    let any_review = surfaces
        .iter()
        .any(configured_non_runtime_surface_needs_review);

    if surfaces.len() == 1 {
        let Some(surface) = surfaces.first() else {
            return "inspect configured channels".to_owned();
        };
        let runtime_kind = surface_runtime_kind(surface);

        if runtime_kind == Some(mvp::config::ChannelRuntimeKind::PluginBacked) {
            return if any_review {
                format!("review {} bridge", surface.catalog.label)
            } else {
                format!("inspect {} bridge", surface.catalog.label)
            };
        }

        return if any_review {
            format!("review {} setup", surface.catalog.label)
        } else {
            format!("inspect {}", surface.catalog.label)
        };
    }

    let all_plugin_backed = surfaces.iter().all(surface_is_plugin_backed);
    let all_outbound_only = surfaces.iter().all(surface_is_outbound_only);

    if all_plugin_backed {
        return if any_review {
            "review configured bridges".to_owned()
        } else {
            "inspect configured bridges".to_owned()
        };
    }

    if all_outbound_only {
        return if any_review {
            "review configured outbound channels".to_owned()
        } else {
            "inspect configured outbound channels".to_owned()
        };
    }

    if any_review {
        "review configured channels".to_owned()
    } else {
        "inspect configured channels".to_owned()
    }
}

fn configured_non_runtime_surface_needs_review(surface: &mvp::channel::ChannelSurface) -> bool {
    match surface_runtime_kind(surface) {
        Some(mvp::config::ChannelRuntimeKind::PluginBacked) => {
            configured_plugin_surface_needs_review(surface)
        }
        Some(mvp::config::ChannelRuntimeKind::OutboundOnly) => {
            configured_outbound_surface_needs_review(surface)
        }
        _ => false,
    }
}

fn configured_outbound_surface_needs_review(surface: &mvp::channel::ChannelSurface) -> bool {
    let enabled_accounts = surface
        .configured_accounts
        .iter()
        .filter(|snapshot| snapshot.enabled)
        .collect::<Vec<_>>();
    if enabled_accounts.is_empty() {
        return true;
    }

    enabled_accounts.iter().any(|snapshot| {
        !snapshot
            .operation(mvp::channel::CHANNEL_OPERATION_SEND_ID)
            .is_some_and(|operation| {
                operation.health == mvp::channel::ChannelOperationHealth::Ready
            })
    })
}

fn configured_plugin_surface_needs_review(surface: &mvp::channel::ChannelSurface) -> bool {
    let Some(discovery) = surface.plugin_bridge_discovery.as_ref() else {
        return true;
    };

    let discovery_ready = discovery.status
        == mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound
        && discovery
            .selection_status
            .map(|status| status.selects_ready_plugin())
            .unwrap_or(false)
        && discovery.ambiguity_status.is_none();
    if !discovery_ready {
        return true;
    }

    let enabled_accounts = surface
        .configured_accounts
        .iter()
        .filter(|snapshot| snapshot.enabled)
        .collect::<Vec<_>>();
    if enabled_accounts.is_empty() {
        return true;
    }

    enabled_accounts.iter().any(|snapshot| {
        snapshot.operations.iter().any(|operation| {
            operation.health == mvp::channel::ChannelOperationHealth::Misconfigured
                || (operation.health == mvp::channel::ChannelOperationHealth::Unsupported
                    && !snapshot
                        .notes
                        .iter()
                        .any(|note| note == "bridge_runtime_owner=external_plugin"))
        })
    })
}

fn enabled_channel_adapters(config: &mvp::config::LoongConfig) -> Vec<&'static ChannelAdapter> {
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

fn apply_fallback_channel_section(
    target: &mut mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
    channel_id: &str,
) -> FallbackChannelSectionOutcome {
    let install_root_conflict =
        resolve_plugin_bridge_install_root_conflict(target, source, channel_id);

    if let Some(conflict) = install_root_conflict {
        return FallbackChannelSectionOutcome {
            changed: false,
            conflict: Some(conflict),
        };
    }

    let default_config = mvp::config::LoongConfig::default();
    let mut target_value = match serde_json::to_value(&*target) {
        Ok(value) => value,
        Err(_) => return FallbackChannelSectionOutcome::unchanged(),
    };
    let source_value = match serde_json::to_value(source) {
        Ok(value) => value,
        Err(_) => return FallbackChannelSectionOutcome::unchanged(),
    };
    let default_value = match serde_json::to_value(default_config) {
        Ok(value) => value,
        Err(_) => return FallbackChannelSectionOutcome::unchanged(),
    };
    let Some(target_object) = target_value.as_object_mut() else {
        return FallbackChannelSectionOutcome::unchanged();
    };
    let Some(source_object) = source_value.as_object() else {
        return FallbackChannelSectionOutcome::unchanged();
    };
    let Some(default_object) = default_value.as_object() else {
        return FallbackChannelSectionOutcome::unchanged();
    };
    let Some(source_section) = source_object.get(channel_id) else {
        return FallbackChannelSectionOutcome::unchanged();
    };
    let null_value = Value::Null;
    let default_section = default_object.get(channel_id).unwrap_or(&null_value);
    let target_section = target_object
        .entry(channel_id.to_owned())
        .or_insert_with(|| default_section.clone());
    let section_changed =
        merge_channel_section_value(target_section, source_section, default_section);

    if section_changed {
        let restored_config = match serde_json::from_value(target_value) {
            Ok(config) => config,
            Err(_) => return FallbackChannelSectionOutcome::unchanged(),
        };

        *target = restored_config;
    }

    let install_root_changed = supplement_plugin_bridge_install_root(target, source);
    let changed = section_changed || install_root_changed;

    FallbackChannelSectionOutcome {
        changed,
        conflict: None,
    }
}

fn merge_channel_section_value(target: &mut Value, source: &Value, default: &Value) -> bool {
    let target_object = target.as_object_mut();
    let source_object = source.as_object();
    let default_object = default.as_object();

    if let (Some(target_object), Some(source_object), Some(default_object)) =
        (target_object, source_object, default_object)
    {
        let mut changed = false;

        for (key, source_value) in source_object {
            let null_value = Value::Null;
            let default_value = default_object.get(key).unwrap_or(&null_value);
            let target_value = target_object
                .entry(key.clone())
                .or_insert_with(|| default_value.clone());
            let merged = merge_channel_section_value(target_value, source_value, default_value);

            changed |= merged;
        }

        return changed;
    }

    let target_matches_default = *target == *default;
    let source_matches_default = *source == *default;

    if !target_matches_default || source_matches_default {
        return false;
    }

    *target = source.clone();

    true
}

fn supplement_plugin_bridge_install_root(
    target: &mut mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
) -> bool {
    let target_install_root = normalized_plugin_bridge_install_root(target);

    if target_install_root.is_some() {
        return false;
    }

    let source_install_root = normalized_plugin_bridge_install_root(source);
    let Some(source_install_root) = source_install_root else {
        return false;
    };

    target.external_skills.install_root = Some(source_install_root);

    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FallbackChannelSectionOutcome {
    changed: bool,
    conflict: Option<ChannelApplyConflict>,
}

impl FallbackChannelSectionOutcome {
    fn unchanged() -> Self {
        Self {
            changed: false,
            conflict: None,
        }
    }
}

fn resolve_plugin_bridge_install_root_conflict(
    target: &mvp::config::LoongConfig,
    source: &mvp::config::LoongConfig,
    channel_id: &str,
) -> Option<ChannelApplyConflict> {
    if !channel_uses_plugin_bridge_contract(channel_id) {
        return None;
    }

    let target_install_root = normalized_plugin_bridge_install_root(target)?;
    let source_install_root = normalized_plugin_bridge_install_root(source)?;

    if target_install_root == source_install_root {
        return None;
    }

    let conflict = ChannelApplyConflict {
        channel_id: channel_id.to_owned(),
        kind: ChannelApplyConflictKind::PluginBridgeInstallRoot {
            existing_install_root: target_install_root,
            source_install_root,
        },
    };

    Some(conflict)
}

fn channel_uses_plugin_bridge_contract(channel_id: &str) -> bool {
    let catalog_entry = mvp::channel::resolve_channel_catalog_entry(channel_id);
    let Some(catalog_entry) = catalog_entry else {
        return false;
    };

    catalog_entry.plugin_bridge_contract.is_some()
}

fn normalized_plugin_bridge_install_root(config: &mvp::config::LoongConfig) -> Option<String> {
    let install_root = config.external_skills.install_root.as_deref()?;
    let trimmed_install_root = install_root.trim();

    if trimmed_install_root.is_empty() {
        return None;
    }

    Some(trimmed_install_root.to_owned())
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
    let detail = format!("{} · {}", channel_runtime_kind_label(id), detail);

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

pub(crate) fn channel_runtime_kind(
    channel_id: &'static str,
) -> Option<mvp::config::ChannelRuntimeKind> {
    let descriptor = mvp::config::channel_descriptor(channel_id)?;

    Some(descriptor.runtime_kind)
}

pub(crate) fn surface_runtime_kind(
    surface: &mvp::channel::ChannelSurface,
) -> Option<mvp::config::ChannelRuntimeKind> {
    channel_runtime_kind(surface.catalog.id)
}

pub(crate) fn surface_is_plugin_backed(surface: &mvp::channel::ChannelSurface) -> bool {
    let runtime_kind = surface_runtime_kind(surface);

    runtime_kind == Some(mvp::config::ChannelRuntimeKind::PluginBacked)
}

pub(crate) fn surface_is_outbound_only(surface: &mvp::channel::ChannelSurface) -> bool {
    let runtime_kind = surface_runtime_kind(surface);

    runtime_kind == Some(mvp::config::ChannelRuntimeKind::OutboundOnly)
}

fn channel_runtime_kind_label(channel_id: &'static str) -> &'static str {
    match channel_runtime_kind(channel_id) {
        Some(mvp::config::ChannelRuntimeKind::RuntimeBacked) => "runtime-backed",
        Some(mvp::config::ChannelRuntimeKind::PluginBacked) => "plugin-backed",
        Some(mvp::config::ChannelRuntimeKind::OutboundOnly) => "outbound-only",
        Some(mvp::config::ChannelRuntimeKind::CatalogOnly) => "catalog-only",
        Some(mvp::config::ChannelRuntimeKind::Interactive) => "interactive",
        None => "channel",
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
