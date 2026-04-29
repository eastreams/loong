#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::MutexGuard,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn write_file(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, content).expect("write fixture");
}

struct RuntimeSnapshotEnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl RuntimeSnapshotEnvGuard {
    fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = super::lock_daemon_test_environment();
        let mut saved = Vec::new();
        for (key, value) in pairs {
            saved.push(((*key).to_owned(), std::env::var_os(key)));
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for RuntimeSnapshotEnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(&key, value);
                },
                None => unsafe {
                    std::env::remove_var(&key);
                },
            }
        }
    }
}

struct RuntimeSnapshotCurrentDirGuard {
    saved: PathBuf,
}

impl RuntimeSnapshotCurrentDirGuard {
    fn set(path: &Path) -> Self {
        let saved = std::env::current_dir().expect("capture current directory");
        std::env::set_current_dir(path).expect("switch current directory");
        Self { saved }
    }
}

impl Drop for RuntimeSnapshotCurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.saved).expect("restore current directory");
    }
}

struct RuntimeSnapshotPolicyResetGuard {
    runtime_config: mvp::tools::runtime_config::ToolRuntimeConfig,
}

impl RuntimeSnapshotPolicyResetGuard {
    fn new(runtime_config: &mvp::tools::runtime_config::ToolRuntimeConfig) -> Self {
        Self {
            runtime_config: runtime_config.clone(),
        }
    }
}

impl Drop for RuntimeSnapshotPolicyResetGuard {
    fn drop(&mut self) {
        let _ = mvp::tools::execute_tool_core_with_config(
            kernel::ToolCoreRequest {
                tool_name: "external_skills.policy".to_owned(),
                payload: serde_json::json!({
                    "action": "reset",
                    "policy_update_approved": true,
                }),
            },
            &self.runtime_config,
        );
    }
}

fn write_runtime_snapshot_config(root: &Path) -> (PathBuf, mvp::config::LoongConfig) {
    fs::create_dir_all(root).expect("create fixture root");
    let workspace_root = root.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace fixture root");
    let mcp_command = std::env::current_exe()
        .expect("current executable path for MCP fixture")
        .display()
        .to_string();

    let mut config = mvp::config::LoongConfig::default();
    config.tools.file_root = Some(root.display().to_string());
    config.tools.shell_allow = vec!["git".to_owned(), "cargo".to_owned()];
    config.tools.browser.enabled = true;
    config.tools.browser_companion.enabled = true;
    config.tools.browser_companion.command = Some("browser-companion".to_owned());
    config.tools.browser_companion.expected_version = Some("1.2.3".to_owned());
    config.tools.web.enabled = true;
    config.tools.web.allowed_domains = vec!["docs.example.com".to_owned()];
    config.tools.web.blocked_domains = vec!["internal.example".to_owned()];
    config.external_skills.enabled = true;
    config.external_skills.require_download_approval = false;
    config.external_skills.auto_expose_installed = true;
    config.external_skills.allowed_domains = vec!["skills.sh".to_owned()];
    config.external_skills.install_root = Some(root.join("managed-skills").display().to_string());
    config.acp.enabled = true;
    config.acp.dispatch.enabled = true;
    config.acp.default_agent = Some("codex".to_owned());
    config.acp.allowed_agents = vec!["codex".to_owned(), "planner".to_owned()];
    config.mcp.servers.insert(
        "docs".to_owned(),
        mvp::mcp::McpServerConfig {
            transport: mvp::mcp::McpServerTransportConfig::Stdio {
                command: mcp_command,
                args: vec!["context7-mcp".to_owned()],
                env: std::collections::BTreeMap::from([(
                    "API_TOKEN".to_owned(),
                    "secret".to_owned(),
                )]),
                cwd: Some(workspace_root),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: Some(15_000),
            tool_timeout_ms: Some(120_000),
            enabled_tools: vec!["search".to_owned()],
            disabled_tools: vec!["write".to_owned()],
        },
    );
    config.providers.insert(
        "openai-main".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: false,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "gpt-4.1-mini".to_owned(),
                ..Default::default()
            },
        },
    );
    config.set_active_provider_profile(
        "deepseek-lab",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Deepseek,
                model: "deepseek-chat".to_owned(),
                api_key: Some(loong_contracts::SecretRef::Inline("demo-token".to_owned())),
                ..Default::default()
            },
        },
    );

    let config_path = root.join("loong.toml");
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    (config_path, config)
}

fn install_demo_skill(root: &Path, config: &mvp::config::LoongConfig, config_path: &Path) {
    write_file(
        root,
        "source/demo-skill/SKILL.md",
        "# Demo Skill\n\nInstalled for runtime snapshot coverage.\n",
    );

    let runtime_config =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, Some(config_path));
    mvp::tools::execute_tool_core_with_config(
        kernel::ToolCoreRequest {
            tool_name: "external_skills.install".to_owned(),
            payload: serde_json::json!({
                "path": "source/demo-skill"
            }),
        },
        &runtime_config,
    )
    .expect("install demo skill");
}

