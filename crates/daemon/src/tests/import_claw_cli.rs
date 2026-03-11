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
        input: legacy_root.display().to_string(),
        output: Some(output_path.display().to_string()),
        source: Some("nanobot".to_owned()),
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
