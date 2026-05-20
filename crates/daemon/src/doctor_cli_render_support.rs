use super::*;

pub(super) fn render_managed_plugin_bridge_compatible_plugin_ids(
    compatible_plugin_ids: &[String],
) -> String {
    crate::render_line_safe_text_values(compatible_plugin_ids.iter().map(String::as_str), ",")
}

pub(super) fn render_managed_plugin_bridge_discovery_plugins(
    plugins: &[mvp::channel::ChannelDiscoveredPluginBridge],
) -> String {
    if plugins.is_empty() {
        return "-".to_owned();
    }

    let mut rendered_plugins = Vec::new();

    for plugin in plugins {
        let rendered_plugin = render_managed_plugin_bridge_discovery_plugin(plugin);
        rendered_plugins.push(rendered_plugin);
    }

    rendered_plugins.join("; ")
}

pub(super) fn render_managed_plugin_bridge_discovery_plugin(
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
) -> String {
    let mut segments = Vec::new();
    let plugin_id = crate::render_line_safe_text_value(&plugin.plugin_id);
    let bridge_kind = crate::render_line_safe_text_value(&plugin.bridge_kind);
    let adapter_family = crate::render_line_safe_text_value(&plugin.adapter_family);
    let source_path = crate::render_line_safe_text_value(&plugin.source_path);
    let package_root = crate::render_line_safe_text_value(&plugin.package_root);
    let package_manifest_path =
        crate::render_line_safe_optional_text_value(plugin.package_manifest_path.as_deref());

    segments.push(plugin_id);
    segments.push(format!("status={}", plugin.status.as_str()));
    segments.push(format!("bridge_kind={bridge_kind}"));
    segments.push(format!("adapter_family={adapter_family}"));

    if let Some(transport_family) = &plugin.transport_family {
        let rendered_transport_family = crate::render_line_safe_text_value(transport_family);
        segments.push(format!("transport_family={rendered_transport_family}"));
    }

    if let Some(target_contract) = &plugin.target_contract {
        let rendered_target_contract = crate::render_line_safe_text_value(target_contract);
        segments.push(format!("target_contract={rendered_target_contract}"));
    }

    if let Some(account_scope) = &plugin.account_scope {
        let rendered_account_scope = crate::render_line_safe_text_value(account_scope);
        segments.push(format!("account_scope={rendered_account_scope}"));
    }

    if let Some(runtime_contract) = &plugin.runtime_contract {
        let rendered_runtime_contract = crate::render_line_safe_text_value(runtime_contract);
        segments.push(format!("runtime_contract={rendered_runtime_contract}"));
    }

    if !plugin.runtime_operations.is_empty() {
        let rendered_runtime_operations = crate::render_line_safe_text_values(
            plugin.runtime_operations.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("runtime_operations={rendered_runtime_operations}"));
    }

    segments.push(format!("source_path={source_path}"));
    segments.push(format!("package_root={package_root}"));
    segments.push(format!("package_manifest_path={package_manifest_path}"));

    if !plugin.missing_fields.is_empty() {
        let missing_fields = crate::render_line_safe_text_values(
            plugin.missing_fields.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("missing_fields={missing_fields}"));
    }

    if !plugin.issues.is_empty() {
        let issues =
            crate::render_line_safe_text_values(plugin.issues.iter().map(String::as_str), "|");
        segments.push(format!("issues={issues}"));
    }

    if !plugin.required_env_vars.is_empty() {
        let required_env_vars = crate::render_line_safe_text_values(
            plugin.required_env_vars.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("required_env_vars={required_env_vars}"));
    }

    if !plugin.recommended_env_vars.is_empty() {
        let recommended_env_vars = crate::render_line_safe_text_values(
            plugin.recommended_env_vars.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("recommended_env_vars={recommended_env_vars}"));
    }

    if !plugin.required_config_keys.is_empty() {
        let required_config_keys = crate::render_line_safe_text_values(
            plugin.required_config_keys.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("required_config_keys={required_config_keys}"));
    }

    if let Some(default_env_var) = &plugin.default_env_var {
        let rendered_default_env_var = crate::render_line_safe_text_value(default_env_var);
        segments.push(format!("default_env_var={rendered_default_env_var}"));
    }

    if !plugin.setup_docs_urls.is_empty() {
        let setup_docs_urls = crate::render_line_safe_text_values(
            plugin.setup_docs_urls.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("setup_docs_urls={setup_docs_urls}"));
    }

    if let Some(setup_remediation) = &plugin.setup_remediation {
        let rendered_setup_remediation = crate::render_line_safe_text_value(setup_remediation);
        segments.push(format!("setup_remediation={rendered_setup_remediation}"));
    }

    segments.join(" ")
}

pub(super) fn render_u32_list(values: &[u32]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn render_runtime_incident_summary(incidents: &[String]) -> String {
    if incidents.is_empty() {
        return "-".to_owned();
    }

    incidents.join(",")
}

pub(super) fn render_managed_bridge_compatible_plugin_labels(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let duplicate_plugin_id_counts = managed_bridge_duplicate_plugin_id_counts(&discovery.plugins);
    let mut compatible_plugin_labels = Vec::new();

    for plugin in &discovery.plugins {
        let is_compatible =
            plugin.status == mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleReady;

        if !is_compatible {
            continue;
        }

        let plugin_label = managed_bridge_plugin_label(plugin, &duplicate_plugin_id_counts);
        compatible_plugin_labels.push(plugin_label);
    }

    crate::render_line_safe_text_values(compatible_plugin_labels.iter().map(String::as_str), ",")
}

pub(super) fn render_managed_bridge_configured_plugin_labels(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let configured_plugin_id = discovery.configured_plugin_id.as_deref();
    let Some(configured_plugin_id) = configured_plugin_id else {
        return "-".to_owned();
    };

    let duplicate_plugin_id_counts = managed_bridge_duplicate_plugin_id_counts(&discovery.plugins);
    let mut matching_plugin_labels = Vec::new();

    for plugin in &discovery.plugins {
        let matches_configured_plugin_id = plugin.plugin_id == configured_plugin_id;

        if !matches_configured_plugin_id {
            continue;
        }

        let plugin_label = managed_bridge_plugin_label(plugin, &duplicate_plugin_id_counts);
        matching_plugin_labels.push(plugin_label);
    }

    crate::render_line_safe_text_values(matching_plugin_labels.iter().map(String::as_str), ",")
}

pub(super) fn managed_bridge_duplicate_plugin_id_counts(
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

pub(super) fn managed_bridge_plugin_label(
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