fn install_demo_runtime_plugin_package(root: &Path, config_path: &Path) {
    write_file(
        root,
        "runtime-plugins/search/loong.plugin.json",
        r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "demo-search-plugin",
  "provider_id": "demo-search",
  "connector_name": "demo-search-http",
  "endpoint": "https://example.com/search",
  "capabilities": ["InvokeConnector"],
  "summary": "Demo native extension search adapter",
  "tags": ["search", "demo"],
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_family": "governed_native_runtime_extension",
    "loong_extension_trust_lane": "governed_sidecar",
    "loong_extension_facets_json": "[\"tooling\",\"events\"]",
    "loong_extension_methods_json": "[\"extension/tool\",\"extension/event\"]",
    "loong_extension_events_json": "[\"session_start\",\"tool_result\"]",
    "loong_extension_host_actions_json": "[\"append_entry\",\"notify\"]"
  },
  "setup": {
    "mode": "metadata_only",
    "surface": "web_search",
    "required_env_vars": ["RUNTIME_PLUGIN_DEMO_KEY"],
    "required_config_keys": ["tools.web_search.default_provider"]
  },
  "slot_claims": [
    {
      "slot": "provider:web_search",
      "key": "demo",
      "mode": "shared"
    }
  ]
}"#,
    );

    let (path_string, mut reloaded) = mvp::config::load(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("reload config");
    reloaded.runtime_plugins.enabled = true;
    reloaded.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
    mvp::config::write(Some(&path_string.display().to_string()), &reloaded, true)
        .expect("rewrite config fixture with runtime plugin roots");
}

fn install_invalid_runtime_plugin_package(root: &Path, config_path: &Path) {
    write_file(
        root,
        "runtime-plugins/invalid-search/loong.plugin.json",
        r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "invalid-search-plugin",
  "provider_id": "invalid-search",
  "connector_name": "invalid-search-http",
  "endpoint": "https://example.com/search",
  "capabilities": ["InvokeConnector", "ObserveTelemetry"],
  "summary": "Malformed native extension declarations",
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_family": "governed_native_runtime_extension",
    "loong_extension_trust_lane": "governed_sidecar",
    "loong_extension_facets_json": "[\"tooling\",\"events\"]",
    "loong_extension_methods_json": "not-json",
    "loong_extension_events_json": "[\"session_start\"]",
    "loong_extension_host_actions_json": "[\"notify\"]"
  }
}"#,
    );

    let (path_string, mut reloaded) = mvp::config::load(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("reload config");
    reloaded.runtime_plugins.enabled = true;
    reloaded.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
    mvp::config::write(Some(&path_string.display().to_string()), &reloaded, true)
        .expect("rewrite config fixture with runtime plugin roots");
}

fn install_invalid_process_stdio_runtime_plugin_package(root: &Path, config_path: &Path) {
    write_file(
        root,
        "runtime-plugins/invalid-stdio/loong.plugin.json",
        r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "invalid-stdio-plugin",
  "provider_id": "invalid-stdio",
  "connector_name": "invalid-stdio",
  "capabilities": ["InvokeConnector"],
  "summary": "Malformed process stdio extension declarations",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "javascript-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "javascript",
    "command": "node",
    "args_json": "[\"index.js\"]",
    "process_timeout_ms": "15000",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_family": "governed_native_runtime_extension",
    "loong_extension_trust_lane": "governed_sidecar",
    "loong_extension_facets_json": "[\"events\",\"commands\",\"resources\"]",
    "loong_extension_methods_json": "not-json",
    "loong_extension_events_json": "[\"session_start\"]",
    "loong_extension_host_actions_json": "[]"
  }
}"#,
    );
    write_file(
        root,
        "runtime-plugins/invalid-stdio/index.js",
        "#!/usr/bin/env node\nprocess.stdin.resume();\n",
    );

    let (path_string, mut reloaded) = mvp::config::load(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("reload config");
    reloaded.runtime_plugins.enabled = true;
    reloaded.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
    mvp::config::write(Some(&path_string.display().to_string()), &reloaded, true)
        .expect("rewrite config fixture with runtime plugin roots");
}

fn install_host_hook_declared_runtime_plugin_package(root: &Path, config_path: &Path) {
    write_file(
        root,
        "runtime-plugins/host-hook-search/loong.plugin.json",
        r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "host-hook-search-plugin",
  "provider_id": "host-hook-search",
  "connector_name": "host-hook-search-http",
  "endpoint": "https://example.com/search",
  "capabilities": ["InvokeConnector"],
  "summary": "Declares reserved host hooks outside the trusted host lane",
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_family": "governed_native_runtime_extension",
    "loong_extension_trust_lane": "governed_sidecar",
    "loong_extension_facets_json": "[\"tooling\",\"events\"]",
    "loong_extension_methods_json": "[\"extension/tool\",\"extension/event\"]",
    "loong_extension_events_json": "[\"session_start\"]",
    "loong_extension_host_hooks_json": "[\"turn_start\",\"turn_end\"]",
    "loong_extension_host_actions_json": "[\"notify\"]"
  }
}"#,
    );

    let (path_string, mut reloaded) = mvp::config::load(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("reload config");
    reloaded.runtime_plugins.enabled = true;
    reloaded.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
    mvp::config::write(Some(&path_string.display().to_string()), &reloaded, true)
        .expect("rewrite config fixture with runtime plugin roots");
}

