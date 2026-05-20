use std::collections::{BTreeMap, BTreeSet};
use std::fs::Permissions;
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FEISHU_TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

use super::*;
use crate::test_support::ScopedEnv;
use kernel::AuditSink;
use mvp::channel::{
    ChannelOperationHealth, ChannelOperationRuntime, ChannelOperationStatus, ChannelStatusSnapshot,
};

fn runtime_plugin_temp_dir(label: &str) -> PathBuf {
    static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
    let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!(
        "loong-runtime-plugin-doctor-{label}-{}-{seed}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create runtime plugin temp dir");
    temp_dir
}

fn runtime_plugins_test_config(root: &Path, enabled: bool) -> mvp::config::LoongConfig {
    let mut config = mvp::config::LoongConfig::default();
    config.tools.file_root = Some(root.display().to_string());
    config.runtime_plugins.enabled = enabled;
    config.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
    config
}

fn sample_audit_event(
    event_id: &str,
    timestamp_epoch_s: u64,
    agent_id: Option<&str>,
    kind: kernel::AuditEventKind,
) -> kernel::AuditEvent {
    kernel::AuditEvent {
        event_id: event_id.to_owned(),
        timestamp_epoch_s,
        agent_id: agent_id.map(str::to_owned),
        kind,
    }
}

fn managed_bridge_manifest(
    channel_id: &str,
    setup_surface: Option<&str>,
    metadata: BTreeMap<String, String>,
) -> kernel::PluginManifest {
    let mut metadata = metadata;
    metadata
        .entry("channel_runtime_contract".to_owned())
        .or_insert_with(|| "loongclaw_channel_bridge_v1".to_owned());
    metadata
        .entry("channel_runtime_operations_json".to_owned())
        .or_insert_with(|| {
            serde_json::to_string(&[
                "send_message",
                "receive_batch",
                "ack_inbound",
                "complete_batch",
            ])
            .expect("serialize default runtime operations")
        });
    let setup = setup_surface.map(|surface| kernel::PluginSetup {
        mode: kernel::PluginSetupMode::MetadataOnly,
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

fn managed_bridge_manifest_with_setup(
    channel_id: &str,
    metadata: BTreeMap<String, String>,
    setup: Option<kernel::PluginSetup>,
) -> kernel::PluginManifest {
    let plugin_id = format!("{channel_id}-managed-bridge");

    managed_bridge_manifest_with_plugin_id(&plugin_id, channel_id, metadata, setup)
}

fn managed_bridge_manifest_with_plugin_id(
    plugin_id: &str,
    channel_id: &str,
    metadata: BTreeMap<String, String>,
    setup: Option<kernel::PluginSetup>,
) -> kernel::PluginManifest {
    kernel::PluginManifest {
        api_version: Some("v1alpha1".to_owned()),
        version: Some("1.0.0".to_owned()),
        plugin_id: plugin_id.to_owned(),
        provider_id: format!("{channel_id}-provider"),
        connector_name: format!("{channel_id}-connector"),
        channel_id: Some(channel_id.to_owned()),
        endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
        capabilities: BTreeSet::new(),
        trust_tier: kernel::PluginTrustTier::Unverified,
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

#[test]
fn select_doctor_first_turn_actions_skips_doctor_self_recursion() {
    let actions = vec![
        crate::next_actions::SetupNextAction {
            kind: crate::next_actions::SetupNextActionKind::Doctor,
            channel_action_id: None,
            label: "verify managed bridges".to_owned(),
            command: "loong doctor --config '/tmp/loong-config.toml'".to_owned(),
        },
        crate::next_actions::SetupNextAction {
            kind: crate::next_actions::SetupNextActionKind::Channel,
            channel_action_id: Some(crate::migration::channels::CHANNEL_CATALOG_ACTION_ID),
            label: "channels".to_owned(),
            command: "loong channels --config '/tmp/loong-config.toml'".to_owned(),
        },
    ];

    let selected = select_doctor_first_turn_actions(actions);

    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].kind,
        crate::next_actions::SetupNextActionKind::Channel
    );
    assert_eq!(selected[0].label, "channels");
    assert!(
        selected
            .iter()
            .all(|action| { action.kind != crate::next_actions::SetupNextActionKind::Doctor }),
        "doctor success follow-ups should not suggest running doctor again: {selected:#?}"
    );
}

fn managed_bridge_setup_with_guidance(
    surface: &str,
    required_env_vars: Vec<&str>,
    required_config_keys: Vec<&str>,
    docs_urls: Vec<&str>,
    remediation: Option<&str>,
) -> kernel::PluginSetup {
    let normalized_required_env_vars = required_env_vars.into_iter().map(str::to_owned).collect();
    let normalized_required_config_keys = required_config_keys
        .into_iter()
        .map(str::to_owned)
        .collect();
    let normalized_docs_urls = docs_urls.into_iter().map(str::to_owned).collect();
    let normalized_remediation = remediation.map(str::to_owned);

    kernel::PluginSetup {
        mode: kernel::PluginSetupMode::MetadataOnly,
        surface: Some(surface.to_owned()),
        required_env_vars: normalized_required_env_vars,
        recommended_env_vars: Vec::new(),
        required_config_keys: normalized_required_config_keys,
        default_env_var: None,
        docs_urls: normalized_docs_urls,
        remediation: normalized_remediation,
    }
}

fn compatible_managed_bridge_metadata(
    transport_family: &str,
    target_contract: &str,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    let runtime_operations = serde_json::to_string(&[
        "send_message",
        "receive_batch",
        "ack_inbound",
        "complete_batch",
    ])
    .expect("serialize runtime operations");

    metadata.insert("adapter_family".to_owned(), "channel-bridge".to_owned());
    metadata.insert("transport_family".to_owned(), transport_family.to_owned());
    metadata.insert("target_contract".to_owned(), target_contract.to_owned());
    metadata.insert(
        "channel_runtime_contract".to_owned(),
        mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1.to_owned(),
    );
    metadata.insert(
        "channel_runtime_operations_json".to_owned(),
        runtime_operations,
    );

    metadata
}

fn write_managed_bridge_manifest(
    install_root: &Path,
    directory_name: &str,
    manifest: &kernel::PluginManifest,
) {
    let plugin_directory = install_root.join(directory_name);
    let manifest_path = plugin_directory.join("loong.plugin.json");
    let encoded_manifest =
        serde_json::to_string_pretty(manifest).expect("serialize managed bridge manifest");

    std::fs::create_dir_all(&plugin_directory).expect("create managed bridge plugin directory");
    std::fs::write(&manifest_path, encoded_manifest).expect("write managed bridge plugin manifest");
}

struct PermissionRestore {
    path: PathBuf,
    permissions: Permissions,
}

impl PermissionRestore {
    fn new(path: PathBuf, permissions: Permissions) -> Self {
        Self { path, permissions }
    }
}

impl Drop for PermissionRestore {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
    }
}

#[test]
fn resolve_secret_prefers_inline_value() {
    let resolved = resolve_secret_value(Some(" inline-key "), Some("SHOULD_NOT_BE_USED"));
    assert_eq!(resolved.as_deref(), Some("inline-key"));
}

#[test]
fn resolve_secret_reads_env_value() {
    let resolved = resolve_secret_value(None, Some("PATH"));
    assert!(resolved.is_some());
}

#[test]
fn ensure_env_binding_fills_empty_slot() {
    let mut slot = None;
    let mut fixes = Vec::new();
    let changed = ensure_env_binding(&mut slot, "OPENAI_API_KEY", &mut fixes, "set provider");
    assert!(changed);
    assert_eq!(slot.as_deref(), Some("OPENAI_API_KEY"));
    assert_eq!(fixes.len(), 1);
}

#[test]
fn check_channel_surfaces_omit_disabled_channels() {
    let config = mvp::config::LoongConfig::default();
    let checks = check_channel_surfaces(&config);
    assert!(
        checks.is_empty(),
        "disabled optional channels should not generate doctor warnings by default: {checks:#?}"
    );
}

#[test]
fn build_channel_surface_checks_omit_disabled_registry_operations() {
    let snapshots = vec![ChannelStatusSnapshot {
        id: "telegram",
        configured_account_id: "ops".to_owned(),
        configured_account_label: "ops".to_owned(),
        is_default_account: true,
        default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
        label: "Telegram",
        aliases: Vec::new(),
        transport: "telegram_bot_api",
        compiled: true,
        enabled: false,
        api_base_url: Some("https://api.telegram.org".to_owned()),
        notes: Vec::new(),
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "serve",
            label: "event listener",
            command: "channels serve telegram",
            health: ChannelOperationHealth::Disabled,
            detail: "disabled by telegram account configuration".to_owned(),
            issues: Vec::new(),
            runtime: None,
        }],
    }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks.is_empty(),
        "disabled registry-backed operations should not emit live doctor checks: {checks:#?}"
    );
}

#[test]
fn build_channel_surface_checks_reports_plugin_bridge_contract_status_for_configured_surface() {
    let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        },
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret",
            "allowed_peer_ids": ["openid-alice"]
        },
        "onebot": {
            "enabled": true,
            "websocket_url": "ws://127.0.0.1:5700",
            "access_token": "onebot-token",
            "allowed_group_ids": ["123456"]
        }
    }))
    .expect("deserialize bridge-backed config");

    let checks = check_channel_surfaces(&config);

    assert!(checks.iter().any(|check| {
        check.name == "weixin bridge send contract" && check.level == DoctorCheckLevel::Pass
    }));
    assert!(checks.iter().any(|check| {
        check.name == "weixin bridge serve contract" && check.level == DoctorCheckLevel::Pass
    }));
    assert!(
        checks.iter().any(|check| {
            check.name == "qqbot channel" && check.level == DoctorCheckLevel::Pass
        })
    );
    assert!(checks.iter().any(|check| {
        check.name == "onebot bridge send contract" && check.level == DoctorCheckLevel::Pass
    }));
    assert!(checks.iter().any(|check| {
        check.name == "onebot bridge serve contract" && check.level == DoctorCheckLevel::Pass
    }));
}

#[test]
fn check_channel_surfaces_reports_managed_bridge_discovery_for_compatible_plugins() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-compatible");
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    write_managed_bridge_manifest(install_root.as_path(), "weixin-managed-bridge", &manifest);

    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);

    assert!(checks.iter().any(|check| {
        check.name == "weixin bridge send contract" && check.level == DoctorCheckLevel::Pass
    }));
    assert!(checks.iter().any(|check| {
        check.name == "weixin managed bridge discovery"
            && check.level == DoctorCheckLevel::Pass
            && check.detail.contains("compatible=1")
            && check.detail.contains("weixin-managed-bridge")
    }));
}

#[test]
fn check_channel_surfaces_warns_when_managed_bridge_discovery_is_ambiguous() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-ambiguous");
    let first_plugin_directory = "weixin-bridge-a";
    let second_plugin_directory = "weixin-bridge-b";
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
    second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

    write_managed_bridge_manifest(
        install_root.as_path(),
        first_plugin_directory,
        &first_manifest,
    );
    write_managed_bridge_manifest(
        install_root.as_path(),
        second_plugin_directory,
        &second_manifest,
    );

    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);

    assert!(checks.iter().any(|check| {
        check.name == "weixin managed bridge discovery"
            && check.level == DoctorCheckLevel::Warn
            && check
                .detail
                .contains("ambiguity_status=duplicate_compatible_plugin_ids")
            && check
                .detail
                .contains("compatible_plugin_ids=weixin-bridge-shared,weixin-bridge-shared")
            && check.detail.contains("package_root=")
            && check.detail.contains(first_plugin_directory)
            && check.detail.contains(second_plugin_directory)
    }));
}

