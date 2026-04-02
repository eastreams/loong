use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn isolated_home_guard(prefix: &str) -> (PathBuf, MigrationEnvironmentGuard) {
    let home = unique_temp_dir(prefix);
    fs::create_dir_all(&home).expect("create isolated home");
    let home_string = home.to_string_lossy().into_owned();
    let guard = MigrationEnvironmentGuard::set(&[("HOME", Some(home_string.as_str()))]);
    (home, guard)
}

fn write_file(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, content).expect("write fixture");
}

#[test]
fn parse_legacy_claw_source_accepts_supported_ids() {
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("nanobot"),
        Some(mvp::migration::LegacyClawSource::Nanobot)
    );
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("openclaw"),
        Some(mvp::migration::LegacyClawSource::OpenClaw)
    );
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("picoclaw"),
        Some(mvp::migration::LegacyClawSource::PicoClaw)
    );
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("zeroclaw"),
        Some(mvp::migration::LegacyClawSource::ZeroClaw)
    );
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("nanoclaw"),
        Some(mvp::migration::LegacyClawSource::NanoClaw)
    );
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("auto"),
        Some(mvp::migration::LegacyClawSource::Unknown)
    );
    assert_eq!(
        loongclaw_daemon::migrate_cli::parse_legacy_claw_source("unsupported"),
        None
    );
}

#[test]
fn run_migrate_cli_writes_nativeized_config() {
    let legacy_root = unique_temp_dir("loongclaw-import-cli-legacy");
    let output_root = unique_temp_dir("loongclaw-import-cli-output");
    let (home_root, _env_guard) = isolated_home_guard("loongclaw-import-cli-home");
    fs::create_dir_all(&legacy_root).expect("create legacy root");
    fs::create_dir_all(&output_root).expect("create output root");

    write_file(
        &legacy_root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );
    write_file(
        &legacy_root,
        "IDENTITY.md",
        "# Identity\n\n- Name: Release copilot\n- Motto: your nanobot agent for deploys\n",
    );

    let output_path = output_root.join("loongclaw.toml");
    loongclaw_daemon::migrate_cli::run_migrate_cli(
        loongclaw_daemon::migrate_cli::MigrateCommandOptions {
            input: Some(legacy_root.display().to_string()),
            output: Some(output_path.display().to_string()),
            source: Some("nanobot".to_owned()),
            mode: loongclaw_daemon::migrate_cli::MigrateMode::Apply,
            json: false,
            source_id: None,
            safe_profile_merge: false,
            primary_source_id: None,
            apply_external_skills_plan: false,
            force: true,
        },
    )
    .expect("migrate command should succeed");

    let (_, config) = mvp::config::load(Some(&output_path.display().to_string()))
        .expect("migrated config should load");
    assert_eq!(
        config.cli.prompt_pack_id.as_deref(),
        Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID)
    );
    assert_eq!(
        config.memory.profile,
        mvp::config::MemoryProfile::ProfilePlusWindow
    );
    assert_eq!(
        config.cli.system_prompt_addendum.as_deref(),
        Some(
            "## Imported SOUL.md\n# Soul\n\nAlways prefer concise shell output. updated by LoongClaw."
        )
    );
    assert_eq!(
        config.memory.profile_note.as_deref(),
        Some(
            "## Imported IDENTITY.md\n# Identity\n\n- Name: Release copilot\n- Motto: your LoongClaw agent for deploys"
        )
    );

    fs::remove_dir_all(&legacy_root).ok();
    fs::remove_dir_all(&output_root).ok();
    fs::remove_dir_all(&home_root).ok();
}