fn install_auto_discovered_duplicate_process_stdio_runtime_plugin_packages(
    root: &Path,
    home: &Path,
    config_path: &Path,
) {
    write_file(
        root,
        ".loong/extensions/search/loong.plugin.json",
        r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "shared-extension",
  "provider_id": "project-extension",
  "connector_name": "project-extension",
  "capabilities": ["InvokeConnector"],
  "summary": "Project-local extension",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "javascript-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "javascript",
    "command": "node",
    "args_json": "[\"index.js\"]",
    "process_timeout_ms": "5000"
  }
}"#,
    );
    write_file(
        root,
        ".loong/extensions/search/index.js",
        "#!/usr/bin/env node\nprocess.stdin.resume();\n",
    );
    write_file(
        home,
        ".loong/agent/extensions/search/loong.plugin.json",
        r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "shared-extension",
  "provider_id": "global-extension",
  "connector_name": "global-extension",
  "capabilities": ["InvokeConnector"],
  "summary": "Global extension",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "javascript-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "javascript",
    "command": "node",
    "args_json": "[\"index.js\"]",
    "process_timeout_ms": "5000"
  }
}"#,
    );
    write_file(
        home,
        ".loong/agent/extensions/search/index.js",
        "#!/usr/bin/env node\nprocess.stdin.resume();\n",
    );

    let (path_string, mut reloaded) = mvp::config::load(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("reload config");
    reloaded.runtime_plugins.enabled = true;
    reloaded.runtime_plugins.roots = vec!["   ".to_owned()];
    reloaded.runtime_plugins.supported_bridges = vec!["process_stdio".to_owned()];
    reloaded.runtime_plugins.allowed_process_commands = vec!["node".to_owned()];
    mvp::config::write(Some(&path_string.display().to_string()), &reloaded, true)
        .expect("rewrite config fixture with auto-discovered runtime plugin roots");
}

fn array_contains_string(array: &Value, needle: &str) -> bool {
    array.as_array().is_some_and(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .any(|value| value == needle)
    })
}

fn array_contains_object_field(array: &Value, field: &str, needle: &str) -> bool {
    array.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| value == needle)
        })
    })
}

fn array_object_with_string_field<'a>(
    array: &'a Value,
    field: &str,
    needle: &str,
) -> Option<&'a Value> {
    array.as_array()?.iter().find(|item| {
        item.get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| value == needle)
    })
}

