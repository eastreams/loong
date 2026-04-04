#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::Value;
use std::{
    collections::BTreeMap,
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

struct RuntimeRestoreEnvGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl RuntimeRestoreEnvGuard {
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

impl Drop for RuntimeRestoreEnvGuard {
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

fn write_runtime_restore_config(root: &Path) -> (PathBuf, mvp::config::LoongClawConfig) {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongClawConfig::default();
    config.audit.path = root
        .join("audit")
        .join("events.jsonl")
        .display()
        .to_string();
    config.tools.file_root = Some(root.display().to_string());
    config.tools.shell_allow = vec!["git".to_owned(), "cargo".to_owned()];
    config.tools.shell_deny = vec!["rm".to_owned()];
    config.tools.browser.enabled = true;
    config.tools.browser.max_sessions = 4;
    config.tools.browser.max_links = 32;
    config.tools.browser.max_text_chars = 4096;
    config.tools.browser_companion.enabled = true;
    config.tools.browser_companion.command = Some("browser-companion".to_owned());
    config.tools.browser_companion.expected_version = Some("1.2.3".to_owned());
    config.tools.web.enabled = true;
    config.tools.web.allowed_domains = vec!["docs.example.com".to_owned()];
    config.tools.web.blocked_domains = vec!["internal.example".to_owned()];

    config.conversation.context_engine = Some("default".to_owned());
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(6);
    config.conversation.compact_trigger_estimated_tokens = Some(900);
    config.conversation.compact_fail_open = false;

    config.memory.profile = mvp::config::MemoryProfile::WindowPlusSummary;
    config.memory.fail_open = false;
    config.memory.ingest_mode = mvp::config::MemoryIngestMode::AsyncBackground;
    config.memory.profile_note = Some("restore-target".to_owned());

    config.external_skills.enabled = true;
    config.external_skills.require_download_approval = false;
    config.external_skills.auto_expose_installed = true;
    config.external_skills.allowed_domains = vec!["skills.sh".to_owned()];
    config.external_skills.install_root = Some(root.join("managed-skills").display().to_string());

    config.acp.enabled = true;
    config.acp.dispatch.enabled = true;
    config.acp.default_agent = Some("planner".to_owned());
    config.acp.allowed_agents = vec!["planner".to_owned(), "codex".to_owned()];
    config.acp.dispatch.allowed_channels = vec!["feishu".to_owned()];
    config.acp.dispatch.working_directory = Some(root.join("workspace").display().to_string());

    config.providers.insert(
        "openai-main".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: false,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "gpt-4.1-mini".to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "${OPENAI_API_KEY}".to_owned(),
                )),
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
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "${RUNTIME_RESTORE_DEEPSEEK_KEY}".to_owned(),
                )),
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
        "# Demo Skill\n\nInstalled for runtime restore coverage.\n",
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

fn write_snapshot_artifact(
    root: &Path,
    config_path: &Path,
) -> (PathBuf, loongclaw_daemon::RuntimeSnapshotCliState, Value) {
    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let metadata = loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
        created_at: "2026-03-16T11:30:00Z".to_owned(),
        label: Some("baseline".to_owned()),
        experiment_id: Some("exp-runtime-restore".to_owned()),
        parent_snapshot_id: Some("snapshot-parent".to_owned()),
    };
    let payload =
        loongclaw_daemon::build_runtime_snapshot_artifact_json_payload(&snapshot, &metadata)
            .expect("build runtime snapshot artifact");
    let artifact_path = root.join("artifacts/runtime-snapshot.json");
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent).expect("create artifact directory");
    }
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&payload).expect("encode snapshot artifact"),
    )
    .expect("write snapshot artifact");
    (artifact_path, snapshot, payload)
}

fn write_snapshot_artifact_payload(root: &Path, relative: &str, payload: &Value) -> PathBuf {
    let artifact_path = root.join(relative);
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent).expect("create artifact parent");
    }
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(payload).expect("encode artifact payload"),
    )
    .expect("write artifact payload");
    artifact_path
}

