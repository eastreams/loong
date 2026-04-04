use crate::mvp;

pub(crate) fn push_channel_surface_plugin_bridge_contract(
    lines: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
) {
    let plugin_bridge_contract = surface.catalog.plugin_bridge_contract.as_ref();
    let Some(plugin_bridge_contract) = plugin_bridge_contract else {
        return;
    };

    let stable_targets =
        render_channel_surface_plugin_bridge_stable_targets(&plugin_bridge_contract.stable_targets);
    if stable_targets != "-" {
        let rendered_stable_targets = render_line_safe_text_value(&stable_targets);
        let stable_targets_line = format!("  stable_targets={rendered_stable_targets}");
        lines.push(stable_targets_line);
    }

    let account_scope_note = plugin_bridge_contract.account_scope_note;
    let Some(account_scope_note) = account_scope_note else {
        return;
    };

    let rendered_account_scope_note = render_line_safe_text_value(account_scope_note);
    let account_scope_line = format!("  account_scope_note={rendered_account_scope_note}");
    lines.push(account_scope_line);
}

fn render_channel_surface_plugin_bridge_stable_targets(
    stable_targets: &[mvp::channel::ChannelPluginBridgeStableTarget],
) -> String {
    if stable_targets.is_empty() {
        return "-".to_owned();
    }

    let rendered_targets = stable_targets
        .iter()
        .map(render_channel_surface_plugin_bridge_stable_target)
        .collect::<Vec<_>>();
    rendered_targets.join(",")
}

fn render_channel_surface_plugin_bridge_stable_target(
    stable_target: &mvp::channel::ChannelPluginBridgeStableTarget,
) -> String {
    format!(
        "{}[{}]:{}",
        stable_target.template,
        stable_target.target_kind.as_str(),
        stable_target.description,
    )
}

pub(crate) fn push_channel_surface_managed_plugin_bridge_discovery(
    lines: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
) {
    let Some(discovery) = surface.plugin_bridge_discovery.as_ref() else {
        return;
    };

    let managed_install_root =
        render_line_safe_optional_text_value(discovery.managed_install_root.as_deref());
    let scan_issue = render_line_safe_optional_text_value(discovery.scan_issue.as_deref());
    let status = discovery.status.as_str();
    let ambiguity_status = discovery
        .ambiguity_status
        .map(|value| value.as_str())
        .unwrap_or("-");
    let compatible_plugins = discovery.compatible_plugins;
    let compatible_plugin_ids = render_line_safe_text_values(
        discovery.compatible_plugin_ids.iter().map(String::as_str),
        ",",
    );
    let incomplete_plugins = discovery.incomplete_plugins;
    let incompatible_plugins = discovery.incompatible_plugins;

    lines.push(format!(
        "  managed_plugin_bridge_discovery status={} managed_install_root={} scan_issue={} compatible={} compatible_plugin_ids={} ambiguity_status={} incomplete={} incompatible={}",
        status,
        managed_install_root,
        scan_issue,
        compatible_plugins,
        compatible_plugin_ids,
        ambiguity_status,
        incomplete_plugins,
        incompatible_plugins,
    ));

    for plugin in &discovery.plugins {
        let rendered_plugin = render_channel_surface_discovered_plugin_line(plugin);
        lines.push(rendered_plugin);
    }
}

fn render_channel_surface_discovered_plugin_line(
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
) -> String {
    let plugin_id = render_line_safe_text_value(&plugin.plugin_id);
    let bridge_kind = render_line_safe_text_value(&plugin.bridge_kind);
    let adapter_family = render_line_safe_text_value(&plugin.adapter_family);
    let transport_family = render_line_safe_optional_text_value(plugin.transport_family.as_deref());
    let target_contract = render_line_safe_optional_text_value(plugin.target_contract.as_deref());
    let account_scope = render_line_safe_optional_text_value(plugin.account_scope.as_deref());
    let source_path = render_line_safe_text_value(&plugin.source_path);
    let package_root = render_line_safe_text_value(&plugin.package_root);
    let package_manifest_path =
        render_line_safe_optional_text_value(plugin.package_manifest_path.as_deref());
    let missing_fields =
        render_line_safe_text_values(plugin.missing_fields.iter().map(String::as_str), ",");
    let issues = render_line_safe_text_values(plugin.issues.iter().map(String::as_str), "|");
    let required_env_vars =
        render_line_safe_text_values(plugin.required_env_vars.iter().map(String::as_str), ",");
    let recommended_env_vars =
        render_line_safe_text_values(plugin.recommended_env_vars.iter().map(String::as_str), ",");
    let required_config_keys =
        render_line_safe_text_values(plugin.required_config_keys.iter().map(String::as_str), ",");
    let default_env_var = render_line_safe_optional_text_value(plugin.default_env_var.as_deref());
    let setup_docs_urls =
        render_line_safe_text_values(plugin.setup_docs_urls.iter().map(String::as_str), ",");
    let setup_remediation =
        render_line_safe_optional_text_value(plugin.setup_remediation.as_deref());

    format!(
        "    managed_plugin id={} status={} bridge_kind={} adapter_family={} transport_family={} target_contract={} account_scope={} source_path={} package_root={} package_manifest_path={} missing_fields={} issues={} required_env_vars={} recommended_env_vars={} required_config_keys={} default_env_var={} setup_docs_urls={} setup_remediation={}",
        plugin_id,
        plugin.status.as_str(),
        bridge_kind,
        adapter_family,
        transport_family,
        target_contract,
        account_scope,
        source_path,
        package_root,
        package_manifest_path,
        missing_fields,
        issues,
        required_env_vars,
        recommended_env_vars,
        required_config_keys,
        default_env_var,
        setup_docs_urls,
        setup_remediation,
    )
}

pub(crate) fn render_line_safe_text_value(raw: &str) -> String {
    let raw_is_line_safe = raw.chars().all(line_safe_unquoted_char);

    if raw_is_line_safe {
        return raw.to_owned();
    }

    match serde_json::to_string(raw) {
        Ok(value) => value,
        Err(_) => "\"<unrenderable-text>\"".to_owned(),
    }
}

pub(crate) fn render_line_safe_optional_text_value(raw: Option<&str>) -> String {
    match raw {
        Some(value) => render_line_safe_text_value(value),
        None => "-".to_owned(),
    }
}

pub(crate) fn render_line_safe_text_values<'a, I>(values: I, separator: &str) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut rendered_values = Vec::new();

    for value in values {
        let rendered_value = render_line_safe_text_value(value);
        rendered_values.push(rendered_value);
    }

    if rendered_values.is_empty() {
        return "-".to_owned();
    }

    rendered_values.join(separator)
}

fn line_safe_unquoted_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '.' | '-' | '_' | '/' | ':' | '@' | '+' | '~' | '?' | '#' | '%' | '&'
        )
}