#[test]
fn run_migrate_cli_plan_mode_returns_preview_without_writing() {
    let legacy_root = unique_temp_dir("loongclaw-import-cli-plan-legacy");
    let output_root = unique_temp_dir("loongclaw-import-cli-plan-output");
    let (home_root, _env_guard) = isolated_home_guard("loongclaw-import-cli-plan-home");
    fs::create_dir_all(&legacy_root).expect("create legacy root");
    fs::create_dir_all(&output_root).expect("create output root");

    write_file(
        &legacy_root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );
    let output_path = output_root.join("preview-only.toml");
    loongclaw_daemon::migrate_cli::run_migrate_cli(
        loongclaw_daemon::migrate_cli::MigrateCommandOptions {
            input: Some(legacy_root.display().to_string()),
            output: Some(output_path.display().to_string()),
            source: Some("nanobot".to_owned()),
            mode: loongclaw_daemon::migrate_cli::MigrateMode::Plan,
            json: false,
            source_id: None,
            safe_profile_merge: false,
            primary_source_id: None,
            apply_external_skills_plan: false,
            force: true,
        },
    )
    .expect("plan mode should succeed");

    assert!(
        !output_path.exists(),
        "plan mode should not write output config"
    );

    fs::remove_dir_all(&legacy_root).ok();
    fs::remove_dir_all(&output_root).ok();
    fs::remove_dir_all(&home_root).ok();
}

#[test]
fn run_migrate_cli_apply_selected_mode_writes_manifest_and_config() {
    let discovery_root = unique_temp_dir("loongclaw-import-cli-selected-discovery");
    let output_root = unique_temp_dir("loongclaw-import-cli-selected-output");
    let (home_root, _env_guard) = isolated_home_guard("loongclaw-import-cli-selected-home");
    fs::create_dir_all(&discovery_root).expect("create discovery root");
    fs::create_dir_all(&output_root).expect("create output root");

    let openclaw_root = discovery_root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw source root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );

    let output_path = output_root.join("selected.toml");
    loongclaw_daemon::migrate_cli::run_migrate_cli(
        loongclaw_daemon::migrate_cli::MigrateCommandOptions {
            input: Some(discovery_root.display().to_string()),
            output: Some(output_path.display().to_string()),
            source: None,
            mode: loongclaw_daemon::migrate_cli::MigrateMode::ApplySelected,
            json: false,
            source_id: Some("openclaw".to_owned()),
            safe_profile_merge: false,
            primary_source_id: None,
            apply_external_skills_plan: false,
            force: true,
        },
    )
    .expect("apply_selected mode should succeed");

    assert!(
        output_path.exists(),
        "selected migration should write config"
    );
    let manifest_path = output_root
        .join(".loongclaw-migration")
        .join("selected.toml.last-migration.json");
    assert!(
        manifest_path.exists(),
        "apply_selected mode should write migration manifest"
    );

    fs::remove_dir_all(&discovery_root).ok();
    fs::remove_dir_all(&output_root).ok();
    fs::remove_dir_all(&home_root).ok();
}

#[test]
fn run_migrate_cli_apply_selected_mode_can_apply_external_skill_plan() {
    let discovery_root = unique_temp_dir("loongclaw-import-cli-external-skills-discovery");
    let output_root = unique_temp_dir("loongclaw-import-cli-external-skills-output");
    let (home_root, _env_guard) = isolated_home_guard("loongclaw-import-cli-external-skills-home");
    fs::create_dir_all(&discovery_root).expect("create discovery root");
    fs::create_dir_all(&output_root).expect("create output root");

    let openclaw_root = discovery_root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw source root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );
    write_file(
        &discovery_root,
        "SKILLS.md",
        "# Skills\n\n- custom/skill-a\n",
    );
    write_file(
        &discovery_root,
        ".codex/skills/release-guard/SKILL.md",
        "# Release Guard\n\nUse this skill when release discipline matters.\n",
    );

    let output_path = output_root.join("selected-external.toml");
    loongclaw_daemon::migrate_cli::run_migrate_cli(
        loongclaw_daemon::migrate_cli::MigrateCommandOptions {
            input: Some(discovery_root.display().to_string()),
            output: Some(output_path.display().to_string()),
            source: None,
            mode: loongclaw_daemon::migrate_cli::MigrateMode::ApplySelected,
            json: false,
            source_id: Some("openclaw".to_owned()),
            safe_profile_merge: false,
            primary_source_id: None,
            apply_external_skills_plan: true,
            force: true,
        },
    )
    .expect("apply_selected mode with external skills should succeed");

    let raw = fs::read_to_string(&output_path).expect("read generated config");
    assert!(raw.contains("Imported External Skills Artifacts"));
    assert!(raw.contains("kind=skills_catalog"));
    assert!(
        raw.contains("enabled = true"),
        "bridged installs should enable external skills in the written config"
    );
    let external_manifest_path = output_root
        .join(".loongclaw-migration")
        .join("selected-external.toml.external-skills.json");
    assert!(
        external_manifest_path.exists(),
        "apply_selected mode should write external skills manifest"
    );
    assert!(
        output_root
            .join("external-skills-installed")
            .join("release-guard")
            .join("SKILL.md")
            .exists(),
        "apply_selected mode should bridge installable local skills into the managed runtime"
    );

    fs::remove_dir_all(&discovery_root).ok();
    fs::remove_dir_all(&output_root).ok();
    fs::remove_dir_all(&home_root).ok();
}

