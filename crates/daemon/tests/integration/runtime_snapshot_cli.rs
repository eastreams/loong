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
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search"
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
    assert!(payload["provider"]["transport_runtime"]["failover_total_events"].is_number());
    assert!(payload["provider"]["transport_runtime"]["failover_continued_events"].is_number());
    assert!(payload["provider"]["transport_runtime"]["failover_exhausted_events"].is_number());
    assert!(payload["provider"]["transport_runtime"]["failover_by_reason"].is_object());
    assert!(payload["provider"]["transport_runtime"]["failover_by_stage"].is_object());
    assert!(payload["provider"]["transport_runtime"]["failover_by_provider"].is_object());
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
    assert!(rendered.contains("hygiene_strategy=turn_floor_only"));
    assert!(rendered.contains("diagnostics_surface=turn_checkpoint"));
    #[cfg(feature = "memory-sqlite")]
    let expected_compaction_hygiene_line =
        "context_engine compaction_hygiene evidence_status=no_evidence";
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_compaction_hygiene_line = "context_engine compaction_hygiene evidence_status=idle";
    assert!(rendered.contains(expected_compaction_hygiene_line));
    assert!(rendered.contains("posture=idle"));
    #[cfg(feature = "memory-sqlite")]
    let expected_diagnostics_coverage_line = "diagnostics_coverage=0/1 (0.0%)";
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_diagnostics_coverage_line = "diagnostics_coverage=0/0 (-)";
    assert!(rendered.contains(expected_diagnostics_coverage_line));
    assert!(rendered.contains("failed_open_rate=0/0 (-)"));
    assert!(rendered.contains("context_engine compaction_pressure demoted_recent_avg=-"));
    #[cfg(feature = "memory-sqlite")]
    let expected_compaction_trend_line =
        "context_engine compaction_trend scope=recent_sessions_fallback";
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_compaction_trend_line = "context_engine compaction_trend scope=idle";
    assert!(rendered.contains(expected_compaction_trend_line));
    assert!(rendered.contains("sample_order=updated_at_desc"));
    #[cfg(feature = "memory-sqlite")]
    let expected_continuity_line = "continuity=scope_limited";
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_continuity_line = "continuity=idle";
    #[cfg(feature = "memory-sqlite")]
    let expected_repairability_line =
        "context_engine compaction_repairability repairability=scope_limited";
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_repairability_line = "context_engine compaction_repairability repairability=idle";
    #[cfg(feature = "memory-sqlite")]
    let expected_recovery_posture_line = "recovery_posture=scope_limited";
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_recovery_posture_line = "recovery_posture=idle";
    assert!(rendered.contains(expected_continuity_line));
    assert!(rendered.contains(expected_repairability_line));
    assert!(rendered.contains(expected_recovery_posture_line));
    assert!(rendered.contains("reliability=insufficient_history"));
    assert!(rendered.contains("memory selected="));
    assert!(rendered.contains("acp enabled=true"));
    assert!(rendered.contains("acp mcp_servers=1"));
    assert!(rendered.contains("runtime_backed_enabled=-"));
    assert!(rendered.contains("plugin_backed_enabled=-"));
    assert!(rendered.contains("outbound_only_enabled=-"));
    assert!(
        rendered.contains(
            "surfaces=29 runtime_backed=8 config_backed=15 plugin_backed=3 catalog_only=3"
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
    assert!(rendered.contains("external_skills inventory_status=ok override_active=false"));
    assert!(rendered.contains("demo-skill"));

    fs::remove_dir_all(&root).ok();
}