#[test]
fn runtime_snapshot_json_payload_includes_provider_tool_and_external_skill_inventory() {
    let root = unique_temp_dir("loong-runtime-snapshot-json");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("DEEPSEEK_API_KEY", None),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
    ]);
    let (config_path, config) = write_runtime_snapshot_config(&root);
    install_demo_skill(&root, &config, &config_path);
    install_demo_runtime_plugin_package(&root, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");

    assert_eq!(
        payload["schema"]["version"],
        loong_daemon::RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION
    );
    assert_eq!(payload["provider"]["active_profile_id"], "deepseek-lab");
    assert!(array_contains_string(
        &payload["provider"]["saved_profile_ids"],
        "deepseek-lab"
    ));
    let active_profile = array_object_with_string_field(
        &payload["provider"]["profiles"],
        "profile_id",
        "deepseek-lab",
    )
    .expect("active provider profile should be present");
    assert_eq!(active_profile["credential_resolved"], true);
    assert!(payload["provider"]["transport_runtime"]["http_client_cache_entries"].is_number());
    assert!(payload["provider"]["transport_runtime"]["http_client_cache_hits"].is_number());
    assert!(payload["provider"]["transport_runtime"]["http_client_cache_misses"].is_number());
    assert!(payload["provider"]["transport_runtime"]["built_http_clients"].is_number());
    assert!(array_contains_string(
        &payload["tools"]["visible_tool_names"],
        "external_skills.list"
    ));
    assert_eq!(payload["external_skills"]["policy"]["enabled"], true);
    assert!(array_contains_object_field(
        &payload["external_skills"]["inventory"]["skills"],
        "skill_id",
        "demo-skill"
    ));
    assert!(
        payload["tools"]["capability_snapshot_sha256"]
            .as_str()
            .is_some_and(|value: &str| !value.is_empty()),
        "capability snapshot digest should be populated"
    );
    assert_eq!(payload["runtime_plugins"]["enabled"], true);
    assert_eq!(payload["runtime_plugins"]["discovered_plugin_count"], 1);
    assert_eq!(
        payload["runtime_plugins"]["setup_incomplete_plugin_count"],
        1
    );
    assert_eq!(
        payload["runtime_plugins"]["readiness_evaluation"],
        "default_bridge_support_matrix"
    );
    assert!(array_contains_object_field(
        &payload["runtime_plugins"]["plugins"],
        "plugin_id",
        "demo-search-plugin"
    ));
    let plugin = array_object_with_string_field(
        &payload["runtime_plugins"]["plugins"],
        "plugin_id",
        "demo-search-plugin",
    )
    .expect("runtime plugin should be present");
    assert_eq!(
        plugin["extension_contract"],
        serde_json::json!("process_stdio_json_line_v1")
    );
    assert_eq!(
        plugin["manifest_api_version"],
        serde_json::json!("v1alpha1")
    );
    assert_eq!(plugin["plugin_version"], serde_json::json!("1.0.0"));
    assert_eq!(
        plugin["summary"],
        serde_json::json!("Demo native extension search adapter")
    );
    assert_eq!(plugin["tags"], serde_json::json!(["search", "demo"]));
    assert_eq!(
        plugin["dialect"],
        serde_json::json!("loong_package_manifest")
    );
    assert_eq!(plugin["compatibility_mode"], serde_json::json!("native"));
    assert_eq!(plugin["compatibility_shim"], serde_json::Value::Null);
    assert_eq!(
        plugin["compatibility_shim_supported_dialects"],
        serde_json::json!([])
    );
    assert_eq!(
        plugin["compatibility_shim_mismatch_reasons"],
        serde_json::json!([])
    );
    assert_eq!(plugin["source_language"], serde_json::json!("manifest"));
    assert_eq!(
        plugin["entrypoint_hint"],
        serde_json::json!("https://example.com/search")
    );
    assert_eq!(
        plugin["extension_facets"],
        serde_json::json!(["tooling", "events"])
    );
    assert_eq!(
        plugin["extension_methods"],
        serde_json::json!(["extension/tool", "extension/event"])
    );
    assert_eq!(
        plugin["extension_events"],
        serde_json::json!(["session_start", "tool_result"])
    );
    assert_eq!(plugin["extension_host_hooks"], serde_json::json!([]));
    assert_eq!(
        plugin["extension_host_actions"],
        serde_json::json!(["append_entry", "notify"])
    );
    assert_eq!(plugin["extension_metadata_issues"], serde_json::json!([]));
    assert_eq!(plugin["diagnostic_codes"], serde_json::json!([]));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_artifact_payload_can_embed_live_plugin_inventory_truth() {
    let root = unique_temp_dir("loong-runtime-snapshot-artifact-inventory");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, _config) = write_runtime_snapshot_config(&root);
    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let metadata = loong_daemon::RuntimeSnapshotArtifactMetadata {
        created_at: "2026-04-27T00:00:00Z".to_owned(),
        label: Some("artifact".to_owned()),
        experiment_id: None,
        parent_snapshot_id: None,
    };

    let payload = loong_daemon::build_runtime_snapshot_artifact_json_payload_with_inventory(
        &snapshot,
        &metadata,
        Some(loong_daemon::plugins_cli::RuntimePluginInventoryReadModel {
            available: true,
            reason: None,
            error: None,
            roots_source: Some("configured".to_owned()),
            returned_results: Some(1),
            summary: None,
            native_extension_authoring_summary: None,
            shadowed_plugin_ids: Vec::new(),
            discovery_guidance: None,
            results: Vec::new(),
        }),
    )
    .expect("build runtime snapshot artifact");

    assert_eq!(
        payload["runtime_plugin_inventory"]["available"],
        serde_json::json!(true)
    );
    assert_eq!(
        payload["runtime_plugin_inventory"]["returned_results"],
        serde_json::json!(1)
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_surfaces_native_extension_authoring_guidance_for_invalid_process_stdio_package()
{
    let root = unique_temp_dir("loong-runtime-snapshot-invalid-stdio-authoring");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("DEEPSEEK_API_KEY", None),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
    ]);
    let (config_path, _config) = write_runtime_snapshot_config(&root);
    install_invalid_process_stdio_runtime_plugin_package(&root, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");
    let plugin = array_object_with_string_field(
        &payload["runtime_plugins"]["plugins"],
        "plugin_id",
        "invalid-stdio-plugin",
    )
    .expect("runtime snapshot should include invalid process stdio plugin");

    let guidance = &plugin["authoring_guidance"];
    assert_eq!(
        guidance["reference_example_path"],
        serde_json::json!("examples/plugins-process/native-extension-javascript")
    );
    assert_eq!(guidance["source_language_arg"], serde_json::json!("js"));
    assert_eq!(guidance["smoke_allow_command"], serde_json::json!("node"));
    let actions = guidance["author_remediation_actions"]
        .as_array()
        .expect("author remediation actions should be an array");
    assert!(actions.iter().any(|action| {
        action["kind"] == serde_json::json!("repair_extension_metadata")
            && action["role"] == serde_json::json!("author")
            && action["execution_kind"] == serde_json::json!("manual_edit")
    }));
    assert!(actions.iter().any(|action| {
        action["kind"] == serde_json::json!("rerun_smoke_test")
            && action["role"] == serde_json::json!("verification")
            && action["execution_kind"] == serde_json::json!("governed_smoke_probe")
            && action["agent_runnable"] == serde_json::json!(true)
            && action["requires_allow_command"] == serde_json::json!(true)
            && action["allow_command"] == serde_json::json!("node")
    }));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_text_surfaces_native_extension_authoring_guidance_for_invalid_process_stdio_package()
 {
    let root = unique_temp_dir("loong-runtime-snapshot-invalid-stdio-authoring-text");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("DEEPSEEK_API_KEY", None),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
    ]);
    let (config_path, _config) = write_runtime_snapshot_config(&root);
    install_invalid_process_stdio_runtime_plugin_package(&root, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let rendered = render_runtime_snapshot_text(&snapshot);

    assert!(rendered.contains("invalid-stdio-plugin"));
    assert!(rendered.contains("authoring_summary guided_plugins=1"));
    assert!(rendered.contains("plugins_with_metadata_issues=1"));
    assert!(
        rendered.contains("reference_example=examples/plugins-process/native-extension-javascript")
    );
    assert!(rendered.contains("smoke_allow_command=node"));
    assert!(rendered.contains("action_roles=author,verification"));
    for action_kind in [
        "repair_extension_metadata",
        "rerun_doctor",
        "rerun_inventory",
        "rerun_smoke_test",
    ] {
        assert!(
            rendered.contains(action_kind),
            "runtime snapshot text should keep remediation action kind {action_kind}: {rendered}"
        );
    }
    assert!(rendered.contains("runnable_action_kinds="));
    assert!(rendered.contains("allow_command_action_kinds=rerun_smoke_test"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_prefers_project_local_loong_extensions_over_global_duplicates() {
    let root = unique_temp_dir("loong-runtime-snapshot-auto-discovery-precedence");
    let home = unique_temp_dir("loong-runtime-snapshot-auto-discovery-home");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("DEEPSEEK_API_KEY", None),
        ("HOME", Some(home.to_string_lossy().as_ref())),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
    ]);
    let (config_path, _config) = write_runtime_snapshot_config(&root);
    let _cwd = RuntimeSnapshotCurrentDirGuard::set(&root);
    install_auto_discovered_duplicate_process_stdio_runtime_plugin_packages(
        &root,
        &home,
        &config_path,
    );

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");
    let plugin = array_object_with_string_field(
        &payload["runtime_plugins"]["plugins"],
        "plugin_id",
        "shared-extension",
    )
    .expect("runtime snapshot should include the effective shared extension");

    assert_eq!(
        payload["runtime_plugins"]["roots_source"],
        serde_json::json!("auto_discovered")
    );
    assert_eq!(
        payload["runtime_plugins"]["shadowed_plugin_ids"],
        serde_json::json!(["shared-extension"])
    );
    assert_eq!(
        payload["runtime_plugins"]["discovery_guidance"]["precedence_rule"],
        serde_json::json!("project_local_over_global")
    );
    assert_eq!(
        payload["runtime_plugins"]["discovery_guidance"]["recommended_action"],
        serde_json::json!("review_global_duplicate")
    );
    assert_eq!(
        payload["runtime_plugins"]["discovery_guidance"]["discovery_actions"][0]["kind"],
        serde_json::json!("inspect_effective_package")
    );
    assert_eq!(
        payload["runtime_plugins"]["discovery_guidance"]["discovery_actions"][0]["role"],
        serde_json::json!("operator")
    );
    assert_eq!(
        payload["runtime_plugins"]["discovery_guidance"]["discovery_actions"][0]["execution_kind"],
        serde_json::json!("read_only_cli")
    );
    assert!(
        payload["runtime_plugins"]["discovery_guidance"]["discovery_actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| {
                action["kind"] == serde_json::json!("compare_shadowed_manifests")
                    && action["command"]
                        .as_str()
                        .is_some_and(|command| command.contains("git diff --no-index"))
            }))
    );
    assert!(
        payload["runtime_plugins"]["discovery_guidance"]["discovery_actions"][0]["command"]
            .as_str()
            .is_some_and(|command| command.contains("loong plugins doctor --root"))
    );
    assert_eq!(
        payload["runtime_plugins"]["discovery_guidance"]["shadowed_conflicts"][0]["plugin_id"],
        serde_json::json!("shared-extension")
    );
    assert!(
        payload["runtime_plugins"]["discovery_guidance"]["shadowed_conflicts"][0]
            ["effective_source_path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".loong/extensions/search/loong.plugin.json"))
    );
    assert!(
        payload["runtime_plugins"]["discovery_guidance"]["shadowed_conflicts"][0]
            ["shadowed_source_paths"][0]
            .as_str()
            .is_some_and(|path| path.ends_with(".loong/agent/extensions/search/loong.plugin.json"))
    );
    assert!(
        plugin["source_path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".loong/extensions/search/loong.plugin.json")),
        "runtime snapshot should keep the project-local descriptor: {plugin:#?}"
    );

    let rendered = render_runtime_snapshot_text(&snapshot);
    assert!(rendered.contains("roots_source=auto_discovered"));
    assert!(rendered.contains("shadowed_plugins=1"));
    assert!(rendered.contains("shadowed_plugin_ids=shared-extension"));
    assert!(rendered.contains("precedence_rule=project_local_over_global"));
    assert!(rendered.contains("recommended_action=review_global_duplicate"));
    assert!(rendered.contains("discovery_action_kinds=inspect_effective_package"));
    assert!(rendered.contains("compare_shadowed_manifests"));
    assert!(rendered.contains("effective_source_path="));
    assert!(rendered.contains("shadowed_source_paths="));

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&home).ok();
}

#[tokio::test]
async fn runtime_snapshot_and_inventory_share_invalid_extension_declaration_truth() {
    let root = unique_temp_dir("loong-runtime-snapshot-invalid-extension-declarations");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("DEEPSEEK_API_KEY", None),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
    ]);
    let (config_path, _config) = write_runtime_snapshot_config(&root);
    install_invalid_runtime_plugin_package(&root, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");
    let runtime_plugin = array_object_with_string_field(
        &payload["runtime_plugins"]["plugins"],
        "plugin_id",
        "invalid-search-plugin",
    )
    .expect("runtime snapshot should include invalid plugin");

    let inventory_execution = loong_daemon::plugins_cli::execute_plugins_command(
        loong_daemon::plugins_cli::PluginsCommandOptions {
            json: false,
            command: loong_daemon::plugins_cli::PluginsCommands::Inventory(
                loong_daemon::plugins_cli::PluginInventoryCommand {
                    source: loong_daemon::plugins_cli::PluginScanSourceArgs {
                        roots: vec![root.join("runtime-plugins").display().to_string()],
                        query: String::new(),
                        limit: None,
                        bridge_support: None,
                        bridge_profile: None,
                        bridge_support_delta: None,
                        bridge_support_sha256: None,
                        bridge_support_delta_sha256: None,
                    },
                    include_ready: true,
                    include_blocked: true,
                    include_deferred: true,
                    include_examples: false,
                },
            ),
        },
    )
    .await
    .expect("inventory should decode invalid extension declarations");

    let loong_daemon::plugins_cli::PluginsCommandExecution::Inventory(inventory_execution) =
        inventory_execution
    else {
        panic!("expected inventory execution");
    };

    let inventory_plugin = &inventory_execution.results[0];
    assert_eq!(
        runtime_plugin["extension_contract"],
        serde_json::json!("process_stdio_json_line_v1")
    );
    assert_eq!(
        runtime_plugin["capabilities"],
        serde_json::json!(["invoke_connector", "observe_telemetry"])
    );
    assert_eq!(
        runtime_plugin["extension_family"],
        serde_json::json!("governed_native_runtime_extension")
    );
    assert_eq!(
        runtime_plugin["extension_trust_lane"],
        serde_json::json!("governed_sidecar")
    );
    assert_eq!(
        serde_json::to_value(&inventory_plugin.capabilities)
            .expect("serialize inventory plugin capabilities"),
        serde_json::json!(["invoke_connector", "observe_telemetry"])
    );
    assert_eq!(
        inventory_plugin.extension_family.as_deref(),
        Some("governed_native_runtime_extension")
    );
    assert_eq!(
        inventory_plugin.extension_trust_lane.as_deref(),
        Some("governed_sidecar")
    );
    assert_eq!(
        runtime_plugin["extension_events"],
        serde_json::json!(["session_start"])
    );
    assert_eq!(
        runtime_plugin["extension_host_hooks"],
        serde_json::json!([])
    );
    assert_eq!(
        runtime_plugin["extension_host_actions"],
        serde_json::json!(["notify"])
    );
    assert_eq!(
        inventory_plugin.extension_contract.as_deref(),
        Some("process_stdio_json_line_v1")
    );
    assert_eq!(
        inventory_plugin.extension_events,
        vec!["session_start".to_owned()]
    );
    assert!(inventory_plugin.extension_host_hooks.is_empty());
    assert_eq!(
        inventory_plugin.extension_host_actions,
        vec!["notify".to_owned()]
    );
    assert_eq!(runtime_plugin["extension_methods"], serde_json::json!([]));
    assert!(inventory_plugin.extension_methods.is_empty());
    assert_eq!(
        runtime_plugin["extension_metadata_issues"],
        serde_json::to_value(&inventory_plugin.extension_metadata_issues)
            .expect("serialize inventory metadata issues")
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn runtime_snapshot_and_inventory_share_reserved_host_hook_declaration_truth() {
    let root = unique_temp_dir("loong-runtime-snapshot-host-hook-declarations");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("DEEPSEEK_API_KEY", None),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
    ]);
    let (config_path, _config) = write_runtime_snapshot_config(&root);
    install_host_hook_declared_runtime_plugin_package(&root, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");
    let runtime_plugin = array_object_with_string_field(
        &payload["runtime_plugins"]["plugins"],
        "plugin_id",
        "host-hook-search-plugin",
    )
    .expect("runtime snapshot should include host-hook plugin");

    let inventory_execution = loong_daemon::plugins_cli::execute_plugins_command(
        loong_daemon::plugins_cli::PluginsCommandOptions {
            json: false,
            command: loong_daemon::plugins_cli::PluginsCommands::Inventory(
                loong_daemon::plugins_cli::PluginInventoryCommand {
                    source: loong_daemon::plugins_cli::PluginScanSourceArgs {
                        roots: vec![root.join("runtime-plugins").display().to_string()],
                        query: String::new(),
                        limit: None,
                        bridge_support: None,
                        bridge_profile: None,
                        bridge_support_delta: None,
                        bridge_support_sha256: None,
                        bridge_support_delta_sha256: None,
                    },
                    include_ready: true,
                    include_blocked: true,
                    include_deferred: true,
                    include_examples: false,
                },
            ),
        },
    )
    .await
    .expect("inventory should decode host hook declarations");

    let loong_daemon::plugins_cli::PluginsCommandExecution::Inventory(inventory_execution) =
        inventory_execution
    else {
        panic!("expected inventory execution");
    };

    let inventory_plugin = &inventory_execution.results[0];
    assert_eq!(
        runtime_plugin["extension_host_hooks"],
        serde_json::json!(["turn_start", "turn_end"])
    );
    assert_eq!(
        inventory_plugin.extension_host_hooks,
        vec!["turn_start".to_owned(), "turn_end".to_owned()]
    );
    assert!(
        inventory_plugin
            .extension_metadata_issues
            .iter()
            .any(|issue| issue.contains("loong_extension_host_hooks_json")),
        "expected reserved host-hook declaration issue, got {:?}",
        inventory_plugin.extension_metadata_issues
    );
    assert_eq!(
        runtime_plugin["extension_metadata_issues"],
        serde_json::to_value(&inventory_plugin.extension_metadata_issues)
            .expect("serialize inventory metadata issues")
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_json_payload_marks_x_api_key_profiles_as_credential_resolved() {
    let root = unique_temp_dir("loong-runtime-snapshot-x-api-key");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        (
            "RUNTIME_SNAPSHOT_ANTHROPIC_KEY",
            Some("anthropic-demo-token"),
        ),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, mut config) = write_runtime_snapshot_config(&root);
    config.providers.insert(
        "anthropic-lab".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: false,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Anthropic,
                model: "claude-3-7-sonnet-latest".to_owned(),
                api_key: Some(loong_contracts::SecretRef::Inline(
                    "${RUNTIME_SNAPSHOT_ANTHROPIC_KEY}".to_owned(),
                )),
                ..Default::default()
            },
        },
    );
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("rewrite config fixture");

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");

    let anthropic_profile = array_object_with_string_field(
        &payload["provider"]["profiles"],
        "profile_id",
        "anthropic-lab",
    )
    .expect("anthropic provider profile should be present");
    assert_eq!(anthropic_profile["credential_resolved"], true);
    assert_eq!(
        anthropic_profile["descriptor"]["schema"]["version"],
        serde_json::json!(mvp::config::PROVIDER_DESCRIPTOR_SCHEMA_VERSION)
    );
    assert_eq!(
        anthropic_profile["descriptor"]["auth"]["scheme"],
        serde_json::json!("x_api_key")
    );
    assert_eq!(
        anthropic_profile["descriptor"]["auth"]["requires_explicit_configuration"],
        serde_json::json!(true)
    );
    assert_eq!(
        anthropic_profile["descriptor"]["feature"]["family"],
        serde_json::json!("anthropic")
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_json_payload_preserves_auth_optional_provider_descriptor_contract() {
    let root = unique_temp_dir("loong-runtime-snapshot-auth-optional");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, mut config) = write_runtime_snapshot_config(&root);
    config.providers.insert(
        "ollama-local".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: false,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Ollama,
                model: "qwen2.5-coder".to_owned(),
                ..Default::default()
            },
        },
    );
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("rewrite config fixture");

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");

    let ollama_profile = array_object_with_string_field(
        &payload["provider"]["profiles"],
        "profile_id",
        "ollama-local",
    )
    .expect("ollama provider profile should be present");
    assert_eq!(ollama_profile["credential_resolved"], false);
    assert_eq!(
        ollama_profile["descriptor"]["auth"]["scheme"],
        serde_json::json!("bearer")
    );
    assert_eq!(
        ollama_profile["descriptor"]["auth"]["auth_optional"],
        serde_json::json!(true)
    );
    assert_eq!(
        ollama_profile["descriptor"]["auth"]["requires_explicit_configuration"],
        serde_json::json!(false)
    );
    assert_eq!(
        ollama_profile["descriptor"]["auth"]["hint_env_names"],
        serde_json::json!([])
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_json_payload_reflects_effective_external_skills_policy_override() {
    let root = unique_temp_dir("loong-runtime-snapshot-policy-override");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, config) = write_runtime_snapshot_config(&root);
    install_demo_skill(&root, &config, &config_path);

    let enabled_snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect enabled runtime snapshot");
    let enabled_payload = build_runtime_snapshot_cli_json_payload(&enabled_snapshot)
        .expect("build enabled runtime snapshot payload");
    let enabled_digest = enabled_payload["tools"]["capability_snapshot_sha256"].clone();
    assert!(array_contains_string(
        &enabled_payload["tools"]["visible_tool_names"],
        "external_skills.list"
    ));

    let runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &config,
        Some(config_path.as_path()),
    );
    let _policy_reset = RuntimeSnapshotPolicyResetGuard::new(&runtime_config);
    mvp::tools::execute_tool_core_with_config(
        kernel::ToolCoreRequest {
            tool_name: "external_skills.policy".to_owned(),
            payload: serde_json::json!({
                "action": "set",
                "policy_update_approved": true,
                "enabled": false,
                "require_download_approval": true,
                "allowed_domains": ["override.example"],
                "blocked_domains": ["blocked.example"],
            }),
        },
        &runtime_config,
    )
    .expect("override runtime external skills policy");

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");

    assert!(!snapshot.tool_runtime.external_skills.enabled);
    assert!(
        snapshot
            .tool_runtime
            .external_skills
            .require_download_approval
    );
    assert!(
        snapshot
            .tool_runtime
            .external_skills
            .allowed_domains
            .contains("override.example")
    );
    assert!(
        snapshot
            .tool_runtime
            .external_skills
            .blocked_domains
            .contains("blocked.example")
    );
    assert_eq!(payload["external_skills"]["policy"]["enabled"], false);
    assert_eq!(
        payload["external_skills"]["policy"]["require_download_approval"],
        true
    );
    assert!(array_contains_string(
        &payload["external_skills"]["policy"]["allowed_domains"],
        "override.example"
    ));
    assert!(array_contains_string(
        &payload["external_skills"]["policy"]["blocked_domains"],
        "blocked.example"
    ));
    assert_eq!(payload["external_skills"]["override_active"], true);
    assert_eq!(payload["external_skills"]["inventory_status"], "disabled");
    assert_eq!(payload["external_skills"]["resolved_skill_count"], 0);
    assert!(!array_contains_string(
        &payload["tools"]["visible_tool_names"],
        "external_skills.list"
    ));
    assert!(array_contains_string(
        &payload["tools"]["visible_tool_names"],
        "external_skills.policy"
    ));
    assert_ne!(
        payload["tools"]["capability_snapshot_sha256"],
        enabled_digest
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_text_highlights_experiment_relevant_sections() {
    let root = unique_temp_dir("loong-runtime-snapshot-text");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONG_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, config) = write_runtime_snapshot_config(&root);
    install_demo_skill(&root, &config, &config_path);
    install_demo_runtime_plugin_package(&root, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let rendered = render_runtime_snapshot_text(&snapshot);

    assert!(
        rendered
            .lines()
            .any(|line| line.starts_with("LOONG") || line.contains(" loong ")),
        "runtime snapshot text should now use the shared ratatui operator shell header: {rendered}"
    );
    assert!(rendered.contains("runtime snapshot"));
    assert!(rendered.contains("provider active_profile=deepseek-lab"));
    assert!(rendered.contains("provider transport cache_entries="));
    assert!(rendered.contains("context_engine selected="));
    assert!(rendered.contains("memory selected="));
    assert!(rendered.contains("acp enabled=true"));
    assert!(rendered.contains("acp mcp_servers=1"));
    assert!(rendered.contains("runtime_backed_enabled=-"));
    assert!(rendered.contains("plugin_backed_enabled=-"));
    assert!(rendered.contains("outbound_only_enabled=-"));
    assert!(
        rendered.contains(
            "surfaces=28 runtime_backed=7 config_backed=15 plugin_backed=3 catalog_only=3"
        )
    );
    assert!(rendered.contains("acp_mcp docs status=pending"));
    assert!(rendered.contains("tool_runtime web_access ordinary_network_enabled="));
    assert!(rendered.contains("query_search_default_provider=duckduckgo"));
    assert!(rendered.contains("query_search_credential_ready=true"));
    assert!(rendered.contains("tools visible_count="));
    assert!(rendered.contains("runtime_plugins inventory_status=ok enabled=true"));
    assert!(rendered.contains("readiness_evaluation=default_bridge_support_matrix"));
    assert!(rendered.contains("demo-search-plugin"));
    assert!(rendered.contains("source_path="));
    assert!(rendered.contains("package_root="));
    assert!(rendered.contains("setup_mode=metadata_only"));
    assert!(rendered.contains("setup_surface=web_search"));
    assert!(rendered.contains("missing_env_vars=RUNTIME_PLUGIN_DEMO_KEY"));
    assert!(rendered.contains("manifest_api_version=v1alpha1"));
    assert!(rendered.contains("plugin_version=1.0.0"));
    assert!(rendered.contains("summary=\"Demo native extension search adapter\""));
    assert!(rendered.contains("tags=search,demo"));
    assert!(rendered.contains("dialect=loong_package_manifest"));
    assert!(rendered.contains("compatibility_mode=native"));
    assert!(rendered.contains("compatibility_shim=-"));
    assert!(rendered.contains("compatibility_shim_supported_dialects=-"));
    assert!(rendered.contains("compatibility_shim_mismatch_reasons=-"));
    assert!(rendered.contains("source_language=manifest"));
    assert!(rendered.contains("entrypoint_hint=https://example.com/search"));
    assert!(rendered.contains("extension_contract=process_stdio_json_line_v1"));
    assert!(rendered.contains("extension_facets=tooling,events"));
    assert!(rendered.contains("extension_methods=extension/tool,extension/event"));
    assert!(rendered.contains("extension_events=session_start,tool_result"));
    assert!(rendered.contains("extension_host_hooks=-"));
    assert!(rendered.contains("extension_host_actions=append_entry,notify"));
    assert!(rendered.contains("external_skills inventory_status=ok override_active=false"));
    assert!(rendered.contains("demo-skill"));

    fs::remove_dir_all(&root).ok();
}