#[test]
fn run_migrate_cli_apply_mode_rejects_output_path_outside_configured_file_root() {
    let policy_root = unique_temp_dir("loongclaw-import-cli-policy-root");
    let legacy_root = policy_root.join("legacy-root");
    let escape_root = unique_temp_dir("loongclaw-import-cli-policy-escape");
    let (home_root, _env_guard) = isolated_home_guard("loongclaw-import-cli-policy-home");
    fs::create_dir_all(&legacy_root).expect("create legacy root under policy root");
    fs::create_dir_all(&escape_root).expect("create escape root");

    let mut config = mvp::config::LoongClawConfig::default();
    config.tools.file_root = Some(policy_root.display().to_string());
    mvp::config::write(None, &config, true).expect("write discovered config");

    write_file(
        &legacy_root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );

    let escape_output = escape_root.join("outside-root.toml");
    let error = loongclaw_daemon::migrate_cli::run_migrate_cli(
        loongclaw_daemon::migrate_cli::MigrateCommandOptions {
            input: Some(legacy_root.display().to_string()),
            output: Some(escape_output.display().to_string()),
            source: Some("nanobot".to_owned()),
            mode: loongclaw_daemon::migrate_cli::MigrateMode::Apply,
            json: false,
            source_id: None,
            safe_profile_merge: false,
            primary_source_id: None,
            apply_external_skills_plan: false,
            force: true,
        },
    )
    .expect_err("policy root should deny writing outside configured file root");

    assert!(
        error.contains("policy_denied"),
        "expected policy denial, got: {error}"
    );

    fs::remove_dir_all(&policy_root).ok();
    fs::remove_dir_all(&escape_root).ok();
    fs::remove_dir_all(&home_root).ok();
}

#[test]
fn migrate_cli_ux_apply_mode_reports_flag_level_output_requirement() {
    let error = loongclaw_daemon::migrate_cli::run_migrate_cli(
        loongclaw_daemon::migrate_cli::MigrateCommandOptions {
            input: Some(".".to_owned()),
            output: None,
            source: None,
            mode: loongclaw_daemon::migrate_cli::MigrateMode::Apply,
            json: false,
            source_id: None,
            safe_profile_merge: false,
            primary_source_id: None,
            apply_external_skills_plan: false,
            force: false,
        },
    )
    .expect_err("apply mode without --output should fail");

    assert_eq!(
        error,
        "`--output` is required for `loong migrate --mode apply`"
    );
    assert!(
        !error.contains("payload.output_path"),
        "raw tool payload wording leaked into CLI error: {error}"
    );
}

#[test]
fn migrate_cli_ux_help_mentions_mode_specific_required_flags() {
    let binary_path = cli_binary_path();
    let output = std::process::Command::new(binary_path)
        .args(["migrate", "--help"])
        .output()
        .expect("run loongclaw migrate --help");

    assert!(output.status.success(), "help should succeed");
    let stdout = String::from_utf8(output.stdout).expect("help output should be utf8");
    assert!(
        stdout.contains("apply: requires `--input` and `--output`"),
        "help should mention apply mode requirements, got: {stdout}"
    );
    assert!(
        stdout.contains("rollback_last_apply: requires `--output`"),
        "help should mention rollback requirements, got: {stdout}"
    );
}