#[test]
fn check_channel_surfaces_warns_when_configured_managed_bridge_plugin_id_is_duplicated() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-selection-duplicated");
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "weixin-bridge-shared",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
    second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);

    assert!(checks.iter().any(|check| {
        check.name == "weixin managed bridge discovery"
            && check.level == DoctorCheckLevel::Warn
            && check
                .detail
                .contains("configured_plugin_id=weixin-bridge-shared")
            && check
                .detail
                .contains("selection_status=configured_plugin_id_duplicated")
    }));
}

#[test]
fn check_channel_surfaces_warns_when_managed_bridge_discovery_only_finds_incomplete_plugins() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-incomplete");
    let mut metadata = compatible_managed_bridge_metadata(
        "qq_official_bot_gateway_or_plugin_bridge",
        "qqbot_reply_loop",
    );
    let removed_transport_family = metadata.remove("transport_family");
    let manifest = managed_bridge_manifest("qqbot", Some("channel"), metadata);
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret",
            "allowed_peer_ids": ["openid-alice"]
        }
    }))
    .expect("deserialize qqbot config");

    assert_eq!(
        removed_transport_family.as_deref(),
        Some("qq_official_bot_gateway_or_plugin_bridge")
    );

    write_managed_bridge_manifest(install_root.as_path(), "qqbot-incomplete-bridge", &manifest);

    config.skills.install_root = Some(install_root.display().to_string());

    let _ = check_channel_surfaces(&config);
}

#[test]
fn check_channel_surfaces_detail_includes_managed_bridge_setup_guidance() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-setup-guidance");
    let mut metadata = compatible_managed_bridge_metadata(
        "qq_official_bot_gateway_or_plugin_bridge",
        "qqbot_reply_loop",
    );
    let removed_transport_family = metadata.remove("transport_family");
    let setup = managed_bridge_setup_with_guidance(
        "channel",
        vec!["QQBOT_BRIDGE_URL"],
        vec!["qqbot.bridge_url"],
        vec!["https://example.test/docs/qqbot-bridge"],
        Some(
            "Run the QQ bridge setup flow before enabling this bridge.\nThen confirm exactly one managed bridge remains.",
        ),
    );
    let mut manifest = managed_bridge_manifest_with_setup("qqbot", metadata, Some(setup));
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret",
            "allowed_peer_ids": ["openid-alice"]
        }
    }))
    .expect("deserialize qqbot config");

    manifest.plugin_id = "qqbot-bridge-guided".to_owned();
    assert_eq!(
        removed_transport_family.as_deref(),
        Some("qq_official_bot_gateway_or_plugin_bridge")
    );

    write_managed_bridge_manifest(install_root.as_path(), "qqbot-bridge-guided", &manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let _ = check_channel_surfaces(&config);
}

#[test]
fn managed_plugin_bridge_discovery_detail_escapes_untrusted_values() {
    let discovery = mvp::channel::ChannelPluginBridgeDiscovery {
        managed_install_root: Some("/tmp/managed bridge".to_owned()),
        status: mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound,
        scan_issue: Some("scan failed\nplease inspect".to_owned()),
        configured_plugin_id: Some("bridge\none".to_owned()),
        selected_plugin_id: Some("bridge\none".to_owned()),
        selection_status: Some(
            mvp::channel::ChannelPluginBridgeSelectionStatus::SelectedCompatible,
        ),
        ambiguity_status: Some(
            mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins,
        ),
        compatible_plugins: 1,
        compatible_plugin_ids: vec!["bridge\none".to_owned()],
        incomplete_plugins: 1,
        incompatible_plugins: 0,
        plugins: vec![mvp::channel::ChannelDiscoveredPluginBridge {
            plugin_id: "qqbot bridge".to_owned(),
            source_path: "/tmp/plugin root/bridge\nplugin.json".to_owned(),
            package_root: "/tmp/plugin root".to_owned(),
            package_manifest_path: Some("/tmp/plugin root/manifest\tbridge.json".to_owned()),
            bridge_kind: "managed connector".to_owned(),
            adapter_family: "channel bridge".to_owned(),
            transport_family: Some("qq official".to_owned()),
            target_contract: Some("qqbot\nreply".to_owned()),
            account_scope: Some("shared scope".to_owned()),
            runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
            runtime_operations: vec!["send_message".to_owned(), "receive_batch".to_owned()],
            status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
            issues: vec!["missing\nfield".to_owned()],
            missing_fields: vec!["metadata.transport family".to_owned()],
            required_env_vars: vec!["QQBOT BRIDGE URL".to_owned()],
            recommended_env_vars: vec!["QQBOT BRIDGE TOKEN".to_owned()],
            required_config_keys: vec!["qqbot.bridge url".to_owned()],
            default_env_var: Some("QQBOT DEFAULT ENV".to_owned()),
            setup_docs_urls: vec!["https://example.test/docs bridge".to_owned()],
            setup_remediation: Some("fix bridge\nthen retry".to_owned()),
        }],
    };

    let surface = mvp::channel::ChannelSurface {
        catalog: mvp::channel::ChannelCatalogEntry {
            id: "qqbot",
            label: "QQBot",
            selection_order: 0,
            selection_label: "QQBot",
            blurb: "plugin bridge",
            aliases: Vec::new(),
            transport: "plugin_bridge",
            implementation_status: mvp::channel::ChannelCatalogImplementationStatus::PluginBacked,
            capabilities: Vec::new(),
            operations: Vec::new(),
            onboarding: mvp::channel::ChannelOnboardingDescriptor {
                strategy: mvp::channel::ChannelOnboardingStrategy::PluginBridge,
                setup_hint: "plugin bridge",
                status_command: "loong doctor",
                repair_command: None,
            },
            supported_target_kinds: Vec::new(),
            plugin_bridge_contract: Some(mvp::channel::ChannelPluginBridgeContract {
                manifest_channel_id: "qqbot",
                required_setup_surface: "channel",
                runtime_owner: "external_plugin",
                supported_operations: Vec::new(),
                recommended_metadata_keys: Vec::new(),
                stable_targets: Vec::new(),
                account_scope_note: None,
            }),
        },
        configured_accounts: Vec::new(),
        default_configured_account_id: None,
        plugin_bridge_discovery: Some(discovery.clone()),
    };
    let detail = managed_plugin_bridge_discovery_check_detail(&surface, &discovery);

    assert!(detail.contains("root=\"/tmp/managed bridge\""));
    assert!(detail.contains("compatible_plugin_ids=\"bridge\\none\""));
    assert!(detail.contains("\"qqbot bridge\""));
    assert!(detail.contains("target_contract=\"qqbot\\nreply\""));
    assert!(detail.contains("setup_docs_urls=\"https://example.test/docs bridge\""));
    assert!(detail.contains("setup_remediation=\"fix bridge\\nthen retry\""));
}

#[test]
fn managed_bridge_incomplete_setup_step_escapes_untrusted_values() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let surface = inventory
        .channel_surfaces
        .iter()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let plugin = mvp::channel::ChannelDiscoveredPluginBridge {
        plugin_id: "weixin bridge".to_owned(),
        source_path: "/tmp/plugin root/bridge\nplugin.json".to_owned(),
        package_root: "/tmp/plugin root".to_owned(),
        package_manifest_path: Some("/tmp/plugin root/manifest bridge.json".to_owned()),
        bridge_kind: "managed connector".to_owned(),
        adapter_family: "channel bridge".to_owned(),
        transport_family: Some("wechat clawbot".to_owned()),
        target_contract: Some("weixin reply".to_owned()),
        account_scope: Some("shared scope".to_owned()),
        runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
        runtime_operations: vec!["send_message".to_owned(), "receive_batch".to_owned()],
        status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
        issues: vec!["missing\nfield".to_owned()],
        missing_fields: vec!["metadata.transport family".to_owned()],
        required_env_vars: vec!["WEIXIN BRIDGE URL".to_owned()],
        recommended_env_vars: vec!["WEIXIN BRIDGE TOKEN".to_owned()],
        required_config_keys: vec!["weixin.bridge url".to_owned()],
        default_env_var: Some("WEIXIN DEFAULT ENV".to_owned()),
        setup_docs_urls: vec!["https://example.test/docs bridge".to_owned()],
        setup_remediation: Some("fix bridge\nthen retry".to_owned()),
    };
    let duplicate_plugin_id_counts =
        managed_bridge_duplicate_plugin_id_counts(std::slice::from_ref(&plugin));
    let step = managed_bridge_incomplete_setup_step(surface, &plugin, &duplicate_plugin_id_counts);

    assert!(step.contains("plugin \"weixin bridge\""));
    assert!(step.contains("required env: \"WEIXIN BRIDGE URL\""));
    assert!(step.contains("required config keys: \"weixin.bridge url\""));
    assert!(step.contains("docs: \"https://example.test/docs bridge\""));
    assert!(step.contains("remediation: \"fix bridge\\nthen retry\""));
}

#[test]
fn build_channel_surface_checks_fails_plugin_bridge_contract_when_serve_requirements_are_missing() {
    let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "qqbot": {
            "enabled": true,
            "app_id": "10001",
            "client_secret": "qqbot-secret"
        }
    }))
    .expect("deserialize qqbot config");

    let checks = check_channel_surfaces(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "qqbot channel" && check.level == DoctorCheckLevel::Pass
        })
    );
    assert!(checks.iter().any(|check| {
        check.name == "qqbot channel"
            && check.level == DoctorCheckLevel::Fail
            && check.detail.contains("allowed_peer_ids is empty")
    }));
}

#[test]
fn build_channel_surface_checks_fails_plugin_bridge_contract_when_surface_is_uncompiled() {
    let snapshots = vec![ChannelStatusSnapshot {
        id: "weixin",
        configured_account_id: "default".to_owned(),
        configured_account_label: "default".to_owned(),
        is_default_account: true,
        default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
        label: "Weixin",
        aliases: vec!["wechat", "wx"],
        transport: "wechat_clawbot_ilink_bridge",
        compiled: false,
        enabled: true,
        api_base_url: None,
        notes: vec!["bridge_runtime_owner=external_plugin".to_owned()],
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "send",
            label: "bridge send",
            command: "channels send weixin",
            health: ChannelOperationHealth::Unsupported,
            detail: "weixin bridge surface is unavailable in this build".to_owned(),
            issues: vec!["weixin bridge surface is unavailable in this build".to_owned()],
            runtime: None,
        }],
    }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(checks.iter().any(|check| {
        check.name == "weixin bridge send contract"
            && check.level == DoctorCheckLevel::Fail
            && check.detail.contains("unavailable in this build")
    }));
}