fn mutate_runtime_restore_config(config_path: &Path, root: &Path) {
    let (_, mut config) = mvp::config::load(Some(
        config_path
            .to_str()
            .expect("config path should be valid utf-8"),
    ))
    .expect("reload fixture config");

    config.tools.shell_allow = vec!["git".to_owned()];
    config.tools.shell_deny.clear();
    config.tools.browser.enabled = false;
    config.tools.browser_companion.enabled = false;
    config.tools.web.allowed_domains.clear();
    config.tools.web.blocked_domains.clear();

    config.conversation.compact_min_messages = Some(2);
    config.conversation.compact_trigger_estimated_tokens = Some(128);
    config.conversation.compact_fail_open = true;

    config.memory.profile = mvp::config::MemoryProfile::WindowOnly;
    config.memory.fail_open = true;
    config.memory.ingest_mode = mvp::config::MemoryIngestMode::SyncMinimal;
    config.memory.profile_note = Some("mutated".to_owned());

    config.external_skills.enabled = false;
    config.external_skills.require_download_approval = true;
    config.external_skills.auto_expose_installed = false;
    config.external_skills.allowed_domains.clear();

    config.acp.enabled = false;
    config.acp.dispatch.enabled = false;
    config.acp.default_agent = Some("codex".to_owned());
    config.acp.allowed_agents = vec!["codex".to_owned()];
    config.acp.dispatch.allowed_channels.clear();
    config.acp.dispatch.working_directory =
        Some(root.join("other-workspace").display().to_string());

    config.set_active_provider_profile(
        "openai-main",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "gpt-4.1".to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "${OPENAI_API_KEY}".to_owned(),
                )),
                ..Default::default()
            },
        },
    );
    config.last_provider = Some("deepseek-lab".to_owned());

    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write mutated config");

    let managed_root = root.join("managed-skills");
    if managed_root.exists() {
        fs::remove_dir_all(&managed_root).expect("remove managed skills root");
    }
}

#[test]
fn runtime_snapshot_artifact_json_includes_lineage_and_restore_spec() {
    let root = unique_temp_dir("loongclaw-runtime-restore-artifact");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);

    let (_artifact_path, _snapshot, payload) = write_snapshot_artifact(&root, &config_path);

    assert_eq!(payload["schema"]["version"], 2);
    assert_eq!(payload["lineage"]["label"], "baseline");
    assert_eq!(payload["lineage"]["experiment_id"], "exp-runtime-restore");
    assert_eq!(payload["lineage"]["parent_snapshot_id"], "snapshot-parent");
    assert!(
        payload["lineage"]["snapshot_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(
        payload["restore_spec"]["provider"]["active_provider"],
        "deepseek-lab"
    );
    assert_eq!(
        payload["restore_spec"]["managed_skills"]["skills"][0]["skill_id"],
        "demo-skill"
    );
    assert_eq!(
        payload["restore_spec"]["managed_skills"]["skills"][0]["source_kind"],
        "directory"
    );
}

#[test]
fn runtime_snapshot_artifact_json_redacts_inline_provider_secrets_from_restore_spec() {
    let root = unique_temp_dir("loongclaw-runtime-restore-redaction");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", None),
    ]);
    let (config_path, mut config) = write_runtime_restore_config(&root);
    config.set_active_provider_profile(
        "deepseek-lab",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Deepseek,
                model: "deepseek-chat".to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "literal-secret-value".to_owned(),
                )),
                headers: BTreeMap::from([
                    (
                        "anthropic-api-key".to_owned(),
                        "literal-header-secret".to_owned(),
                    ),
                    ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
                    ("x-secret-beta".to_owned(), "literal-beta-secret".to_owned()),
                    ("x-goog-api-key".to_owned(), "${GOOGLE_API_KEY}".to_owned()),
                    ("user-agent".to_owned(), "loongclaw-test-suite".to_owned()),
                ]),
                ..Default::default()
            },
        },
    );
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write redaction fixture");

    let (_artifact_path, _snapshot, payload) = write_snapshot_artifact(&root, &config_path);

    let profile = &payload["restore_spec"]["provider"]["profiles"]["deepseek-lab"];
    assert!(profile["api_key"].is_null());
    assert!(profile["oauth_access_token"].is_null());
    assert!(profile["headers"]["anthropic-api-key"].is_null());
    assert_eq!(profile["headers"]["anthropic-version"], "2023-06-01");
    assert!(profile["headers"]["x-secret-beta"].is_null());
    assert_eq!(profile["headers"]["x-goog-api-key"], "${GOOGLE_API_KEY}");
    assert_eq!(profile["headers"]["user-agent"], "loongclaw-test-suite");
    let warnings = payload["restore_spec"]["warnings"]
        .as_array()
        .expect("warnings should be an array");
    assert!(
        warnings.iter().filter_map(Value::as_str).any(
            |warning| warning.contains("deepseek-lab") && warning.contains("anthropic-api-key")
        ),
        "restore spec should surface a warning for the redacted anthropic api key header"
    );
    assert!(
        warnings
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("deepseek-lab") && warning.contains("x-secret-beta")),
        "restore spec should surface a warning for the redacted beta-style secret header"
    );
}

