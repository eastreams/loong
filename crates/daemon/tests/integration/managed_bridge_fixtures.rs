use super::*;
use std::path::Path;

pub(crate) const MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY: &str =
    "configured_account=ops (default): ready; configured_account=backup: bridge_url is missing";

pub(crate) fn managed_bridge_manifest(
    channel_id: &str,
    setup_surface: Option<&str>,
    metadata: BTreeMap<String, String>,
) -> loongclaw_daemon::kernel::PluginManifest {
    let setup = setup_surface.map(|surface| loongclaw_daemon::kernel::PluginSetup {
        mode: loongclaw_daemon::kernel::PluginSetupMode::MetadataOnly,
        surface: Some(surface.to_owned()),
        required_env_vars: Vec::new(),
        recommended_env_vars: Vec::new(),
        required_config_keys: Vec::new(),
        default_env_var: None,
        docs_urls: Vec::new(),
        remediation: None,
    });

    managed_bridge_manifest_with_setup(channel_id, metadata, setup)
}

pub(crate) fn managed_bridge_manifest_with_setup(
    channel_id: &str,
    metadata: BTreeMap<String, String>,
    setup: Option<loongclaw_daemon::kernel::PluginSetup>,
) -> loongclaw_daemon::kernel::PluginManifest {
    let plugin_id = format!("{channel_id}-managed-bridge");

    managed_bridge_manifest_with_plugin_id(plugin_id.as_str(), channel_id, metadata, setup)
}

pub(crate) fn managed_bridge_manifest_with_plugin_id(
    plugin_id: &str,
    channel_id: &str,
    metadata: BTreeMap<String, String>,
    setup: Option<loongclaw_daemon::kernel::PluginSetup>,
) -> loongclaw_daemon::kernel::PluginManifest {
    loongclaw_daemon::kernel::PluginManifest {
        api_version: Some("v1alpha1".to_owned()),
        version: Some("1.0.0".to_owned()),
        plugin_id: plugin_id.to_owned(),
        provider_id: format!("{channel_id}-provider"),
        connector_name: format!("{channel_id}-connector"),
        channel_id: Some(channel_id.to_owned()),
        endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
        capabilities: BTreeSet::new(),
        trust_tier: loongclaw_daemon::kernel::PluginTrustTier::Unverified,
        metadata,
        summary: None,
        tags: Vec::new(),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        defer_loading: false,
        setup,
        slot_claims: Vec::new(),
        compatibility: None,
    }
}

pub(crate) fn managed_bridge_setup_with_guidance(
    surface: &str,
    required_env_vars: Vec<&str>,
    required_config_keys: Vec<&str>,
    docs_urls: Vec<&str>,
    remediation: Option<&str>,
) -> loongclaw_daemon::kernel::PluginSetup {
    let normalized_required_env_vars = required_env_vars.into_iter().map(str::to_owned).collect();
    let normalized_required_config_keys = required_config_keys
        .into_iter()
        .map(str::to_owned)
        .collect();
    let normalized_docs_urls = docs_urls.into_iter().map(str::to_owned).collect();
    let normalized_remediation = remediation.map(str::to_owned);

    loongclaw_daemon::kernel::PluginSetup {
        mode: loongclaw_daemon::kernel::PluginSetupMode::MetadataOnly,
        surface: Some(surface.to_owned()),
        required_env_vars: normalized_required_env_vars,
        recommended_env_vars: Vec::new(),
        required_config_keys: normalized_required_config_keys,
        default_env_var: None,
        docs_urls: normalized_docs_urls,
        remediation: normalized_remediation,
    }
}

pub(crate) fn compatible_managed_bridge_metadata(
    transport_family: &str,
    target_contract: &str,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();

    metadata.insert("adapter_family".to_owned(), "channel-bridge".to_owned());
    metadata.insert("transport_family".to_owned(), transport_family.to_owned());
    metadata.insert("target_contract".to_owned(), target_contract.to_owned());

    metadata
}

pub(crate) fn mixed_account_weixin_plugin_bridge_config() -> mvp::config::LoongClawConfig {
    serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "default_account": "ops",
            "accounts": {
                "ops": {
                    "enabled": true,
                    "account_id": "ops",
                    "bridge_url": "https://bridge.example.test/ops",
                    "bridge_access_token": "ops-token",
                    "allowed_contact_ids": ["wxid_ops"]
                },
                "backup": {
                    "enabled": true,
                    "account_id": "backup",
                    "bridge_access_token": "backup-token",
                    "allowed_contact_ids": ["wxid_backup"]
                }
            }
        }
    }))
    .expect("deserialize mixed-account weixin config")
}

pub(crate) fn install_ready_weixin_managed_bridge(install_root: &Path) {
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );

    std::fs::create_dir_all(install_root).expect("create managed bridge install root");
    write_managed_bridge_manifest(install_root, "weixin-managed-bridge", &manifest);
}

pub(crate) fn write_managed_bridge_manifest(
    install_root: &Path,
    directory_name: &str,
    manifest: &loongclaw_daemon::kernel::PluginManifest,
) {
    let plugin_directory = install_root.join(directory_name);
    let manifest_path = plugin_directory.join("loongclaw.plugin.json");
    let encoded_manifest =
        serde_json::to_string_pretty(manifest).expect("serialize managed bridge manifest");

    std::fs::create_dir_all(&plugin_directory).expect("create managed bridge plugin directory");
    std::fs::write(&manifest_path, encoded_manifest).expect("write managed bridge plugin manifest");
}
