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

fn write_runtime_snapshot_config(root: &Path) -> (PathBuf, mvp::config::LoongClawConfig) {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongClawConfig::default();
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
    config.active_provider = Some("deepseek-lab".to_owned());
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
    config.providers.insert(
        "deepseek-lab".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Deepseek,
                model: "deepseek-chat".to_owned(),
                api_key: Some("${RUNTIME_SNAPSHOT_DEEPSEEK_KEY}".to_owned()),
                ..Default::default()
            },
        },
    );

    let config_path = root.join("loongclaw.toml");
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    (config_path, config)
}

fn install_demo_skill(root: &Path, config: &mvp::config::LoongClawConfig, config_path: &Path) {
    write_file(
        root,
        "source/demo-skill/SKILL.md",
        "# Demo Skill\n\nInstalled for runtime snapshot coverage.\n",
    );

    let runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(config_path),
    );
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

#[test]
fn runtime_snapshot_json_payload_includes_provider_tool_and_external_skill_inventory() {
    let root = unique_temp_dir("loongclaw-runtime-snapshot-json");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, config) = write_runtime_snapshot_config(&root);
    install_demo_skill(&root, &config, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload = build_runtime_snapshot_cli_json_payload(&snapshot);

    assert_eq!(payload["schema"]["version"], 1);
    assert_eq!(payload["provider"]["active_profile_id"], "deepseek-lab");
    assert!(array_contains_string(
        &payload["provider"]["saved_profile_ids"],
        "deepseek-lab"
    ));
    assert_eq!(
        payload["provider"]["profiles"][0]["credential_resolved"],
        true
    );
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

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_json_payload_reflects_effective_external_skills_policy_override() {
    let root = unique_temp_dir("loongclaw-runtime-snapshot-policy-override");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, config) = write_runtime_snapshot_config(&root);
    install_demo_skill(&root, &config, &config_path);

    let runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
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
    let payload = build_runtime_snapshot_cli_json_payload(&snapshot);

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

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_snapshot_text_highlights_experiment_relevant_sections() {
    let root = unique_temp_dir("loongclaw-runtime-snapshot-text");
    let _env = RuntimeSnapshotEnvGuard::set(&[
        ("RUNTIME_SNAPSHOT_DEEPSEEK_KEY", Some("demo-token")),
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
    ]);
    let (config_path, config) = write_runtime_snapshot_config(&root);
    install_demo_skill(&root, &config, &config_path);

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let rendered = render_runtime_snapshot_text(&snapshot);

    assert!(rendered.contains("provider active_profile=deepseek-lab"));
    assert!(rendered.contains("context_engine selected="));
    assert!(rendered.contains("memory selected="));
    assert!(rendered.contains("acp enabled=true"));
    assert!(rendered.contains("tools visible_count="));
    assert!(rendered.contains("external_skills inventory_status=ok override_active=false"));
    assert!(rendered.contains("demo-skill"));

    fs::remove_dir_all(&root).ok();
}