#[test]
fn runtime_snapshot_artifact_json_warns_when_managed_skill_inventory_is_disabled() {
    let root = unique_temp_dir("loongclaw-runtime-restore-disabled-inventory");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, mut config) = write_runtime_restore_config(&root);
    config.external_skills.enabled = false;
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write disabled inventory fixture");

    let (_artifact_path, _snapshot, payload) = write_snapshot_artifact(&root, &config_path);

    assert!(
        payload["restore_spec"]["warnings"]
            .as_array()
            .expect("warnings should be an array")
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("inventory is disabled")),
        "restore spec should warn when managed skill inventory is disabled"
    );
}

#[test]
fn runtime_restore_dry_run_reports_pending_mutations_and_leaves_config_unchanged() {
    let root = unique_temp_dir("loongclaw-runtime-restore-dry-run");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);
    let (artifact_path, _snapshot, _payload) = write_snapshot_artifact(&root, &config_path);

    mutate_runtime_restore_config(&config_path, &root);

    let execution = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: false,
        },
    )
    .expect("runtime restore dry-run should succeed");

    assert!(execution.plan.can_apply);
    assert!(
        execution
            .plan
            .changed_surfaces
            .iter()
            .any(|surface| surface == "provider")
    );
    assert!(
        execution
            .plan
            .changed_surfaces
            .iter()
            .any(|surface| surface == "external_skills")
    );
    assert!(
        execution
            .plan
            .managed_skill_actions
            .iter()
            .any(|action| action.skill_id == "demo-skill" && action.action == "install")
    );

    let (_, reloaded) = mvp::config::load(Some(config_path.to_string_lossy().as_ref()))
        .expect("reload dry-run config");
    assert_eq!(reloaded.active_provider_id(), Some("openai-main"));
    assert!(!reloaded.external_skills.enabled);
    assert_eq!(
        reloaded.memory.profile,
        mvp::config::MemoryProfile::WindowOnly
    );
}

#[test]
fn runtime_restore_dry_run_blocks_apply_when_provider_credentials_were_redacted() {
    let root = unique_temp_dir("loongclaw-runtime-restore-redacted-block");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", None),
    ]);
    let (config_path, mut config) = write_runtime_restore_config(&root);
    config.set_active_provider_profile(
        "deepseek-lab",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Deepseek,
                model: "deepseek-chat".to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "literal-secret-value".to_owned(),
                )),
                ..Default::default()
            },
        },
    );
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write redacted restore config");

    let (artifact_path, _snapshot, _payload) = write_snapshot_artifact(&root, &config_path);

    let dry_run = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: false,
        },
    )
    .expect("runtime restore dry-run should surface blocking warnings");

    assert!(!dry_run.plan.can_apply);
    assert!(
        dry_run
            .plan
            .warnings
            .iter()
            .any(|warning| warning.contains("redacted inline provider credential")),
        "dry-run should keep the redacted credential warning visible"
    );

    let apply_error = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: true,
        },
    )
    .expect_err("apply should reject snapshots with redacted inline credentials");
    assert!(apply_error.contains("cannot be safely applied"));
}