#[test]
fn channel_doctor_checks_report_enabled_channels_from_registry() {
    let mut config = mvp::config::LoongConfig::default();
    config.telegram.enabled = true;
    config.telegram.bot_token = Some(SecretRef::Inline("123456:test-token".to_owned()));
    config.telegram.allowed_chat_ids = vec![123_i64];
    config.feishu.enabled = true;
    config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(SecretRef::Inline("feishu-secret".to_owned()));
    config.matrix.enabled = true;
    config.matrix.access_token = Some(SecretRef::Inline("matrix-token".to_owned()));
    config.matrix.base_url = Some("https://matrix.example.org".to_owned());
    config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
    config.matrix.user_id = Some("@ops-bot:example.org".to_owned());

    let checks = check_channel_surfaces(&config);
    let names = checks
        .iter()
        .map(|check| check.name.as_str())
        .collect::<Vec<_>>();

    assert!(
        names.contains(&"telegram channel"),
        "telegram send/serve surfaces should appear in live doctor output: {checks:#?}"
    );
    assert!(
        names.contains(&"telegram channel runtime"),
        "ready telegram serve surfaces should emit runtime checks in live doctor output: {checks:#?}"
    );
    assert!(
        names.contains(&"matrix channel") && names.contains(&"matrix room sync"),
        "matrix send/serve surfaces should appear in live doctor output: {checks:#?}"
    );
    assert!(
        names.contains(&"matrix channel runtime"),
        "ready matrix serve surfaces should emit runtime checks in live doctor output: {checks:#?}"
    );
    assert!(
        names.contains(&"feishu channel") && names.contains(&"feishu inbound transport"),
        "feishu send/serve surfaces should appear in live doctor output: {checks:#?}"
    );
    assert!(
        checks
            .iter()
            .any(|check| check.name == "matrix room sync" && check.level == DoctorCheckLevel::Pass),
        "matrix serve configuration should stay healthy through the live doctor path: {checks:#?}"
    );
}

#[test]
fn channel_env_fix_uses_registered_channel_defaults() {
    let mut config = mvp::config::LoongConfig::default();
    config.telegram.bot_token_env = None;
    config.feishu.app_id_env = None;
    config.feishu.app_secret_env = None;
    config.feishu.verification_token_env = None;
    config.feishu.encrypt_key_env = None;
    config.matrix.access_token_env = None;

    let mut fixes = Vec::new();
    let changed = maybe_apply_channel_env_fix(&mut config, true, &mut fixes);

    assert!(changed);
    assert_eq!(
        config.telegram.bot_token_env.as_deref(),
        Some("TELEGRAM_BOT_TOKEN")
    );
    assert_eq!(config.feishu.app_id_env.as_deref(), Some("FEISHU_APP_ID"));
    assert_eq!(
        config.feishu.app_secret_env.as_deref(),
        Some("FEISHU_APP_SECRET")
    );
    assert!(
        config.feishu.verification_token_env.is_none(),
        "default feishu mode is websocket; doctor env fix must not set webhook verification_token_env"
    );
    assert!(
        config.feishu.encrypt_key_env.is_none(),
        "default feishu mode is websocket; doctor env fix must not set webhook encrypt_key_env"
    );
    assert_eq!(
        config.matrix.access_token_env.as_deref(),
        Some("MATRIX_ACCESS_TOKEN")
    );
    assert_eq!(fixes.len(), 4);
}

#[test]
fn provider_credential_env_hints_prioritize_oauth_defaults() {
    let provider = mvp::config::ProviderConfig::default();
    let hints = provider_credential_policy::provider_credential_env_hints(&provider);

    assert!(
        hints.contains(&"OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
        "doctor hints should include the provider's oauth default when available: {hints:?}"
    );
    assert!(
        hints.contains(&"OPENAI_API_KEY".to_owned()),
        "doctor hints should still include the api key fallback for providers that support both auth paths: {hints:?}"
    );
}

#[test]
fn provider_env_fix_prefers_oauth_default_when_available() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.api_key_env = None;
    config.provider.oauth_access_token_env = None;

    let mut fixes = Vec::new();
    let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

    assert!(changed);
    assert_eq!(
        config.provider.oauth_access_token,
        Some(loong_contracts::SecretRef::Env {
            env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
        })
    );
    assert_eq!(config.provider.api_key_env, None);
    assert_eq!(
        fixes,
        vec!["set provider.oauth_access_token.env=OPENAI_CODEX_OAUTH_TOKEN".to_owned()]
    );
}

#[test]
fn provider_env_fix_does_not_overwrite_inline_api_key() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.api_key = Some(loong_contracts::SecretRef::Inline(
        "inline-secret".to_owned(),
    ));
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let mut fixes = Vec::new();
    let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

    assert!(!changed);
    assert_eq!(
        config.provider.api_key,
        Some(loong_contracts::SecretRef::Inline(
            "inline-secret".to_owned(),
        ))
    );
    assert_eq!(config.provider.api_key_env, None);
    assert!(fixes.is_empty());
}

#[test]
fn provider_env_fix_does_not_overwrite_file_backed_api_key() {
    let mut config = mvp::config::LoongConfig::default();
    let credential_path = PathBuf::from("/tmp/openai-api-key.txt");
    config.provider.api_key = Some(loong_contracts::SecretRef::File {
        file: credential_path.clone(),
    });
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let mut fixes = Vec::new();
    let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

    assert!(!changed);
    assert_eq!(
        config.provider.api_key,
        Some(loong_contracts::SecretRef::File {
            file: credential_path,
        })
    );
    assert_eq!(config.provider.oauth_access_token, None);
    assert_eq!(config.provider.api_key_env, None);
    assert_eq!(config.provider.oauth_access_token_env, None);
    assert!(fixes.is_empty());
}

#[test]
fn provider_transport_doctor_check_warns_for_responses_compatibility_mode() {
    let provider = mvp::config::ProviderConfig {
        kind: mvp::config::ProviderKind::Deepseek,
        model: "deepseek-chat".to_owned(),
        wire_api: mvp::config::ProviderWireApi::Responses,
        ..mvp::config::ProviderConfig::default()
    };

    let check = provider_transport_doctor_check(&provider);

    assert_eq!(check.name, "provider transport");
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(
        check
            .detail
            .contains("retry chat_completions automatically"),
        "doctor should surface the automatic transport fallback in review mode: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_warns_for_explicit_model() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.model = "openai/gpt-5.1-codex".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(
        check.detail.contains("explicitly configured"),
        "doctor should explain that explicit-model runtime may still work when catalog probing fails: {check:#?}"
    );
}

#[test]
fn provider_model_probe_transport_failure_prioritizes_route_guidance() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.model = "custom-explicit-model".to_owned();

    let check = provider_model_probe_failure_check(
        &config,
        "provider model-list request failed on attempt 3/3: operation timed out".to_owned(),
    );

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        check
            .detail
            .contains(crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER),
        "transport probe failures should use the route-focused marker: {check:#?}"
    );
    assert!(
        !check.detail.contains("provider.model"),
        "transport probe failures should not suggest model-selection repair when the route is the real blocker: {check:#?}"
    );
    assert!(
        !check.detail.contains("below"),
        "doctor should not promise a later route-probe section that may not exist when collection is unavailable: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_fails_for_auto_model() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        check.detail.contains("OpenAI [openai]"),
        "doctor failures should still identify the active provider context: {check:#?}"
    );
    assert!(
        check.detail.contains("model = auto"),
        "doctor failures should explain why runtime cannot rely on an unresolved automatic model: {check:#?}"
    );
    assert!(
        check.detail.contains("provider.model"),
        "doctor failures should point users to an explicit provider.model remediation path: {check:#?}"
    );
    assert!(
        check.detail.contains("preferred_models"),
        "doctor failures should point users to preferred_models when catalog probing is unavailable: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_warns_for_preferred_model_fallbacks() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    config.provider.model = "auto".to_owned();
    config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(
        check.detail.contains("configured preferred"),
        "doctor should only advertise fallback continuation for explicitly configured preferred models: {check:#?}"
    );
    assert!(
        check.detail.contains("MiniMax-M2.5"),
        "doctor warning should surface the fallback candidate to keep remediation concrete: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_guides_reviewed_default_for_auto_model() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider rejected the model list".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        check.detail.contains("deepseek-chat"),
        "reviewed providers should point users to the reviewed onboarding default when doctor cannot list models: {check:#?}"
    );
    assert!(
        check.detail.contains("rerun onboarding"),
        "doctor should suggest rerunning onboarding to accept the reviewed model instead of leaving recovery implicit: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_includes_region_hint_for_zhipu() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Zhipu;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider returned status 401".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        check.detail.contains("https://api.z.ai"),
        "doctor probe failures should surface the alternate regional endpoint when auth can be region-bound: {check:#?}"
    );
}

#[test]
fn provider_model_probe_failure_skips_region_hint_for_non_auth_errors() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Zhipu;
    config.provider.model = "auto".to_owned();

    let check =
        provider_model_probe_failure_check(&config, "provider returned status 503".to_owned());

    assert_eq!(check.name, "provider model probe");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        !check.detail.contains("provider.base_url"),
        "non-auth doctor probe failures should not steer operators toward region endpoint changes: {check:#?}"
    );
}

#[test]
fn build_doctor_next_steps_includes_region_endpoint_step_for_minimax_probe_failures() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    let checks = vec![
        DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        },
        DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "MiniMax [minimax]: model catalog probe failed (provider returned status 401)"
                .to_owned(),
        },
    ];

    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.contains("provider.base_url")
                && step.contains("https://api.minimax.io")
                && step.contains("https://api.minimaxi.com")
        }),
        "doctor next steps should include a concrete region endpoint adjustment for MiniMax auth/probe failures: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_skips_region_endpoint_step_for_non_auth_probe_failures() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Minimax;
    let checks = vec![
        DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        },
        DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "MiniMax [minimax]: model catalog probe failed (provider returned status 503)"
                .to_owned(),
        },
    ];

    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        !next_steps
            .iter()
            .any(|step| step.contains("provider.base_url")),
        "doctor next steps should not include a region endpoint adjustment for non-auth probe failures: {next_steps:#?}"
    );
}

#[test]
fn audit_retention_doctor_check_warns_for_in_memory_mode() {
    let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::InMemory,
        ..mvp::config::AuditConfig::default()
    });

    assert_eq!(check.name, "audit retention");
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(check.detail.contains("lost on restart"));
}

#[test]
fn audit_integrity_doctor_check_warns_for_in_memory_mode() {
    let check = audit_integrity_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::InMemory,
        ..mvp::config::AuditConfig::default()
    });

    assert_eq!(check.name, "audit integrity");
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(
        check
            .detail
            .contains("unavailable while audit.mode=in_memory")
    );
}

#[test]
fn audit_integrity_doctor_check_passes_for_valid_chain() {
    let temp_dir = runtime_plugin_temp_dir("audit-integrity-valid");
    let journal_path = temp_dir.join("events.jsonl");
    let sink = kernel::JsonlAuditSink::new(journal_path.clone()).expect("create jsonl sink");

    sink.record(sample_audit_event(
        "evt-integrity-1",
        1_700_010_400,
        Some("agent-a"),
        kernel::AuditEventKind::TokenRevoked {
            token_id: "token-a".to_owned(),
        },
    ))
    .expect("record event");

    let check = audit_integrity_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Jsonl,
        path: journal_path.display().to_string(),
        retain_in_memory: false,
    });

    assert_eq!(check.name, "audit integrity");
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert!(check.detail.contains("verified 1 of 1 audit events"));
}

