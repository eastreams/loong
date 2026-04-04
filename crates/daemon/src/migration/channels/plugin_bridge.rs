use loongclaw_app as mvp;

use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ImportSurfaceLevel;

pub(super) fn collect_previews(
    config: &mvp::config::LoongClawConfig,
    source: &str,
) -> Vec<ChannelPreview> {
    let surfaces = configured_plugin_bridge_surfaces(config);

    surfaces
        .into_iter()
        .filter_map(|surface| build_preview(surface, source))
        .collect()
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<ChannelPreflightCheck> {
    let surfaces = configured_plugin_bridge_surfaces(config);

    surfaces
        .into_iter()
        .filter_map(build_preflight_check)
        .collect()
}

pub(super) fn enabled_channels_have_blockers(config: &mvp::config::LoongClawConfig) -> bool {
    let checks = collect_preflight_checks(config);

    checks
        .into_iter()
        .any(|check| check.level != ChannelCheckLevel::Pass)
}

fn configured_plugin_bridge_surfaces(
    config: &mvp::config::LoongClawConfig,
) -> Vec<mvp::channel::ChannelSurface> {
    let inventory = mvp::channel::channel_inventory(config);

    inventory
        .channel_surfaces
        .into_iter()
        .filter(surface_uses_plugin_bridge)
        .filter(surface_is_materially_configured)
        .collect()
}

fn surface_uses_plugin_bridge(surface: &mvp::channel::ChannelSurface) -> bool {
    surface.catalog.plugin_bridge_contract.is_some()
}

fn surface_is_materially_configured(surface: &mvp::channel::ChannelSurface) -> bool {
    surface
        .configured_accounts
        .iter()
        .any(snapshot_is_materially_configured)
}

fn snapshot_is_materially_configured(snapshot: &mvp::channel::ChannelStatusSnapshot) -> bool {
    if snapshot.enabled {
        return true;
    }

    snapshot.operations.iter().any(operation_is_not_disabled)
}

fn operation_is_not_disabled(operation: &mvp::channel::ChannelOperationStatus) -> bool {
    operation.health != mvp::channel::ChannelOperationHealth::Disabled
}

fn build_preview(surface: mvp::channel::ChannelSurface, source: &str) -> Option<ChannelPreview> {
    let discovery = surface.plugin_bridge_discovery.as_ref()?;
    let channel_id = surface.catalog.id;
    let channel_label = channel_label(channel_id);
    let surface_name = channel_surface_name(channel_id);
    let level = preview_level(discovery);
    let detail = discovery_detail(discovery);
    let preview = build_channel_preview(
        channel_id,
        channel_label,
        surface_name,
        source.to_owned(),
        level,
        detail,
    );

    Some(preview)
}

fn build_preflight_check(surface: mvp::channel::ChannelSurface) -> Option<ChannelPreflightCheck> {
    let discovery = surface.plugin_bridge_discovery.as_ref()?;
    let check = ChannelPreflightCheck {
        name: channel_surface_name(surface.catalog.id),
        level: preflight_level(discovery),
        detail: discovery_detail(discovery),
    };

    Some(check)
}

fn channel_label(channel_id: &'static str) -> &'static str {
    let descriptor = mvp::config::channel_descriptor(channel_id);

    match descriptor {
        Some(descriptor) => descriptor.label,
        None => channel_id,
    }
}

fn channel_surface_name(channel_id: &'static str) -> &'static str {
    let descriptor = mvp::config::channel_descriptor(channel_id);

    match descriptor {
        Some(descriptor) => descriptor.surface_label,
        None => channel_label(channel_id),
    }
}

fn preview_level(discovery: &mvp::channel::ChannelPluginBridgeDiscovery) -> ImportSurfaceLevel {
    if discovery_passes_preflight(discovery) {
        return ImportSurfaceLevel::Ready;
    }

    ImportSurfaceLevel::Review
}

fn preflight_level(discovery: &mvp::channel::ChannelPluginBridgeDiscovery) -> ChannelCheckLevel {
    if discovery_passes_preflight(discovery) {
        return ChannelCheckLevel::Pass;
    }

    ChannelCheckLevel::Warn
}

fn discovery_passes_preflight(discovery: &mvp::channel::ChannelPluginBridgeDiscovery) -> bool {
    let is_matches_found =
        discovery.status == mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    let has_single_compatible_plugin = discovery.compatible_plugins == 1;
    let has_ambiguity = discovery.ambiguity_status.is_some();

    is_matches_found && has_single_compatible_plugin && !has_ambiguity
}

fn discovery_detail(discovery: &mvp::channel::ChannelPluginBridgeDiscovery) -> String {
    let managed_install_root = discovery.managed_install_root.as_deref().unwrap_or("-");

    match discovery.status {
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::NotConfigured => {
            "managed bridge discovery is unavailable because external_skills.install_root is not configured".to_owned()
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::ScanFailed => {
            let scan_issue = discovery.scan_issue.as_deref().unwrap_or("unknown scan failure");

            format!("managed bridge discovery failed under {managed_install_root}: {scan_issue}")
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::NoMatches => {
            format!(
                "managed bridge discovery found no matching bridge plugins under {managed_install_root}"
            )
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound => {
            matches_found_detail(discovery, managed_install_root)
        }
    }
}

fn matches_found_detail(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
    managed_install_root: &str,
) -> String {
    if discovery_passes_preflight(discovery) {
        let compatible_plugin_ids = render_compatible_plugin_ids(&discovery.compatible_plugin_ids);

        return format!(
            "managed bridge ready under {managed_install_root}: compatible plugin {compatible_plugin_ids}"
        );
    }

    if discovery.ambiguity_status.is_some() {
        let compatible_plugin_ids = render_compatible_plugin_ids(&discovery.compatible_plugin_ids);

        return format!(
            "managed bridge discovery found multiple compatible plugins under {managed_install_root}: {compatible_plugin_ids}"
        );
    }

    let incomplete_plugin = discovery
        .plugins
        .iter()
        .find(|plugin| plugin_is_incomplete(plugin.status));

    if let Some(plugin) = incomplete_plugin {
        return incomplete_plugin_detail(plugin, managed_install_root);
    }

    format!(
        "managed bridge discovery found no compatible bridge plugins under {managed_install_root}"
    )
}

fn render_compatible_plugin_ids(compatible_plugin_ids: &[String]) -> String {
    if compatible_plugin_ids.is_empty() {
        return "-".to_owned();
    }

    compatible_plugin_ids.join(", ")
}

fn plugin_is_incomplete(status: mvp::channel::ChannelDiscoveredPluginBridgeStatus) -> bool {
    matches!(
        status,
        mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract
            | mvp::channel::ChannelDiscoveredPluginBridgeStatus::MissingSetupSurface
    )
}

fn incomplete_plugin_detail(
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
    managed_install_root: &str,
) -> String {
    let mut segments = Vec::new();

    segments.push(format!(
        "managed bridge setup incomplete under {managed_install_root}: plugin {}",
        plugin.plugin_id
    ));

    if !plugin.missing_fields.is_empty() {
        let missing_fields = plugin.missing_fields.join(", ");

        segments.push(format!("missing contract fields: {missing_fields}"));
    }

    if !plugin.issues.is_empty() {
        let issues = plugin.issues.join(", ");

        segments.push(format!("issues: {issues}"));
    }

    if !plugin.required_env_vars.is_empty() {
        let required_env_vars = plugin.required_env_vars.join(", ");

        segments.push(format!("required env vars: {required_env_vars}"));
    }

    if !plugin.required_config_keys.is_empty() {
        let required_config_keys = plugin.required_config_keys.join(", ");

        segments.push(format!("required config keys: {required_config_keys}"));
    }

    if !plugin.setup_docs_urls.is_empty() {
        let docs_urls = plugin.setup_docs_urls.join(", ");

        segments.push(format!("docs: {docs_urls}"));
    }

    if let Some(setup_remediation) = plugin.setup_remediation.as_deref() {
        segments.push(format!("remediation: {setup_remediation}"));
    }

    segments.join(" · ")
}