#[test]
fn runtime_restore_dry_run_blocks_apply_when_managed_skill_inventory_was_not_captured() {
    let root = unique_temp_dir("loongclaw-runtime-restore-missing-managed-inventory");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, mut config) = write_runtime_restore_config(&root);
    config.external_skills.enabled = false;
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write disabled inventory restore config");

    let (artifact_path, _snapshot, payload) = write_snapshot_artifact(&root, &config_path);
    assert!(
        payload["restore_spec"]["warnings"]
            .as_array()
            .expect("warnings should be an array")
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("could not enumerate managed external skills")),
        "fixture should capture the managed-skill inventory warning"
    );

    let dry_run = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: false,
        },
    )
    .expect("runtime restore dry-run should still load the artifact");

    assert!(!dry_run.plan.can_apply);
    assert!(
        dry_run
            .plan
            .warnings
            .iter()
            .any(|warning| warning.contains("could not enumerate managed external skills")),
        "dry-run should keep the managed-skill inventory warning visible"
    );
}

#[test]
fn runtime_restore_dry_run_collects_current_inventory_when_target_snapshot_disables_external_skills()
 {
    let root = unique_temp_dir("loongclaw-runtime-restore-target-disabled");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);
    let (_artifact_path, _snapshot, mut payload) = write_snapshot_artifact(&root, &config_path);

    payload["restore_spec"]["external_skills"]["enabled"] = Value::Bool(false);
    payload["restore_spec"]["managed_skills"]["skills"] = Value::Array(Vec::new());
    let disabled_artifact_path = write_snapshot_artifact_payload(
        &root,
        "artifacts/runtime-snapshot-disabled.json",
        &payload,
    );

    let execution = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: disabled_artifact_path.display().to_string(),
            json: false,
            apply: false,
        },
    )
    .expect("restore planning should not depend on the target snapshot runtime being enabled");

    assert!(execution.plan.can_apply);
    assert!(
        execution
            .plan
            .managed_skill_actions
            .iter()
            .any(|action| action.skill_id == "demo-skill" && action.action == "remove"),
        "current managed inventory should still be collected and planned for removal"
    );
}

#[test]
fn runtime_restore_dry_run_ignores_source_path_only_drift_for_managed_skills() {
    let root = unique_temp_dir("loongclaw-runtime-restore-source-drift");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);
    let (_artifact_path, _snapshot, mut payload) = write_snapshot_artifact(&root, &config_path);

    payload["restore_spec"]["managed_skills"]["skills"][0]["source_path"] =
        Value::String("/tmp/other-machine/demo-skill".to_owned());
    let artifact_path = write_snapshot_artifact_payload(
        &root,
        "artifacts/runtime-snapshot-source-drift.json",
        &payload,
    );

    let execution = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: false,
        },
    )
    .expect("source-path-only drift should remain a no-op when content digest matches");

    assert!(execution.plan.managed_skill_actions.is_empty());
    assert!(
        !execution
            .plan
            .changed_surfaces
            .iter()
            .any(|surface| surface == "managed_skills")
    );
}

#[test]
fn runtime_restore_apply_replays_snapshot_state_and_verifies_post_apply_match() {
    let root = unique_temp_dir("loongclaw-runtime-restore-apply");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);
    let (artifact_path, _snapshot, _payload) = write_snapshot_artifact(&root, &config_path);

    mutate_runtime_restore_config(&config_path, &root);

    let execution = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: true,
        },
    )
    .expect("runtime restore apply should succeed");

    assert!(execution.applied);
    assert!(
        execution
            .verification
            .as_ref()
            .expect("apply should produce verification")
            .restored_exactly
    );

    let (_, reloaded) = mvp::config::load(Some(config_path.to_string_lossy().as_ref()))
        .expect("reload restored config");
    assert_eq!(reloaded.active_provider_id(), Some("deepseek-lab"));
    assert!(reloaded.external_skills.enabled);
    assert_eq!(
        reloaded.memory.profile,
        mvp::config::MemoryProfile::WindowPlusSummary
    );
    assert!(!reloaded.memory.fail_open);
    assert!(reloaded.acp.enabled);
    assert!(reloaded.tools.browser.enabled);
    assert_eq!(
        reloaded.tools.browser_companion.expected_version.as_deref(),
        Some("1.2.3")
    );

    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect restored snapshot");
    let payload =
        build_runtime_snapshot_cli_json_payload(&snapshot).expect("build runtime snapshot payload");
    assert_eq!(payload["provider"]["active_profile_id"], "deepseek-lab");
    assert!(
        payload["external_skills"]["policy"]["enabled"]
            .as_bool()
            .expect("enabled should be boolean")
    );
    assert!(
        payload["external_skills"]["inventory"]["skills"]
            .as_array()
            .expect("skills should be an array")
            .iter()
            .any(|skill| skill["skill_id"] == "demo-skill")
    );
}