#[test]
fn audit_integrity_doctor_check_fails_for_tampered_chain() {
    let temp_dir = runtime_plugin_temp_dir("audit-integrity-tampered");
    let journal_path = temp_dir.join("events.jsonl");
    let sink = kernel::JsonlAuditSink::new(journal_path.clone()).expect("create jsonl sink");

    sink.record(sample_audit_event(
        "evt-integrity-1",
        1_700_010_410,
        Some("agent-a"),
        kernel::AuditEventKind::TokenRevoked {
            token_id: "token-a".to_owned(),
        },
    ))
    .expect("record event");

    sink.record(sample_audit_event(
        "evt-integrity-2",
        1_700_010_411,
        Some("agent-b"),
        kernel::AuditEventKind::TokenRevoked {
            token_id: "token-b".to_owned(),
        },
    ))
    .expect("record event");

    let contents = std::fs::read_to_string(&journal_path).expect("read audit journal");
    let tampered = contents.replacen("token-b", "token-x", 1);
    std::fs::write(&journal_path, tampered).expect("rewrite tampered journal");

    let check = audit_integrity_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Jsonl,
        path: journal_path.display().to_string(),
        retain_in_memory: false,
    });

    assert_eq!(check.name, "audit integrity");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(check.detail.contains("failed at line 2"));
}

#[test]
fn build_doctor_next_steps_guides_durable_audit_when_in_memory() {
    let checks = vec![DoctorCheck {
        name: "audit retention".to_owned(),
        level: DoctorCheckLevel::Warn,
        detail: "audit.mode=in_memory; security-critical audit evidence is lost on restart"
            .to_owned(),
    }];
    let config_path = PathBuf::from("/tmp/loong.toml");
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        &config_path,
        &mvp::config::LoongConfig::default(),
        false,
        None,
    );

    assert!(
        next_steps
            .iter()
            .any(|step| step == "Switch to durable audit retention: set [audit].mode = \"fanout\""),
        "doctor should recommend durable audit retention when audit remains in-memory: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_fix_when_audit_path_is_invalid() {
    let checks = vec![DoctorCheck {
        name: "audit retention".to_owned(),
        level: DoctorCheckLevel::Fail,
        detail: "audit.mode=fanout -> /tmp/audit exists but is not a regular file".to_owned(),
    }];
    let config_path = PathBuf::from("/tmp/loong.toml");
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        &config_path,
        &mvp::config::LoongConfig::default(),
        false,
        None,
    );

    assert!(
        next_steps
            .iter()
            .any(|step| step.contains("Point [audit].path at a writable journal file path")),
        "doctor should guide operators toward a writable audit journal target when durable audit retention is misconfigured: {next_steps:#?}"
    );
}

#[test]
fn audit_journal_directory_check_accepts_bare_relative_filename() {
    let mut fixes = Vec::new();
    let audit_path = PathBuf::from("events.jsonl");
    let directory = audit_path.parent().unwrap_or(Path::new("."));
    let check = check_audit_journal_directory(directory, false, &mut fixes);

    assert_eq!(check.name, "audit journal directory");
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert!(check.detail.contains("current working directory"));
    assert!(fixes.is_empty());
}

#[test]
fn audit_retention_doctor_check_fails_when_durable_path_is_directory() {
    let temp_dir = runtime_plugin_temp_dir("audit-target-directory");
    let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Fanout,
        path: temp_dir.display().to_string(),
        retain_in_memory: true,
    });

    assert_eq!(check.name, "audit retention");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(check.detail.contains("not a regular file"));
}

#[test]
fn audit_retention_doctor_check_fails_when_durable_path_is_readonly_file() {
    let temp_dir = runtime_plugin_temp_dir("audit-target-readonly");
    let journal_path = temp_dir.join("events.jsonl");
    std::fs::write(&journal_path, b"{}\n").expect("write audit journal fixture");
    let original_permissions = std::fs::metadata(&journal_path)
        .expect("audit journal metadata")
        .permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    std::fs::set_permissions(&journal_path, permissions)
        .expect("mark audit journal fixture readonly");
    let _permission_restore = PermissionRestore::new(journal_path.clone(), original_permissions);

    let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Jsonl,
        path: journal_path.display().to_string(),
        retain_in_memory: false,
    });

    assert_eq!(check.name, "audit retention");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(check.detail.contains("not writable"));
}

#[test]
fn audit_retention_doctor_check_fails_when_parent_path_is_not_a_directory() {
    let temp_dir = runtime_plugin_temp_dir("audit-target-parent-not-directory");
    let blocked_parent = temp_dir.join("readonly-audit");
    std::fs::write(&blocked_parent, b"not a directory").expect("create blocking parent file");

    let journal_path = blocked_parent.join("events.jsonl");
    let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Fanout,
        path: journal_path.display().to_string(),
        retain_in_memory: true,
    });

    assert_eq!(check.name, "audit retention");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        check
            .detail
            .contains(journal_path.display().to_string().as_str()),
        "expected failing detail to mention the blocked journal path, got: {}",
        check.detail
    );
}

#[test]
fn audit_retention_doctor_check_fails_when_missing_parent_chain_runs_into_file_boundary() {
    let temp_dir = runtime_plugin_temp_dir("audit-target-missing-parent-chain");
    let blocked_parent = temp_dir.join("readonly-audit");
    std::fs::write(&blocked_parent, b"not a directory").expect("create blocking parent file");

    let journal_path = blocked_parent.join("nested").join("events.jsonl");
    let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Fanout,
        path: journal_path.display().to_string(),
        retain_in_memory: true,
    });

    assert_eq!(check.name, "audit retention");
    assert_eq!(check.level, DoctorCheckLevel::Fail);
    assert!(
        check
            .detail
            .contains(journal_path.display().to_string().as_str()),
        "expected failing detail to mention the blocked journal path, got: {}",
        check.detail
    );
}

#[test]
fn audit_retention_doctor_check_cleans_up_probe_artifacts_for_creatable_missing_path() {
    let temp_dir = runtime_plugin_temp_dir("audit-target-cleanup");
    let journal_path = temp_dir.join("nested").join("events.jsonl");

    let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
        mode: mvp::config::AuditMode::Fanout,
        path: journal_path.display().to_string(),
        retain_in_memory: true,
    });

    assert_eq!(check.name, "audit retention");
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert!(!journal_path.exists());
    assert!(!journal_path.parent().expect("nested parent").exists());
}

fn unique_temp_feishu_db(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    let sequence = FEISHU_TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "loong-doctor-feishu-{label}-{}-{nanos}-{sequence}.sqlite3",
            std::process::id()
        ))
        .display()
        .to_string()
}

#[test]
fn feishu_integration_requested_is_false_for_default_config() {
    let config = mvp::config::FeishuChannelConfig::default();
    assert!(!feishu_integration_requested(&config));
}

#[test]
fn check_feishu_integration_warns_when_user_grants_are_missing() {
    let mut config = mvp::config::LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(SecretRef::Inline("app-secret".to_owned()));
    config.feishu_integration.sqlite_path = unique_temp_feishu_db("missing-grant");
    let mut fixes = Vec::new();

    let checks = check_feishu_integration(&config, false, &mut fixes);

    assert!(
        checks.iter().any(|check| {
            check.name == "feishu integration credentials" && check.level == DoctorCheckLevel::Pass
        }),
        "configured Feishu account should report available credentials"
    );
    assert!(
        checks.iter().any(|check| {
            check.level == DoctorCheckLevel::Warn
                && check.name.contains("feishu user grant")
                && check.detail.contains("missing stored user grant")
        }),
        "missing grants should warn rather than fail hard"
    );
}

#[test]
fn check_feishu_integration_passes_when_ready_grant_exists() {
    let mut config = mvp::config::LoongConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(SecretRef::Inline("app-secret".to_owned()));
    config.feishu_integration.sqlite_path = unique_temp_feishu_db("ready-grant");
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve default feishu account");
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(
        config.feishu_integration.resolved_sqlite_path(),
    );
    store
        .save_grant(&mvp::channel::feishu::api::FeishuGrant {
            principal: mvp::channel::feishu::api::FeishuUserPrincipal {
                account_id: resolved.account.id,
                open_id: "ou_123".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: Some("alice@example.com".to_owned()),
                enterprise_email: None,
            },
            access_token: "u-token".to_owned(),
            refresh_token: "r-token".to_owned(),
            scopes: mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes(
                config.feishu_integration.trimmed_default_scopes(),
            ),
            access_expires_at_s: chrono::Utc::now().timestamp() + 3600,
            refresh_expires_at_s: chrono::Utc::now().timestamp() + 86_400,
            refreshed_at_s: chrono::Utc::now().timestamp(),
        })
        .expect("save feishu grant");
    let mut fixes = Vec::new();

    let checks = check_feishu_integration(&config, false, &mut fixes);

    assert!(
        checks.iter().any(|check| {
            check.name.contains("feishu user grant")
                && check.level == DoctorCheckLevel::Pass
                && check.detail.contains("latest_open_id=ou_123")
        }),
        "stored grants should be visible to doctor"
    );
    assert!(
        checks.iter().any(|check| {
            check.name.contains("feishu token freshness") && check.level == DoctorCheckLevel::Pass
        }),
        "ready grants should upgrade Feishu integration health to pass"
    );
}

#[test]
fn build_channel_surface_checks_warns_when_ready_serve_operation_is_not_running() {
    let snapshots = vec![ChannelStatusSnapshot {
        id: "telegram",
        configured_account_id: "bot_123456".to_owned(),
        configured_account_label: "bot_123456".to_owned(),
        is_default_account: true,
        default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
        label: "Telegram",
        aliases: Vec::new(),
        transport: "telegram_bot_api_polling",
        compiled: true,
        enabled: true,
        api_base_url: Some("https://api.telegram.org".to_owned()),
        notes: Vec::new(),
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "serve",
            label: "reply loop",
            command: "channels serve telegram",
            health: ChannelOperationHealth::Ready,
            detail: "ready".to_owned(),
            issues: Vec::new(),
            runtime: Some(ChannelOperationRuntime {
                running: false,
                stale: false,
                busy: false,
                active_runs: 0,
                consecutive_failures: 0,
                last_run_activity_at: None,
                last_heartbeat_at: None,
                last_failure_at: None,
                last_recovery_at: None,
                last_error: None,
                last_duplicate_reclaim_at: None,
                pid: None,
                account_id: Some("bot_123456".to_owned()),
                account_label: Some("bot:123456".to_owned()),
                instance_count: 1,
                running_instances: 0,
                stale_instances: 0,
                duplicate_owner_pids: Vec::new(),
                last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
                recent_incidents: Vec::new(),
            }),
        }],
    }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks.iter().any(|check| {
            check.name == "telegram channel runtime"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("not currently running")
                && check.detail.contains("account=bot:123456")
        }),
        "ready telegram serve operation should emit runtime warning when not running"
    );
}

