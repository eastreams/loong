use super::*;
use clap::Parser;
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
        crate::import_claw_cli::parse_legacy_claw_source("nanobot"),
        Some(mvp::migration::LegacyClawSource::Nanobot)
    );
    assert_eq!(
        crate::import_claw_cli::parse_legacy_claw_source("openclaw"),
        Some(mvp::migration::LegacyClawSource::OpenClaw)
    );
    assert_eq!(
        crate::import_claw_cli::parse_legacy_claw_source("picoclaw"),
        Some(mvp::migration::LegacyClawSource::PicoClaw)
    );
    assert_eq!(
        crate::import_claw_cli::parse_legacy_claw_source("zeroclaw"),
        Some(mvp::migration::LegacyClawSource::ZeroClaw)
    );
    assert_eq!(
        crate::import_claw_cli::parse_legacy_claw_source("nanoclaw"),
        Some(mvp::migration::LegacyClawSource::NanoClaw)
    );
    assert_eq!(
        crate::import_claw_cli::parse_legacy_claw_source("auto"),
        Some(mvp::migration::LegacyClawSource::Unknown)
    );
    assert_eq!(
        crate::import_claw_cli::parse_legacy_claw_source("unsupported"),
        None
    );
}

#[test]
fn run_import_claw_cli_writes_nativeized_config() {
    let legacy_root = unique_temp_dir("loongclaw-import-cli-legacy");
    let output_root = unique_temp_dir("loongclaw-import-cli-output");
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
    crate::import_claw_cli::run_import_claw_cli(crate::import_claw_cli::ImportClawCommandOptions {
        input: Some(legacy_root.display().to_string()),
        output: Some(output_path.display().to_string()),
        source: Some("nanobot".to_owned()),
        mode: crate::import_claw_cli::ImportClawMode::Apply,
        json: false,
        source_id: None,
        safe_profile_merge: false,
        primary_source_id: None,
        apply_external_skills_plan: false,
        force: true,
    })
    .expect("import command should succeed");

    let (_, config) = mvp::config::load(Some(&output_path.display().to_string()))
        .expect("imported config should load");
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
}

#[test]
fn run_import_claw_cli_plan_mode_returns_preview_without_writing() {
    let legacy_root = unique_temp_dir("loongclaw-import-cli-plan-legacy");
    let output_root = unique_temp_dir("loongclaw-import-cli-plan-output");
    fs::create_dir_all(&legacy_root).expect("create legacy root");
    fs::create_dir_all(&output_root).expect("create output root");

    write_file(
        &legacy_root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );
    let output_path = output_root.join("preview-only.toml");
    crate::import_claw_cli::run_import_claw_cli(crate::import_claw_cli::ImportClawCommandOptions {
        input: Some(legacy_root.display().to_string()),
        output: Some(output_path.display().to_string()),
        source: Some("nanobot".to_owned()),
        mode: crate::import_claw_cli::ImportClawMode::Plan,
        json: false,
        source_id: None,
        safe_profile_merge: false,
        primary_source_id: None,
        apply_external_skills_plan: false,
        force: true,
    })
    .expect("plan mode should succeed");

    assert!(
        !output_path.exists(),
        "plan mode should not write output config"
    );

    fs::remove_dir_all(&legacy_root).ok();
    fs::remove_dir_all(&output_root).ok();
}

#[test]
fn run_import_claw_cli_apply_selected_mode_writes_manifest_and_config() {
    let discovery_root = unique_temp_dir("loongclaw-import-cli-selected-discovery");
    let output_root = unique_temp_dir("loongclaw-import-cli-selected-output");
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
    crate::import_claw_cli::run_import_claw_cli(crate::import_claw_cli::ImportClawCommandOptions {
        input: Some(discovery_root.display().to_string()),
        output: Some(output_path.display().to_string()),
        source: None,
        mode: crate::import_claw_cli::ImportClawMode::ApplySelected,
        json: false,
        source_id: Some("openclaw".to_owned()),
        safe_profile_merge: false,
        primary_source_id: None,
        apply_external_skills_plan: false,
        force: true,
    })
    .expect("apply_selected mode should succeed");

    assert!(output_path.exists(), "selected import should write config");
    let manifest_path = output_root
        .join(".loongclaw-migration")
        .join("selected.toml.last-import.json");
    assert!(
        manifest_path.exists(),
        "apply_selected mode should write migration manifest"
    );

    fs::remove_dir_all(&discovery_root).ok();
    fs::remove_dir_all(&output_root).ok();
}

#[test]
fn run_import_claw_cli_apply_selected_mode_can_apply_external_skill_plan() {
    let discovery_root = unique_temp_dir("loongclaw-import-cli-external-skills-discovery");
    let output_root = unique_temp_dir("loongclaw-import-cli-external-skills-output");
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

    let output_path = output_root.join("selected-external.toml");
    crate::import_claw_cli::run_import_claw_cli(crate::import_claw_cli::ImportClawCommandOptions {
        input: Some(discovery_root.display().to_string()),
        output: Some(output_path.display().to_string()),
        source: None,
        mode: crate::import_claw_cli::ImportClawMode::ApplySelected,
        json: false,
        source_id: Some("openclaw".to_owned()),
        safe_profile_merge: false,
        primary_source_id: None,
        apply_external_skills_plan: true,
        force: true,
    })
    .expect("apply_selected mode with external skills should succeed");

    let raw = fs::read_to_string(&output_path).expect("read generated config");
    assert!(raw.contains("Imported External Skills Artifacts"));
    assert!(raw.contains("kind=skills_catalog"));
    let external_manifest_path = output_root
        .join(".loongclaw-migration")
        .join("selected-external.toml.external-skills.json");
    assert!(
        external_manifest_path.exists(),
        "apply_selected mode should write external skills manifest"
    );

    fs::remove_dir_all(&discovery_root).ok();
    fs::remove_dir_all(&output_root).ok();
}

#[test]
fn import_claw_cli_defaults_to_plan_mode() {
    let cli = Cli::try_parse_from(["loongclaw", "import-claw", "--input", "/tmp/legacy"])
        .expect("import-claw args should parse");
    #[allow(clippy::wildcard_enum_match_arm)]
    match cli.command.expect("subcommand should exist") {
        Commands::ImportClaw { mode, .. } => {
            assert_eq!(mode, crate::import_claw_cli::ImportClawMode::Plan);
        }
        other => panic!("expected import-claw command, got {other:?}"),
    }
}

#[test]
fn import_claw_cli_accepts_selection_alias_flags() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "import-claw",
        "--mode",
        "apply_selected",
        "--input",
        "/tmp/legacy",
        "--selection-id",
        "openclaw",
        "--primary-selection-id",
        "nanobot",
        "--apply-external-skills-plan",
    ])
    .expect("selection alias flags should parse");

    #[allow(clippy::wildcard_enum_match_arm)]
    match cli.command.expect("subcommand should exist") {
        Commands::ImportClaw {
            source_id,
            primary_source_id,
            apply_external_skills_plan,
            ..
        } => {
            assert_eq!(source_id.as_deref(), Some("openclaw"));
            assert_eq!(primary_source_id.as_deref(), Some("nanobot"));
            assert!(apply_external_skills_plan);
        }
        other => panic!("expected import-claw command, got {other:?}"),
    }
}
