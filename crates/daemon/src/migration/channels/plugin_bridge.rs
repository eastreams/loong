use std::collections::BTreeMap;

use loongclaw_app as mvp;

use super::{ChannelCheckLevel, ChannelPreflightCheck, ChannelPreview, build_channel_preview};
use crate::migration::ImportSurfaceLevel;
use crate::plugin_bridge_account_summary::{
    plugin_bridge_account_summary, plugin_bridge_snapshot_blocker_reason,
};

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
    let level = preview_level(&surface, discovery);
    let detail = surface_detail(&surface, discovery);
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
        level: preflight_level(&surface, discovery),
        detail: surface_detail(&surface, discovery),
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

fn preview_level(
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> ImportSurfaceLevel {
    if surface_passes_preflight(surface, discovery) {
        return ImportSurfaceLevel::Ready;
    }

    ImportSurfaceLevel::Review
}

fn preflight_level(
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> ChannelCheckLevel {
    if surface_passes_preflight(surface, discovery) {
        return ChannelCheckLevel::Pass;
    }

    ChannelCheckLevel::Warn
}

fn surface_passes_preflight(
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> bool {
    let discovery_ready = discovery_passes_preflight(discovery);

    if !discovery_ready {
        return false;
    }

    let contract_blocker_detail = surface_contract_blocker_detail(surface);

    contract_blocker_detail.is_none()
}

fn discovery_passes_preflight(discovery: &mvp::channel::ChannelPluginBridgeDiscovery) -> bool {
    let is_matches_found =
        discovery.status == mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound;
    let selection_status = discovery.selection_status;
    let has_ready_selection = selection_status
        .map(|status| status.selects_ready_plugin())
        .unwrap_or(false);
    let has_ambiguity = discovery.ambiguity_status.is_some();

    is_matches_found && has_ready_selection && !has_ambiguity
}

fn surface_detail(
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let contract_blocker_detail = surface_contract_blocker_detail(surface);

    if let Some(contract_blocker_detail) = contract_blocker_detail {
        let has_multiple_enabled_accounts = surface_has_multiple_enabled_accounts(surface);
        let discovery_is_ready = discovery_passes_preflight(discovery);

        if has_multiple_enabled_accounts && discovery_is_ready {
            let ready_discovery_detail = discovery_detail(discovery);

            return format!("{ready_discovery_detail}; {contract_blocker_detail}");
        }

        return contract_blocker_detail;
    }

    discovery_detail(discovery)
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

fn surface_contract_blocker_detail(surface: &mvp::channel::ChannelSurface) -> Option<String> {
    let account_summary = plugin_bridge_account_summary(surface);

    if let Some(account_summary) = account_summary {
        return Some(account_summary);
    }

    for snapshot in enabled_configured_account_snapshots(surface) {
        let blocker_reason = plugin_bridge_snapshot_blocker_reason(snapshot);

        if let Some(blocker_reason) = blocker_reason {
            return Some(blocker_reason);
        }
    }

    None
}

fn enabled_configured_account_snapshots(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<&mvp::channel::ChannelStatusSnapshot> {
    surface
        .configured_accounts
        .iter()
        .filter(|snapshot| snapshot.enabled)
        .collect()
}

fn surface_has_multiple_enabled_accounts(surface: &mvp::channel::ChannelSurface) -> bool {
    let enabled_account_count = enabled_configured_account_snapshots(surface).len();

    enabled_account_count > 1
}

fn matches_found_detail(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
    managed_install_root: &str,
) -> String {
    if discovery_passes_preflight(discovery) {
        let selected_plugin_id = render_optional_plugin_id(discovery.selected_plugin_id.as_deref());

        return format!(
            "managed bridge ready under {managed_install_root}: selected plugin {selected_plugin_id}"
        );
    }

    let configured_selection_detail = configured_selection_detail(discovery, managed_install_root);

    if let Some(configured_selection_detail) = configured_selection_detail {
        return configured_selection_detail;
    }

    let ambiguity_status = discovery.ambiguity_status;

    if let Some(ambiguity_status) = ambiguity_status {
        let ambiguity_detail = ambiguity_detail(discovery, managed_install_root, ambiguity_status);

        return ambiguity_detail;
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

fn configured_selection_detail(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
    managed_install_root: &str,
) -> Option<String> {
    let selection_status = discovery.selection_status?;

    match selection_status {
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginNotFound => {
            let configured_plugin_id =
                render_optional_plugin_id(discovery.configured_plugin_id.as_deref());
            let compatible_plugin_ids =
                render_compatible_plugin_ids(&discovery.compatible_plugin_ids);

            Some(format!(
                "managed bridge discovery could not find configured managed_bridge_plugin_id={configured_plugin_id} under {managed_install_root}; compatible plugins: {compatible_plugin_ids}"
            ))
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIdDuplicated => {
            let configured_plugin_id =
                render_optional_plugin_id(discovery.configured_plugin_id.as_deref());
            let matching_plugin_labels = render_configured_selection_plugin_labels(discovery);

            Some(format!(
                "configured managed_bridge_plugin_id={configured_plugin_id} matches multiple managed bridge packages under {managed_install_root}: {matching_plugin_labels}"
            ))
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIncomplete => {
            let configured_plugin = configured_selection_plugin(discovery)?;

            Some(incomplete_plugin_detail(
                configured_plugin,
                managed_install_root,
            ))
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIncompatible => {
            let configured_plugin_id =
                render_optional_plugin_id(discovery.configured_plugin_id.as_deref());

            Some(format!(
                "configured managed bridge plugin {configured_plugin_id} does not satisfy the channel bridge contract under {managed_install_root}"
            ))
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::NotConfigured => None,
        mvp::channel::ChannelPluginBridgeSelectionStatus::SingleCompatibleMatch => None,
        mvp::channel::ChannelPluginBridgeSelectionStatus::SelectedCompatible => None,
    }
}

fn ambiguity_detail(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
    managed_install_root: &str,
    ambiguity_status: mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus,
) -> String {
    match ambiguity_status {
        mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins => {
            let compatible_plugin_ids =
                render_compatible_plugin_ids(&discovery.compatible_plugin_ids);

            format!(
                "managed bridge discovery found multiple compatible plugins under {managed_install_root}: {compatible_plugin_ids}"
            )
        }
        mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::DuplicateCompatiblePluginIds => {
            let compatible_plugin_labels = render_compatible_plugin_labels(discovery);

            format!(
                "managed bridge discovery found duplicate compatible plugin_id values under {managed_install_root}: {compatible_plugin_labels}"
            )
        }
    }
}

fn configured_selection_plugin(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> Option<&mvp::channel::ChannelDiscoveredPluginBridge> {
    let matching_plugins = configured_selection_plugins(discovery);
    matching_plugins.into_iter().next()
}

fn configured_selection_plugins(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> Vec<&mvp::channel::ChannelDiscoveredPluginBridge> {
    let configured_plugin_id = discovery.configured_plugin_id.as_deref();
    let Some(configured_plugin_id) = configured_plugin_id else {
        return Vec::new();
    };

    let mut matching_plugins = Vec::new();

    for plugin in &discovery.plugins {
        let matches_plugin_id = plugin.plugin_id == configured_plugin_id;

        if !matches_plugin_id {
            continue;
        }

        matching_plugins.push(plugin);
    }

    matching_plugins
}

fn render_compatible_plugin_ids(compatible_plugin_ids: &[String]) -> String {
    if compatible_plugin_ids.is_empty() {
        return "-".to_owned();
    }

    compatible_plugin_ids.join(", ")
}

fn render_optional_plugin_id(plugin_id: Option<&str>) -> String {
    let Some(plugin_id) = plugin_id else {
        return "-".to_owned();
    };

    plugin_id.to_owned()
}

fn render_configured_selection_plugin_labels(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let matching_plugins = configured_selection_plugins(discovery);
    let duplicate_plugin_id_counts = duplicate_plugin_id_counts(&discovery.plugins);

    render_plugin_labels(&matching_plugins, &duplicate_plugin_id_counts)
}

fn render_compatible_plugin_labels(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let duplicate_plugin_id_counts = duplicate_plugin_id_counts(&discovery.plugins);
    let mut compatible_plugins = Vec::new();

    for plugin in &discovery.plugins {
        let is_compatible =
            plugin.status == mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleReady;

        if !is_compatible {
            continue;
        }

        compatible_plugins.push(plugin);
    }

    render_plugin_labels(&compatible_plugins, &duplicate_plugin_id_counts)
}

fn render_plugin_labels(
    plugins: &[&mvp::channel::ChannelDiscoveredPluginBridge],
    duplicate_plugin_id_counts: &BTreeMap<String, usize>,
) -> String {
    if plugins.is_empty() {
        return "-".to_owned();
    }

    let mut plugin_labels = Vec::new();

    for plugin in plugins {
        let plugin_label = render_plugin_label(plugin, duplicate_plugin_id_counts);
        plugin_labels.push(plugin_label);
    }

    plugin_labels.join(", ")
}

fn render_plugin_label(
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
    duplicate_plugin_id_counts: &BTreeMap<String, usize>,
) -> String {
    let duplicate_count = duplicate_plugin_id_counts
        .get(&plugin.plugin_id)
        .copied()
        .unwrap_or(0);
    let has_duplicate_plugin_id = duplicate_count > 1;

    if !has_duplicate_plugin_id {
        return plugin.plugin_id.clone();
    }

    format!("{}@{}", plugin.plugin_id, plugin.package_root)
}

fn duplicate_plugin_id_counts(
    plugins: &[mvp::channel::ChannelDiscoveredPluginBridge],
) -> BTreeMap<String, usize> {
    let mut duplicate_plugin_id_counts = BTreeMap::new();

    for plugin in plugins {
        let count = duplicate_plugin_id_counts
            .entry(plugin.plugin_id.clone())
            .or_insert(0);
        *count += 1;
    }

    duplicate_plugin_id_counts
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