#[test]
fn build_channel_surface_checks_fails_when_ready_serve_operation_is_stale() {
    let snapshots = vec![ChannelStatusSnapshot {
        id: "feishu",
        configured_account_id: "feishu_cli_a1b2c3".to_owned(),
        configured_account_label: "feishu_cli_a1b2c3".to_owned(),
        is_default_account: true,
        default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
        label: "Feishu/Lark",
        aliases: vec!["lark"],
        transport: "feishu_openapi_webhook_or_websocket",
        compiled: true,
        enabled: true,
        api_base_url: Some("https://open.feishu.cn".to_owned()),
        notes: Vec::new(),
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "serve",
            label: "inbound reply service",
            command: "feishu serve",
            health: ChannelOperationHealth::Ready,
            detail: "ready".to_owned(),
            issues: Vec::new(),
            runtime: Some(ChannelOperationRuntime {
                running: false,
                stale: true,
                busy: true,
                active_runs: 1,
                consecutive_failures: 0,
                last_run_activity_at: Some(1_700_000_000_000),
                last_heartbeat_at: Some(1_700_000_005_000),
                last_failure_at: None,
                last_recovery_at: None,
                last_error: None,
                last_duplicate_reclaim_at: None,
                pid: Some(4242),
                account_id: Some("feishu_cli_a1b2c3".to_owned()),
                account_label: Some("feishu:cli_a1b2c3".to_owned()),
                instance_count: 1,
                running_instances: 0,
                stale_instances: 1,
                duplicate_owner_pids: Vec::new(),
                last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
                recent_incidents: Vec::new(),
            }),
        }],
    }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks.iter().any(|check| {
            check.name == "feishu serve runtime"
                && check.level == DoctorCheckLevel::Fail
                && check.detail.contains("stale")
                && check.detail.contains("pid=4242")
                && check.detail.contains("account=feishu:cli_a1b2c3")
        }),
        "stale feishu serve runtime should fail doctor checks"
    );
}

#[test]
fn build_channel_surface_checks_warns_when_multiple_runtime_instances_are_running() {
    let snapshots = vec![ChannelStatusSnapshot {
        id: "telegram",
        configured_account_id: "bot_123456".to_owned(),
        configured_account_label: "bot_123456".to_owned(),
        is_default_account: true,
        default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
        label: "Telegram",
        aliases: Vec::new(),
        transport: "telegram_bot_api_polling",
        compiled: true,
        enabled: true,
        api_base_url: Some("https://api.telegram.org".to_owned()),
        notes: Vec::new(),
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "serve",
            label: "reply loop",
            command: "channels serve telegram",
            health: ChannelOperationHealth::Ready,
            detail: "ready".to_owned(),
            issues: Vec::new(),
            runtime: Some(ChannelOperationRuntime {
                running: true,
                stale: false,
                busy: true,
                active_runs: 1,
                consecutive_failures: 0,
                last_run_activity_at: Some(1_700_000_000_000),
                last_heartbeat_at: Some(1_700_000_005_000),
                last_failure_at: None,
                last_recovery_at: None,
                last_error: None,
                last_duplicate_reclaim_at: Some(1_700_000_007_000),
                pid: Some(3003),
                account_id: Some("bot_123456".to_owned()),
                account_label: Some("bot:123456".to_owned()),
                instance_count: 2,
                running_instances: 2,
                stale_instances: 0,
                duplicate_owner_pids: vec![3003, 3004],
                last_duplicate_reclaim_cleanup_owner_pids: vec![3004],
                recent_incidents: Vec::new(),
            }),
        }],
    }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks.iter().any(|check| {
            check.name == "telegram channel runtime"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("multiple runtime instances")
                && check.detail.contains("running_instances=2")
        }),
        "duplicate running telegram runtimes should emit runtime warning"
    );
}

#[test]
fn build_channel_surface_checks_warns_when_runtime_is_retrying() {
    let snapshots = vec![ChannelStatusSnapshot {
        id: "weixin",
        configured_account_id: "default".to_owned(),
        configured_account_label: "default".to_owned(),
        is_default_account: true,
        default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
        label: "Weixin",
        aliases: vec!["wechat", "wx"],
        transport: "wechat_clawbot_ilink_bridge",
        compiled: true,
        enabled: true,
        api_base_url: None,
        notes: vec!["bridge_runtime_owner=external_plugin".to_owned()],
        reserved_runtime_fields: Vec::new(),
        operations: vec![ChannelOperationStatus {
            id: "serve",
            label: "managed bridge reply loop",
            command: "channels serve weixin",
            health: ChannelOperationHealth::Ready,
            detail: "ready".to_owned(),
            issues: Vec::new(),
            runtime: Some(ChannelOperationRuntime {
                running: true,
                stale: false,
                busy: false,
                active_runs: 0,
                consecutive_failures: 2,
                last_run_activity_at: Some(1_700_000_000_000),
                last_heartbeat_at: Some(1_700_000_005_000),
                last_failure_at: Some(1_700_000_006_000),
                last_recovery_at: None,
                last_error: Some("temporary bridge timeout".to_owned()),
                last_duplicate_reclaim_at: None,
                pid: Some(5151),
                account_id: Some("default".to_owned()),
                account_label: Some("default".to_owned()),
                instance_count: 1,
                running_instances: 1,
                stale_instances: 0,
                duplicate_owner_pids: Vec::new(),
                last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
                recent_incidents: Vec::new(),
            }),
        }],
    }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks.iter().any(|check| {
            check.name == "weixin bridge serve runtime"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("retrying after transient failures")
                && check.detail.contains("consecutive_failures=2")
                && check.detail.contains("last_error=temporary bridge timeout")
        }),
        "retrying runtime should surface failure metadata instead of passing silently: {checks:#?}"
    );
}

#[test]
fn build_channel_surface_checks_resolves_alias_metadata_from_channel_registry() {
    let snapshots = vec![ChannelStatusSnapshot {
            id: "lark",
            configured_account_id: "feishu_main".to_owned(),
            configured_account_label: "feishu_main".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Feishu/Lark",
            aliases: vec!["feishu"],
            transport: "feishu_openapi_webhook_or_websocket",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://open.feishu.cn".to_owned()),
            notes: vec![
                "webhook_inbound_message_types=text,image,file".to_owned(),
                "webhook_inbound_non_text_mode=structured_text_summary".to_owned(),
                "webhook_inbound_binary_fetch=disabled".to_owned(),
                "webhook_resource_download_tool=feishu.messages.resource.get".to_owned(),
                "webhook_resource_selection_mode=single_resource_default_or_unique_partial_inference_or_resource_inventory".to_owned(),
                "webhook_callback_event_types=card.action.trigger".to_owned(),
                "webhook_callback_response_mode=noop_json".to_owned(),
            ],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "inbound reply service",
                command: "feishu serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: true,
                    stale: false,
                    busy: false,
                    active_runs: 1,
                    consecutive_failures: 0,
                    last_run_activity_at: Some(1_700_000_000_000),
                    last_heartbeat_at: Some(1_700_000_005_000),
                    last_failure_at: None,
                    last_recovery_at: None,
                    last_error: None,
                    last_duplicate_reclaim_at: None,
                    pid: Some(4242),
                    account_id: Some("feishu_main".to_owned()),
                    account_label: Some("feishu:main".to_owned()),
                    instance_count: 1,
                    running_instances: 1,
                    stale_instances: 0,
                    duplicate_owner_pids: Vec::new(),
                    last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
                    recent_incidents: Vec::new(),
                }),
            }],
        }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks
            .iter()
            .any(|check| check.name == "feishu inbound transport"),
        "alias channel ids should reuse registry-backed operation-health metadata: {checks:#?}"
    );
    assert!(
        checks
            .iter()
            .any(|check| check.name == "feishu serve runtime"),
        "alias channel ids should reuse registry-backed runtime metadata: {checks:#?}"
    );
    assert!(
        checks
            .iter()
            .any(|check| check.name == "feishu webhook inbound support"),
        "alias channel ids should preserve feishu inbound support checks: {checks:#?}"
    );
}

#[test]
fn build_channel_surface_checks_reports_feishu_inbound_support_matrix() {
    let snapshots = vec![ChannelStatusSnapshot {
            id: "feishu",
            configured_account_id: "feishu_main".to_owned(),
            configured_account_label: "feishu_main".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Feishu/Lark",
            aliases: vec!["lark"],
            transport: "feishu_openapi_webhook_or_websocket",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://open.feishu.cn".to_owned()),
            notes: vec![
                "webhook_inbound_message_types=text,image,file,post,audio,media,folder,sticker,interactive,share_chat,share_user,system,location,video_chat,todo,vote,merge_forward,share_calendar_event,calendar,general_calendar".to_owned(),
                "webhook_inbound_non_text_mode=structured_text_summary".to_owned(),
                "webhook_inbound_binary_fetch=disabled".to_owned(),
                "webhook_resource_download_tool=feishu.messages.resource.get".to_owned(),
                "webhook_resource_selection_mode=single_resource_default_or_unique_partial_inference_or_resource_inventory".to_owned(),
                "webhook_callback_event_types=card.action.trigger,card.action.trigger_v1".to_owned(),
                "webhook_callback_response_mode=noop_json".to_owned(),
            ],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "inbound reply service",
                command: "feishu serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: None,
            }],
        }];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(checks.iter().any(|check| {
            check.name == "feishu webhook inbound support"
                && check.level == DoctorCheckLevel::Pass
                && check
                    .detail
                    .contains("text,image,file,post,audio,media,folder,sticker,interactive,share_chat,share_user,system,location,video_chat,todo,vote,merge_forward,share_calendar_event,calendar,general_calendar")
                && check.detail.contains("structured_text_summary")
                && check.detail.contains("binary_fetch=disabled")
                && check
                    .detail
                    .contains("resource_download_tool=feishu.messages.resource.get")
                && check.detail.contains(
                    "resource_selection_mode=single_resource_default_or_unique_partial_inference_or_resource_inventory"
                )
                && check
                    .detail
                    .contains("callback_event_types=card.action.trigger,card.action.trigger_v1")
                && check.detail.contains("callback_response_mode=noop_json")
        }));
}

#[test]
fn build_channel_surface_checks_scopes_names_for_multi_account_snapshots() {
    let snapshots = vec![
        ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "ops".to_owned(),
            configured_account_label: "ops".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: vec!["configured_account_id=ops".to_owned()],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "channels serve telegram",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: true,
                    stale: false,
                    busy: false,
                    active_runs: 0,
                    consecutive_failures: 0,
                    last_run_activity_at: None,
                    last_heartbeat_at: None,
                    last_failure_at: None,
                    last_recovery_at: None,
                    last_error: None,
                    last_duplicate_reclaim_at: None,
                    pid: Some(2001),
                    account_id: Some("bot_123456".to_owned()),
                    account_label: Some("bot:123456".to_owned()),
                    instance_count: 1,
                    running_instances: 1,
                    stale_instances: 0,
                    duplicate_owner_pids: Vec::new(),
                    last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
                    recent_incidents: Vec::new(),
                }),
            }],
        },
        ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "personal".to_owned(),
            configured_account_label: "personal".to_owned(),
            is_default_account: false,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: vec!["configured_account_id=personal".to_owned()],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "channels serve telegram",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: false,
                    stale: false,
                    busy: false,
                    active_runs: 0,
                    consecutive_failures: 0,
                    last_run_activity_at: None,
                    last_heartbeat_at: None,
                    last_failure_at: None,
                    last_recovery_at: None,
                    last_error: None,
                    last_duplicate_reclaim_at: None,
                    pid: None,
                    account_id: Some("bot_654321".to_owned()),
                    account_label: Some("bot:654321".to_owned()),
                    instance_count: 0,
                    running_instances: 0,
                    stale_instances: 0,
                    duplicate_owner_pids: Vec::new(),
                    last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
                    recent_incidents: Vec::new(),
                }),
            }],
        },
    ];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(
        checks
            .iter()
            .any(|check| check.name == "telegram channel [ops]")
    );
    assert!(
        checks
            .iter()
            .any(|check| check.name == "telegram channel runtime [personal]")
    );
}

