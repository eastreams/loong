use super::*;
use loongclaw_contracts::SecretRef;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT_TEMP_DIR_SEED: AtomicUsize = AtomicUsize::new(1);
    let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let process_id = std::process::id();
    std::env::temp_dir().join(format!("{prefix}-{process_id}-{seed}-{nanos}"))
}

fn write_status_config(
    root: &Path,
    acp_enabled: bool,
    tool_schema_mode: mvp::config::ProviderToolSchemaModeConfig,
) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let sqlite_path = root.join("memory.sqlite3");
    let mut config = mvp::config::LoongClawConfig::default();
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(root.display().to_string());
    config.set_active_provider_profile(
        "demo-openai",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "gpt-4.1-mini".to_owned(),
                api_key: Some(SecretRef::Inline("demo-token".to_owned())),
                tool_schema_mode,
                ..Default::default()
            },
        },
    );

    if acp_enabled {
        config.acp.enabled = true;
        config.acp.dispatch.enabled = true;
        config.acp.default_agent = Some("codex".to_owned());
        config.acp.allowed_agents = vec!["codex".to_owned()];
    }

    let config_path = root.join("loongclaw.toml");
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");
    mvp::config::write(Some(config_path_text), &config, true).expect("write config fixture");
    config_path
}

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn run_status_cli_process(
    config_path: &Path,
    home_root: &Path,
    args: &[&str],
    context: &str,
) -> std::process::Output {
    let home_root_text = home_root.to_str().expect("home root should be valid utf-8");
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");

    Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .arg("status")
        .arg("--config")
        .arg(config_path_text)
        .args(args)
        .env("LOONG_HOME", home_root_text)
        .output()
        .expect(context)
}

#[test]
fn cli_status_help_mentions_operator_runtime_summary() {
    let help = render_cli_help(["status"]);

    assert!(
        help.contains("operator-readable runtime summary"),
        "status help should explain the aggregated operator surface: {help}"
    );
    assert!(
        help.contains("--json"),
        "status help should surface machine-readable output: {help}"
    );
}

#[test]
fn cli_status_parse_accepts_config_and_json_flags() {
    let cli = try_parse_cli([
        "loongclaw",
        "status",
        "--config",
        "/tmp/loongclaw.toml",
        "--json",
    ])
    .expect("status CLI should parse");

    let command = cli.command.expect("CLI should parse a subcommand");
    let Commands::Status { config, json } = command else {
        panic!("unexpected CLI parse result: {command:?}");
    };

    assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
    assert!(json);
}

#[test]
fn status_cli_json_rolls_up_gateway_acp_and_work_unit_sections() {
    let root = unique_temp_dir("loongclaw-status-cli-json");
    let home_root = root.join("home");
    fs::create_dir_all(&home_root).expect("create home root");
    let config_path = write_status_config(
        &root,
        true,
        mvp::config::ProviderToolSchemaModeConfig::EnabledWithDowngrade,
    );
    let output =
        run_status_cli_process(&config_path, &home_root, &["--json"], "run status CLI json");

    if !output.status.success() {
        let stdout = render_output(&output.stdout);
        let stderr = render_output(&output.stderr);
        panic!(
            "status CLI json should succeed: status={:?}\nstdout={stdout}\nstderr={stderr}",
            output.status.code()
        );
    }

    let stdout = render_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout).expect("decode status json");

    assert_eq!(payload["schema"]["surface"], "status");
    assert_eq!(payload["gateway"]["owner"]["phase"], "stopped");
    assert_eq!(
        payload["gateway"]["runtime"]["tool_calling"]["availability"],
        "ready"
    );
    assert_eq!(
        payload["gateway"]["runtime"]["tool_calling"]["structured_tool_schema_enabled"],
        true
    );
    assert_eq!(payload["acp"]["enabled"], true);
    let acp_availability = payload["acp"]["availability"]
        .as_str()
        .expect("acp availability string");
    assert!(
        matches!(acp_availability, "available" | "unavailable"),
        "unexpected acp availability: {payload:#?}"
    );
    if acp_availability == "available" {
        assert!(payload["acp"]["observability"].is_object());
    } else {
        assert!(payload["acp"]["error"].is_string());
    }

    let work_units_availability = payload["work_units"]["availability"]
        .as_str()
        .expect("work-unit availability string");
    assert!(
        matches!(work_units_availability, "available" | "unavailable"),
        "unexpected work-unit availability: {payload:#?}"
    );
    if work_units_availability == "available" {
        assert_eq!(payload["work_units"]["health"]["total_count"], 0);
    } else {
        assert!(payload["work_units"]["error"].is_string());
    }
    assert!(
        payload["recipes"]
            .as_array()
            .map(|recipes| recipes.len() >= 4)
            .unwrap_or(false),
        "status JSON should include drill-down recipes: {payload:#?}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn status_cli_text_surfaces_section_summaries_and_recipes() {
    let root = unique_temp_dir("loongclaw-status-cli-text");
    let home_root = root.join("home");
    fs::create_dir_all(&home_root).expect("create home root");
    let config_path = write_status_config(
        &root,
        false,
        mvp::config::ProviderToolSchemaModeConfig::Disabled,
    );
    let output = run_status_cli_process(&config_path, &home_root, &[], "run status CLI text");

    if !output.status.success() {
        let stdout = render_output(&output.stdout);
        let stderr = render_output(&output.stderr);
        panic!(
            "status CLI text should succeed: status={:?}\nstdout={stdout}\nstderr={stderr}",
            output.status.code()
        );
    }

    let stdout = render_output(&output.stdout);

    assert!(stdout.contains("gateway phase=stopped"));
    assert!(stdout.contains("tool_calling availability=degraded"));
    assert!(stdout.contains("structured_tool_schema_enabled=false"));
    assert!(stdout.contains("acp enabled=false availability=disabled"));
    assert!(stdout.contains("work_units availability="));
    assert!(stdout.contains("recipes:"));

    fs::remove_dir_all(&root).ok();
}