#[test]
fn runtime_restore_apply_reports_verification_failure_without_reverting_applied_state() {
    let root = unique_temp_dir("loongclaw-runtime-restore-verification-failure");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);
    let (_artifact_path, _snapshot, mut payload) = write_snapshot_artifact(&root, &config_path);

    mutate_runtime_restore_config(&config_path, &root);

    payload["restore_spec"]["conversation"]["context_engine"] =
        Value::String("missing-engine".to_owned());
    let artifact_path = write_snapshot_artifact_payload(
        &root,
        "artifacts/runtime-snapshot-invalid-verification.json",
        &payload,
    );

    let execution = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: true,
        },
    )
    .expect("apply should surface verification failure instead of returning an error");

    let verification = execution
        .verification
        .as_ref()
        .expect("apply should still emit verification metadata");
    assert!(!verification.restored_exactly);
    assert!(
        verification
            .mismatches
            .iter()
            .any(|mismatch| mismatch == "verification_unavailable"),
        "verification failure should be explicit in the mismatch list"
    );
    assert!(
        verification
            .verification_error
            .as_deref()
            .is_some_and(|error| error.contains("post-apply runtime snapshot verification failed")),
        "verification error should preserve the post-apply failure context"
    );

    let raw_config = fs::read_to_string(&config_path).expect("read mutated config");
    assert!(
        raw_config.contains("context_engine = \"missing-engine\""),
        "apply should have already persisted the requested restore state"
    );
}

#[test]
fn runtime_restore_apply_rolls_back_managed_skill_changes_when_config_write_fails() {
    let root = unique_temp_dir("loongclaw-runtime-restore-rollback");
    let _env = RuntimeRestoreEnvGuard::set(&[
        ("LOONGCLAW_BROWSER_COMPANION_READY", Some("true")),
        ("OPENAI_API_KEY", None),
        ("RUNTIME_RESTORE_DEEPSEEK_KEY", Some("deepseek-demo-token")),
    ]);
    let (config_path, config) = write_runtime_restore_config(&root);
    install_demo_skill(&root, &config, &config_path);
    let (artifact_path, _snapshot, _payload) = write_snapshot_artifact(&root, &config_path);

    mutate_runtime_restore_config(&config_path, &root);

    let metadata = fs::metadata(&config_path).expect("read config metadata");
    let original_permissions = metadata.permissions();
    let mut readonly_permissions = original_permissions.clone();
    readonly_permissions.set_readonly(true);
    fs::set_permissions(&config_path, readonly_permissions).expect("mark config read-only");

    let apply_error = loongclaw_daemon::runtime_restore_cli::execute_runtime_restore_command(
        loongclaw_daemon::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config: Some(config_path.display().to_string()),
            snapshot: artifact_path.display().to_string(),
            json: false,
            apply: true,
        },
    )
    .expect_err("apply should fail when config persistence fails");

    fs::set_permissions(&config_path, original_permissions).expect("restore config write access");

    assert!(apply_error.contains("persist runtime restore config"));
    assert!(
        !root.join("managed-skills").join("demo-skill").exists(),
        "managed skill install should be rolled back when config persistence fails"
    );

    let index_path = root.join("managed-skills").join("index.json");
    if index_path.exists() {
        let index = serde_json::from_str::<Value>(
            &fs::read_to_string(&index_path).expect("read managed skill index"),
        )
        .expect("decode managed skill index");
        assert!(
            index["skills"]
                .as_array()
                .expect("index skills should be an array")
                .is_empty(),
            "rollback should leave the managed skill index empty"
        );
    }
}