#[test]
fn build_channel_surface_checks_warns_when_multi_account_default_uses_fallback() {
    let snapshots = vec![
        ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "alerts".to_owned(),
            configured_account_label: "alerts".to_owned(),
            is_default_account: true,
            default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::Fallback,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: vec!["default_account_source=fallback".to_owned()],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "channels serve telegram",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: None,
            }],
        },
        ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "work".to_owned(),
            configured_account_label: "work".to_owned(),
            is_default_account: false,
            default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::Fallback,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: vec!["default_account_source=fallback".to_owned()],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "channels serve telegram",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: None,
            }],
        },
    ];

    let checks = build_channel_surface_checks(&snapshots);

    assert!(checks.iter().any(|check| {
        check.name == "telegram default account policy"
            && check.level == DoctorCheckLevel::Warn
            && check.detail.contains("alerts")
            && check.detail.contains("default_account")
    }));
}

#[test]
fn build_channel_surface_checks_ignores_stub_surfaces_without_accounts() {
    let snapshots: Vec<mvp::channel::ChannelStatusSnapshot> = Vec::new();

    let checks = build_channel_surface_checks(&snapshots);

    assert!(checks.is_empty());
}

fn build_weixin_runtime_attention_surfaces(
    stale: bool,
    running_instances: usize,
    consecutive_failures: usize,
) -> (mvp::config::LoongConfig, Vec<mvp::channel::ChannelSurface>) {
    let mut config = mvp::config::LoongConfig::default();
    config.weixin.enabled = true;
    config.weixin.bridge_url = Some("https://bridge.example.test/weixin".to_owned());
    config.weixin.bridge_access_token = Some(loong_contracts::SecretRef::Inline(
        "weixin-token".to_owned(),
    ));
    config.weixin.allowed_contact_ids = vec!["wxid_alice".to_owned()];

    let mut inventory = mvp::channel::channel_inventory(&config);
    let surface = inventory
        .channel_surfaces
        .iter_mut()
        .find(|surface| surface.catalog.id == "weixin")
        .expect("weixin surface");
    let snapshot = surface
        .configured_accounts
        .iter_mut()
        .find(|snapshot| snapshot.configured_account_id == "default")
        .expect("weixin default account");
    let serve = snapshot
        .operations
        .iter_mut()
        .find(|operation| operation.id == mvp::channel::CHANNEL_OPERATION_SERVE_ID)
        .expect("weixin serve operation");
    serve.runtime = Some(mvp::channel::ChannelOperationRuntime {
        running: !stale,
        stale,
        busy: false,
        active_runs: 0,
        consecutive_failures,
        last_run_activity_at: Some(1_700_000_000_000),
        last_heartbeat_at: Some(1_700_000_005_000),
        last_failure_at: if consecutive_failures > 0 {
            Some(1_700_000_006_000)
        } else {
            None
        },
        last_recovery_at: None,
        last_error: if consecutive_failures > 0 {
            Some("temporary bridge timeout".to_owned())
        } else {
            None
        },
        last_duplicate_reclaim_at: if running_instances > 1 {
            Some(1_700_000_007_000)
        } else {
            None
        },
        pid: Some(5151),
        account_id: Some("default".to_owned()),
        account_label: Some("default".to_owned()),
        instance_count: running_instances.max(1),
        running_instances,
        stale_instances: usize::from(stale),
        duplicate_owner_pids: if running_instances > 1 {
            vec![5151, 6262]
        } else {
            Vec::new()
        },
        last_duplicate_reclaim_cleanup_owner_pids: if running_instances > 1 {
            vec![6262]
        } else {
            Vec::new()
        },
        recent_incidents: if consecutive_failures > 0 {
            vec![mvp::channel::ChannelOperationRuntimeIncident {
                at_ms: 1_700_000_006_000,
                kind: mvp::channel::ChannelOperationRuntimeIncidentKind::Failure,
                detail: Some("temporary bridge timeout".to_owned()),
                owner_pids: Vec::new(),
            }]
        } else if running_instances > 1 {
            vec![mvp::channel::ChannelOperationRuntimeIncident {
                at_ms: 1_700_000_007_000,
                kind: mvp::channel::ChannelOperationRuntimeIncidentKind::DuplicateReclaim,
                detail: Some(
                    "requested cooperative shutdown for duplicate runtime owners".to_owned(),
                ),
                owner_pids: vec![6262],
            }]
        } else {
            Vec::new()
        },
    });

    (config, inventory.channel_surfaces)
}

#[test]
fn build_doctor_next_steps_guides_fix_and_provider_credentials() {
    let checks = vec![
            DoctorCheck {
                name: "provider credentials".to_owned(),
                level: DoctorCheckLevel::Warn,
                detail: "provider credentials are missing (try env: OPENAI_CODEX_OAUTH_TOKEN, OPENAI_API_KEY)"
                    .to_owned(),
            },
            DoctorCheck {
                name: "memory path".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail: "/tmp/loong-memory is missing".to_owned(),
            },
        ];
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &mvp::config::LoongConfig::default(),
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert_eq!(
        next_steps[0],
        "Apply safe local repairs: loong doctor --config '/tmp/loong.toml' --fix"
    );
    assert!(
            next_steps.iter().any(|step| {
                step
                    == "Set provider credentials in env: OPENAI_CODEX_OAUTH_TOKEN or OPENAI_OAUTH_ACCESS_TOKEN or OPENAI_API_KEY"
            }),
            "doctor should turn missing provider auth into a concrete next step: {next_steps:#?}"
        );
    assert!(
        next_steps
            .iter()
            .any(|step| step == "Re-run diagnostics: loong doctor --config '/tmp/loong.toml'"),
        "doctor should tell the operator how to confirm the repair path: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_runtime_retry_diagnostics() {
    let checks = vec![DoctorCheck {
            name: "weixin bridge serve runtime".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "runtime is retrying after transient failures (account=default account_id=default pid=5151 busy=false active_runs=0 consecutive_failures=2 instance_count=1 running_instances=1 stale_instances=0 last_run_activity_at=1700000000000 last_heartbeat_at=1700000005000 last_failure_at=1700000006000 last_recovery_at=- last_error=temporary bridge timeout)".to_owned(),
        }];
    let (config, channel_surfaces) = build_weixin_runtime_attention_surfaces(false, 1, 2);

    let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        &channel_surfaces,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
            next_steps.iter().any(|step| {
                step == "Inspect Weixin bridge connectivity, upstream session health, and external bridge logs, then rerun diagnostics: loong doctor --config '/tmp/loong.toml'"
            }),
            "retrying runtime should produce a concrete bridge diagnostics step: {next_steps:#?}"
        );
}

#[test]
fn build_doctor_next_steps_guides_stale_runtime_recovery() {
    let checks = vec![DoctorCheck {
            name: "weixin bridge serve runtime".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "stale runtime detected (account=default account_id=default pid=5151 busy=false active_runs=0 consecutive_failures=0 instance_count=1 running_instances=0 stale_instances=1 last_run_activity_at=1700000000000 last_heartbeat_at=1700000005000 last_failure_at=- last_recovery_at=- last_error=-)".to_owned(),
        }];
    let (config, channel_surfaces) = build_weixin_runtime_attention_surfaces(true, 0, 0);

    let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        &channel_surfaces,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
            next_steps.iter().any(|step| {
                step == "Restart the stale Weixin runtime or external bridge owner: loong channels serve weixin --config '/tmp/loong.toml' --stop --account 'default'"
            }),
            "stale runtime should produce a restart-oriented recovery step: {next_steps:#?}"
        );
}

#[test]
fn build_doctor_next_steps_guides_duplicate_runtime_cleanup() {
    let checks = vec![DoctorCheck {
            name: "weixin bridge serve runtime".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "multiple runtime instances detected (account=default account_id=default pid=5151 busy=false active_runs=0 consecutive_failures=0 instance_count=2 running_instances=2 stale_instances=0 last_run_activity_at=1700000000000 last_heartbeat_at=1700000005000 last_failure_at=- last_recovery_at=- last_error=-)".to_owned(),
        }];
    let (config, channel_surfaces) = build_weixin_runtime_attention_surfaces(false, 2, 0);

    let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        &channel_surfaces,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
            next_steps.iter().any(|step| {
                step == "Stop duplicate Weixin runtime instances so only one serve owner remains (last auto reclaim at=1700000007000; last auto cleanup pids=6262; keep pid=5151; cleanup pids=6262; run loong channels serve weixin --config '/tmp/loong.toml' --stop-duplicates --account 'default')"
            }),
            "duplicate runtime attention should produce a cleanup-oriented recovery step: {next_steps:#?}"
        );
}

#[test]
fn build_doctor_next_steps_guides_managed_bridge_incomplete_setup() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-next-steps-incomplete");
    let mut metadata =
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop");
    let removed_transport_family = metadata.remove("transport_family");
    let setup = managed_bridge_setup_with_guidance(
        "channel",
        vec!["WEIXIN_BRIDGE_URL"],
        vec!["weixin.bridge_url"],
        vec!["https://example.test/docs/weixin-bridge"],
        Some(
            "Run the WeChat bridge setup flow before enabling this bridge.\nThen confirm exactly one managed bridge remains.",
        ),
    );
    let mut manifest = managed_bridge_manifest_with_setup("weixin", metadata, Some(setup));
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "http://localhost:9999"
        }
    }))
    .expect("deserialize weixin config");

    manifest.plugin_id = "weixin-bridge-guided".to_owned();
    assert_eq!(
        removed_transport_family.as_deref(),
        Some("wechat_clawbot_ilink_bridge")
    );

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-guided", &manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| { step.contains("weixin") }),
        "weixin should appear in managed bridge next steps: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_managed_bridge_ambiguity_resolution() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-next-steps-ambiguity");
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
    second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.contains("Resolve managed bridge ambiguity for weixin")
                && step.contains("weixin-bridge-shared@")
                && step.contains("weixin-bridge-a")
                && step.contains("weixin-bridge-b")
        }),
        "doctor should add a deterministic de-ambiguation step when multiple compatible managed bridges are discovered: {next_steps:#?}"
    );
}

#[test]
fn check_channel_surfaces_warns_when_configured_managed_bridge_plugin_id_is_missing() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-selection-missing");
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "missing-bridge",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-a".to_owned();
    second_manifest.plugin_id = "weixin-bridge-b".to_owned();

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);

    assert!(checks.iter().any(|check| {
        check.name == "weixin managed bridge discovery"
            && check.level == DoctorCheckLevel::Warn
            && check.detail.contains("configured_plugin_id=missing-bridge")
            && check
                .detail
                .contains("selection_status=configured_plugin_not_found")
    }));
}

#[test]
fn check_channel_surfaces_summarizes_multi_account_bridge_state_in_discovery_detail() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-discovery-multi-account");
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "default_account": "ops",
            "accounts": {
                "ops": {
                    "enabled": true,
                    "bridge_url": "https://bridge.example.test/ops",
                    "bridge_access_token": "ops-token",
                    "allowed_contact_ids": ["wxid_ops"]
                },
                "backup": {
                    "enabled": true,
                    "bridge_access_token": "backup-token",
                    "allowed_contact_ids": ["wxid_backup"]
                }
            }
        }
    }))
    .expect("deserialize weixin config");

    write_managed_bridge_manifest(install_root.as_path(), "weixin-managed-bridge", &manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);

    assert!(checks.iter().any(|check| {
        check.name == "weixin managed bridge discovery"
            && check.level == DoctorCheckLevel::Pass
            && check
                .detail
                .contains("selected_plugin_id=weixin-managed-bridge")
            && check.detail.contains("configured_account=ops")
            && check.detail.contains("(default): ready")
            && check.detail.contains("configured_account=backup")
            && check.detail.contains("bridge_url is missing")
    }));
}

#[test]
fn doctor_json_checks_include_plugin_bridge_account_summary_for_mixed_multi_account_surface() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-json-multi-account");
    let manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "default_account": "ops",
            "accounts": {
                "ops": {
                    "enabled": true,
                    "bridge_url": "https://bridge.example.test/ops",
                    "bridge_access_token": "ops-token",
                    "allowed_contact_ids": ["wxid_ops"]
                },
                "backup": {
                    "enabled": true,
                    "bridge_access_token": "backup-token",
                    "allowed_contact_ids": ["wxid_backup"]
                }
            }
        }
    }))
    .expect("deserialize weixin config");

    write_managed_bridge_manifest(install_root.as_path(), "weixin-managed-bridge", &manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let inventory = mvp::channel::channel_inventory(&config);
    let checks = collect_channel_surface_checks(&inventory);
    let payload = doctor_checks_json_payload(&checks, &inventory.channel_surfaces);
    let discovery_check = payload
        .iter()
        .find(|value| value["name"].as_str() == Some("weixin managed bridge discovery"))
        .expect("weixin discovery payload");

    assert_eq!(
        discovery_check["plugin_bridge_account_summary"]
            .as_str()
            .expect("plugin bridge account summary string"),
        "configured_account=ops (default): ready; configured_account=backup: bridge_url is missing"
    );
}

#[test]
fn doctor_json_checks_include_runtime_attention_metadata() {
    let checks = vec![DoctorCheck {
            name: "weixin bridge serve runtime".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "runtime is retrying after transient failures (account=default account_id=default pid=5151 busy=false active_runs=0 consecutive_failures=2 instance_count=1 running_instances=1 stale_instances=0 last_run_activity_at=1700000000000 last_heartbeat_at=1700000005000 last_failure_at=1700000006000 last_recovery_at=- last_error=temporary bridge timeout)".to_owned(),
        }];
    let (_config, channel_surfaces) = build_weixin_runtime_attention_surfaces(false, 1, 2);
    let payload = doctor_checks_json_payload(&checks, &channel_surfaces);
    let runtime_check = payload.first().expect("runtime check payload");

    assert_eq!(
        runtime_check["runtime_attention"]["channel_id"]
            .as_str()
            .expect("runtime attention channel id"),
        "weixin"
    );
    assert_eq!(
        runtime_check["runtime_attention"]["reason"]
            .as_str()
            .expect("runtime attention reason"),
        "retrying"
    );
    assert_eq!(
        runtime_check["runtime_attention"]["remediation"]
            .as_str()
            .expect("runtime attention remediation"),
        "inspect_bridge_connectivity"
    );
    assert_eq!(
        runtime_check["runtime_attention"]["recent_incidents"][0]["kind"]
            .as_str()
            .expect("runtime attention incident kind"),
        "failure"
    );
}

#[test]
fn build_doctor_next_steps_guides_missing_managed_bridge_selection_resolution() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-next-steps-selection-missing");
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "missing-bridge",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-a".to_owned();
    second_manifest.plugin_id = "weixin-bridge-b".to_owned();

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.contains("Fix managed bridge selection for weixin")
                && step.contains("managed_bridge_plugin_id=missing-bridge")
                && step.contains("weixin-bridge-a,weixin-bridge-b")
        }),
        "doctor should guide users toward a valid configured managed bridge selection: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_duplicate_managed_bridge_selection_resolution() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-next-steps-selection-duplicated");
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "managed_bridge_plugin_id": "weixin-bridge-shared",
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
    second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.contains("Fix managed bridge selection for weixin")
                && step.contains("managed_bridge_plugin_id=weixin-bridge-shared")
                && step.contains("weixin-bridge-shared@")
                && step.contains("weixin-bridge-a")
                && step.contains("weixin-bridge-b")
        }),
        "doctor should guide operators to remove or rename duplicate managed bridge packages when configured selection is not unique: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_with_channel_surfaces_keeps_managed_bridge_snapshot_stable() {
    let install_root = runtime_plugin_temp_dir("managed-bridge-next-steps-snapshot");
    let mut first_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut second_manifest = managed_bridge_manifest(
        "weixin",
        Some("channel"),
        compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
    );
    let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
        "weixin": {
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "bridge_access_token": "weixin-token",
            "allowed_contact_ids": ["wxid_alice"]
        }
    }))
    .expect("deserialize weixin config");

    first_manifest.plugin_id = "weixin-bridge-a".to_owned();
    second_manifest.plugin_id = "weixin-bridge-b".to_owned();

    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
    write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
    config.skills.install_root = Some(install_root.display().to_string());

    let checks = check_channel_surfaces(&config);
    let inventory = mvp::channel::channel_inventory(&config);
    let removed_plugin_directory = install_root.as_path().join("weixin-bridge-b");

    std::fs::remove_dir_all(&removed_plugin_directory)
        .expect("remove second managed bridge after checks");

    let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        &inventory.channel_surfaces,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.contains("Resolve managed bridge ambiguity for weixin")
                && step.contains("weixin-bridge-a,weixin-bridge-b")
        }),
        "doctor next steps should stay anchored to the same discovery snapshot as the checks even if the managed install root changes afterward: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_use_grouped_runtime_snapshot_command() {
    let checks = vec![DoctorCheck {
        name: "runtime plugins inventory".to_owned(),
        level: DoctorCheckLevel::Fail,
        detail: "missing manifest".to_owned(),
    }];
    let config = mvp::config::LoongConfig::default();

    let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        &[],
        false,
        None,
    );

    assert!(
            next_steps.iter().any(|step| {
                step == "Inspect runtime plugin inventory: loong runtime snapshot --json --config '/tmp/loong.toml'"
            }),
            "doctor next steps should point to the grouped runtime snapshot surface: {next_steps:#?}"
        );
}

#[test]
fn provider_credentials_doctor_check_adds_volcengine_auth_guidance() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Volcengine;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;
    let auth_env_names = config.provider.auth_hint_env_names();
    let mut env = ScopedEnv::new();
    for env_name in auth_env_names {
        env.remove(env_name);
    }

    let check = provider_credentials_doctor_check(&config, false);

    assert_eq!(check.name, "provider credentials");
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(check.detail.contains("ARK_API_KEY"));
    assert!(check.detail.contains("Authorization: Bearer <ARK_API_KEY>"));
}

#[test]
fn provider_credentials_doctor_check_passes_for_auth_optional_provider() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Ollama;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let check = provider_credentials_doctor_check(&config, false);

    assert_eq!(check.name, "provider credentials");
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert!(check.detail.contains("optional for this provider"));
}

#[test]
fn provider_credentials_doctor_check_reports_x_api_key_env_credentials() {
    let mut env = ScopedEnv::new();
    env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Anthropic;
    config.provider.api_key = None;
    config.provider.api_key_env = None;
    config.provider.oauth_access_token = None;
    config.provider.oauth_access_token_env = None;

    let check = provider_credentials_doctor_check(&config, true);

    assert_eq!(check.name, "provider credentials");
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert!(check.detail.contains("ANTHROPIC_API_KEY is available"));
}

#[test]
fn web_search_provider_doctor_check_warns_when_firecrawl_credential_is_missing() {
    let mut env = ScopedEnv::new();
    let mut config = mvp::config::LoongConfig::default();
    let provider_id = mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
    let configured_secret = "${FIRECRAWL_API_KEY}".to_owned();

    env.remove("FIRECRAWL_API_KEY");
    config.tools.web_search.default_provider = provider_id;
    config.tools.web_search.firecrawl_api_key = Some(configured_secret);

    let check = web_search_provider_doctor_check(&config);

    assert_eq!(check.name, crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL);
    assert_eq!(check.level, DoctorCheckLevel::Warn);
    assert!(check.detail.contains("Firecrawl Search"));
    assert!(check.detail.contains("FIRECRAWL_API_KEY"));
    assert!(check.detail.contains("web.search will stay unavailable"));
}

#[test]
fn web_search_provider_doctor_check_passes_when_firecrawl_credential_is_available() {
    let mut config = mvp::config::LoongConfig::default();
    let provider_id = mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
    let configured_secret = "${FIRECRAWL_API_KEY}".to_owned();
    let mut env = ScopedEnv::new();

    env.set("FIRECRAWL_API_KEY", "firecrawl-test-token");
    config.tools.web_search.default_provider = provider_id;
    config.tools.web_search.firecrawl_api_key = Some(configured_secret);

    let check = web_search_provider_doctor_check(&config);

    assert_eq!(check.name, crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL);
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert!(check.detail.contains("Firecrawl Search"));
    assert!(check.detail.contains("FIRECRAWL_API_KEY"));
}

#[test]
fn web_search_provider_doctor_check_passes_when_tool_is_disabled() {
    let mut config = mvp::config::LoongConfig::default();

    config.tools.web_search.enabled = false;
    config.tools.web_search.default_provider =
        mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();

    let check = web_search_provider_doctor_check(&config);

    assert_eq!(check.name, crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL);
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert_eq!(check.detail, "tools.web_search.enabled=false");
}

#[test]
fn web_search_provider_doctor_check_passes_when_openai_native_search_is_available() {
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Openai;
    config.provider.wire_api = mvp::config::ProviderWireApi::Responses;
    config.tools.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
    config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());

    let check = web_search_provider_doctor_check(&config);

    assert_eq!(check.name, crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL);
    assert_eq!(check.level, DoctorCheckLevel::Pass);
    assert_eq!(check.detail, "OpenAI Responses native web search");
}

#[test]
fn build_doctor_next_steps_shell_quotes_config_paths_with_single_quotes() {
    let checks = vec![DoctorCheck {
        name: "memory path".to_owned(),
        level: DoctorCheckLevel::Fail,
        detail: "/tmp/loong-memory is missing".to_owned(),
    }];
    let next_steps = build_doctor_next_steps(
        &checks,
        Path::new("/tmp/loong's config.toml"),
        &mvp::config::LoongConfig::default(),
        false,
    );

    assert!(
            next_steps.iter().any(|step| {
                step == "Apply safe local repairs: loong doctor --config '/tmp/loong'\"'\"'s config.toml' --fix"
            }),
            "doctor should shell-quote config paths with single quotes in fix commands: {next_steps:#?}"
        );
    assert!(
        next_steps.iter().any(|step| {
            step == "Re-run diagnostics: loong doctor --config '/tmp/loong'\"'\"'s config.toml'"
        }),
        "doctor should shell-quote config paths with single quotes in rerun commands: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_missing_web_search_credentials() {
    let checks = vec![DoctorCheck {
            name: crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL.to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "Firecrawl Search: FIRECRAWL_API_KEY (expected). web.search will stay unavailable until the provider credential is supplied".to_owned(),
        }];
    let mut config = mvp::config::LoongConfig::default();
    let config_path = Path::new("/tmp/loong.toml");

    config.tools.web_search.default_provider =
        mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();

    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        config_path,
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );
    let rerun_onboard_command =
        crate::cli_handoff::format_subcommand_with_config("onboard", "/tmp/loong.toml");
    let expected_onboard_step = crate::access_terms::review_query_search_provider_choice_step(
        rerun_onboard_command.as_str(),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.as_str()
                == crate::access_terms::set_query_search_credential_step("FIRECRAWL_API_KEY")
        }),
        "doctor should surface the missing Firecrawl env binding as a concrete next step: {next_steps:#?}"
    );
    assert!(
        next_steps.iter().any(|step| step == &expected_onboard_step),
        "doctor should keep the onboarding recovery path explicit for query search credentials: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_reviewed_onboarding_default_for_auto_model_probe_failures() {
    let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); current config still uses `model = auto`; rerun onboarding and accept reviewed model `deepseek-chat`, or set `provider.model` / `preferred_models` explicitly".to_owned(),
        }];
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();

    let next_steps = build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

    assert!(
            next_steps.iter().any(|step| {
                step == "Rerun onboarding and accept reviewed model `deepseek-chat`: loong onboard --config '/tmp/loong.toml'"
            }),
            "doctor should point reviewed providers back to onboarding when auto-model recovery needs an explicit reviewed default: {next_steps:#?}"
        );
    assert!(
            next_steps.iter().any(|step| {
                step == "Or set `provider.model` / `preferred_models` explicitly, then re-run diagnostics: loong doctor --config '/tmp/loong.toml'"
            }),
            "doctor should also keep the manual remediation path explicit for operators who do not want to rerun onboarding: {next_steps:#?}"
        );
    assert!(
        next_steps
            .iter()
            .all(|step| !step.contains("--skip-model-probe")),
        "doctor should not suggest --skip-model-probe when the real blocker is still `model = auto` without explicit recovery candidates: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_guides_warn_level_explicit_model_probe_recovery() {
    let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); chat may still work because model `deepseek-chat` is explicitly configured".to_owned(),
        }];
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-chat".to_owned();

    let next_steps = build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

    assert!(
            next_steps.iter().any(|step| {
                step == "Retry provider probe only after credentials are ready: loong doctor --config '/tmp/loong.toml'"
            }),
            "warn-level explicit model recovery should still tell operators how to retry diagnostics: {next_steps:#?}"
        );
    assert!(
            next_steps.iter().any(|step| {
                step == "If your provider blocks model listing during setup, retry with: loong doctor --config '/tmp/loong.toml' --skip-model-probe"
            }),
            "warn-level explicit model recovery should still keep the skip-model-probe escape hatch visible: {next_steps:#?}"
        );
}

#[test]
fn build_doctor_next_steps_guides_warn_level_preferred_model_probe_recovery() {
    let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); runtime will try configured preferred model fallback(s): `deepseek-chat`, `deepseek-reasoner`".to_owned(),
        }];
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "auto".to_owned();
    config.provider.preferred_models =
        vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()];

    let next_steps = build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

    assert!(
            next_steps.iter().any(|step| {
                step == "Retry provider probe only after credentials are ready: loong doctor --config '/tmp/loong.toml'"
            }),
            "warn-level preferred-model recovery should still tell operators how to retry diagnostics: {next_steps:#?}"
        );
    assert!(
            next_steps.iter().any(|step| {
                step == "If your provider blocks model listing during setup, retry with: loong doctor --config '/tmp/loong.toml' --skip-model-probe"
            }),
            "warn-level preferred-model recovery should still keep the skip-model-probe escape hatch visible: {next_steps:#?}"
        );
}

#[test]
fn build_doctor_next_steps_guides_provider_route_probe_repairs() {
    let checks = vec![
            DoctorCheck {
                name: "provider model probe".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail:
                    "OpenAI [openai]: model catalog transport failed (provider model-list request failed on attempt 3/3: operation timed out)"
                        .to_owned(),
            },
            DoctorCheck {
                name: "provider route probe".to_owned(),
                level: DoctorCheckLevel::Warn,
                detail:
                    "request/models host api.openai.com:443: dns resolved to 198.18.0.2 (fake-ip-style); tcp connect ok. the route currently depends on local fake-ip/TUN interception."
                        .to_owned(),
            },
        ];

    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &mvp::config::LoongConfig::default(),
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step.contains("provider route")
                && step.contains("loong doctor --config '/tmp/loong.toml'")
        }),
        "route-probe findings should produce a concrete diagnostics rerun step: {next_steps:#?}"
    );
    assert!(
        next_steps.iter().any(|step| {
            step.contains("fake-ip") || step.contains("direct/bypass") || step.contains("proxy")
        }),
        "route-probe findings should explain how to repair proxy/fake-ip routing instead of leaving recovery implicit: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_ignores_non_failure_model_probe_warnings() {
    let checks = vec![DoctorCheck {
        name: "provider model probe".to_owned(),
        level: DoctorCheckLevel::Warn,
        detail: "skipped because credentials are missing".to_owned(),
    }];
    let mut config = mvp::config::LoongConfig::default();
    config.provider.kind = mvp::config::ProviderKind::Deepseek;
    config.provider.model = "deepseek-chat".to_owned();

    let next_steps = build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

    assert!(
        next_steps
            .iter()
            .all(|step| !step.contains("Retry provider probe only after credentials are ready")),
        "skipped probe warnings should not look like real model catalog failures: {next_steps:#?}"
    );
    assert!(
        next_steps
            .iter()
            .all(|step| !step.contains("--skip-model-probe")),
        "skipped probe warnings should not advertise the skip-model-probe recovery branch: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_promotes_ask_and_chat_when_green() {
    let checks = vec![
        DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        },
        DoctorCheck {
            name: "provider transport".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "responses api".to_owned(),
        },
    ];
    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &mvp::config::LoongConfig::default(),
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
            next_steps.iter().any(|step| {
                step == "Get a first answer: loong ask --config '/tmp/loong.toml' --message 'Summarize this repository and suggest the best next step.'"
            }),
            "green doctor runs should hand the user into ask immediately: {next_steps:#?}"
        );
    assert!(
        next_steps
            .iter()
            .any(|step| { step == "Continue in chat: LOONG_CONFIG_PATH='/tmp/loong.toml' loong" }),
        "green doctor runs should still advertise chat as the follow-up path: {next_steps:#?}"
    );
    assert!(
        next_steps.iter().any(|step| {
            step == "Set your working preferences: loong personalize --config '/tmp/loong.toml'"
        }),
        "green doctor runs should surface personalization as the third healthy-path suggestion: {next_steps:#?}"
    );
    assert!(
        !next_steps
            .iter()
            .any(|step| { step == "Open a channel: loong channels --config '/tmp/loong.toml'" }),
        "green doctor runs should cap the healthy-path list before lower-priority channel setup suggestions: {next_steps:#?}"
    );
    assert!(
        !next_steps
            .iter()
            .any(|step| { step == "Open a channel: loong channels --config '/tmp/loong.toml'" }),
        "green doctor runs should keep lower-priority setup prompts behind personalization: {next_steps:#?}"
    );
}

#[test]
fn build_doctor_next_steps_prioritizes_personalization_when_channels_are_enabled() {
    let checks = vec![DoctorCheck {
        name: "provider credentials".to_owned(),
        level: DoctorCheckLevel::Pass,
        detail: "provider credentials are available".to_owned(),
    }];
    let mut config = mvp::config::LoongConfig::default();
    config.telegram.enabled = true;

    let next_steps = build_doctor_next_steps_with_path_env(
        &checks,
        Path::new("/tmp/loong.toml"),
        &config,
        false,
        Some(std::ffi::OsStr::new("")),
    );

    assert!(
        next_steps.iter().any(|step| {
            step == "Set your working preferences: loong personalize --config '/tmp/loong.toml'"
        }),
        "doctor should prioritize personalization ahead of lower-priority setup prompts when the healthy-path list is capped: {next_steps:#?}"
    );
}

#[test]
fn collect_runtime_plugins_doctor_checks_warns_when_runtime_is_disabled() {
    let root = runtime_plugin_temp_dir("runtime-plugins-disabled");
    let config = runtime_plugins_test_config(&root, false);

    let checks = collect_runtime_plugins_doctor_checks(&config);

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].name, "runtime plugins runtime");
    assert_eq!(checks[0].level, DoctorCheckLevel::Warn);
    assert!(checks[0].detail.contains("enabled=false"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn collect_runtime_plugins_doctor_checks_escape_runtime_values() {
    let root = runtime_plugin_temp_dir("runtime-plugins-escaped");
    let mut config = runtime_plugins_test_config(&root, true);
    let escaped_root = root.join("runtime\nplugins");

    config.runtime_plugins.roots = vec![escaped_root.display().to_string()];
    config.runtime_plugins.supported_adapter_families = vec!["web\nsearch".to_owned()];

    let checks = collect_runtime_plugins_doctor_checks(&config);
    let runtime_check = checks
        .iter()
        .find(|check| check.name == "runtime plugins runtime")
        .expect("runtime plugins runtime check should exist");
    let inventory_check = checks
        .iter()
        .find(|check| check.name == "runtime plugins inventory")
        .expect("runtime plugins inventory check should exist");

    assert!(
        runtime_check
            .detail
            .contains("supported_adapter_families=\"web\\nsearch\"")
    );
    assert!(runtime_check.detail.contains("roots=\""));
    assert!(runtime_check.detail.contains("\\nplugins\""));
    assert!(
        inventory_check
            .detail
            .contains("error=\"runtime plugin scan failed for ")
    );
    assert!(inventory_check.detail.contains("\\nplugins"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn collect_runtime_plugins_doctor_checks_warns_when_no_runtime_roots_are_scanned() {
    let root = runtime_plugin_temp_dir("runtime-plugins-zero-roots");
    let mut config = runtime_plugins_test_config(&root, true);
    config.runtime_plugins.roots = vec!["   ".to_owned()];

    let checks = collect_runtime_plugins_doctor_checks(&config);

    assert!(
        checks.iter().any(|check| {
            check.name == "runtime plugins runtime"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("enabled=true")
                && check.detail.contains("scanned_roots=0")
        }),
        "runtime plugins runtime should warn when no usable roots can be scanned: {checks:#?}"
    );
    assert!(
        checks.iter().any(|check| {
            check.name == "runtime plugins inventory"
                && check.level == DoctorCheckLevel::Fail
                && check.detail.contains("inventory_status=error")
        }),
        "runtime plugins inventory should fail when roots resolve to nothing: {checks:#?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn build_doctor_next_steps_guides_runtime_plugin_enablement_when_disabled() {
    let checks = vec![DoctorCheck {
            name: "runtime plugins runtime".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "enabled=false supported_bridges=- supported_adapter_families=- roots=/tmp/runtime-plugins scanned_roots=0".to_owned(),
        }];
    let config = mvp::config::LoongConfig::default();

    let next_steps = build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

    assert!(
            next_steps.iter().any(|step| {
                step == "Enable runtime plugins by setting [runtime_plugins].enabled = true, then re-run diagnostics: loong doctor --config '/tmp/loong.toml'"
            }),
            "doctor should surface an explicit runtime-plugin enablement step: {next_steps:#?}"
        );
    assert!(
        next_steps
            .iter()
            .all(|step| { !step.starts_with("Inspect runtime plugin inventory:") }),
        "disabled runtime plugins should not suggest inventory inspection before enablement: {next_steps:#?}"
    );
}
