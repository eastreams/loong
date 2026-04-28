use super::*;
use crate::test_support::ScopedEnv;
use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static UNIQUE_TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let pid = process::id();
    let counter = UNIQUE_TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}-{counter}"))
}

fn isolated_home(prefix: &str) -> (ScopedEnv, PathBuf, PathBuf) {
    let mut env = ScopedEnv::new();
    let home = unique_temp_dir(prefix);
    fs::create_dir_all(&home).expect("create isolated home");
    env.set("HOME", &home);
    env.remove("LOONG_HOME");
    env.remove("LOONG_CONFIG_PATH");
    env.remove("LOONGCLAW_CONFIG_PATH");
    let config_path = home.join(".loong").join("loong.toml");
    env.set("LOONG_CONFIG_PATH", &config_path);
    (env, home, config_path)
}

#[test]
fn resolve_default_entry_command_routes_to_onboard_when_config_is_missing() {
    let (_env, _home, _config_path) = isolated_home("loongclaw-default-entry-missing");

    assert!(
        matches!(resolve_default_entry_command(), Commands::Onboard { .. }),
        "missing config should route to onboard"
    );
}

#[test]
fn resolve_default_entry_command_ignores_legacy_home_when_config_is_missing() {
    let (_env, home, _config_path) = isolated_home("loongclaw-default-entry-legacy-home");
    let legacy_home = home.join(".loongclaw");
    fs::create_dir_all(&legacy_home).expect("create legacy home");

    assert!(
        matches!(resolve_default_entry_command(), Commands::Onboard { .. }),
        "legacy home alone should still route to onboard"
    );
}

#[test]
fn resolve_default_entry_command_routes_to_welcome_when_default_config_exists() {
    let (_env, _home, config_path) = isolated_home("loongclaw-default-entry-present");

    mvp::config::write(
        Some(config_path.to_str().expect("utf8 config path")),
        &mvp::config::LoongConfig::default(),
        true,
    )
    .expect("write default config");

    assert!(
        matches!(resolve_default_entry_command(), Commands::Welcome),
        "present config should route to welcome"
    );
}

#[test]
fn resolve_default_entry_command_ignores_loongclaw_config_path_without_compat_shim() {
    let (mut env, _home, _default_config_path) = isolated_home("loong-default-entry-legacy-env");
    let config_path = unique_temp_dir("loongclaw-default-entry-env").join("custom-config.toml");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    mvp::config::write(
        Some(config_path.to_str().expect("utf8 config path")),
        &mvp::config::LoongConfig::default(),
        true,
    )
    .expect("write explicit config");
    env.set("LOONGCLAW_CONFIG_PATH", &config_path);

    assert!(
        matches!(resolve_default_entry_command(), Commands::Onboard { .. }),
        "legacy env override should not bypass the canonical default-entry path without compatibility shims"
    );
}

#[test]
fn resolve_default_entry_command_honors_loong_config_path_override() {
    let mut env = ScopedEnv::new();
    let config_path = unique_temp_dir("loong-default-entry-env").join("custom-config.toml");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    mvp::config::write(
        Some(config_path.to_str().expect("utf8 config path")),
        &mvp::config::LoongConfig::default(),
        true,
    )
    .expect("write explicit config");
    env.set("LOONG_CONFIG_PATH", &config_path);

    assert!(
        matches!(resolve_default_entry_command(), Commands::Welcome),
        "new env override config should route to welcome"
    );
}

#[test]
fn resolve_default_entry_command_routes_to_onboard_when_config_path_is_a_directory() {
    let (mut env, _home, _config_path) = isolated_home("loong-default-entry-dir");
    let config_dir = unique_temp_dir("loong-default-entry-dir");
    fs::create_dir_all(&config_dir).expect("create config directory");
    env.set("LOONG_CONFIG_PATH", &config_dir);

    assert!(
        matches!(resolve_default_entry_command(), Commands::Onboard { .. }),
        "directory config path should still route to onboard"
    );
}

#[test]
fn redacted_command_name_omits_sensitive_command_payloads() {
    let command = Commands::Turn {
        command: TurnCommands::Run {
            config: Some("/tmp/private.toml".to_owned()),
            session: Some("session-secret".to_owned()),
            message: "secret objective".to_owned(),
            acp: false,
            acp_event_stream: false,
            acp_bootstrap_mcp_server: Vec::new(),
            acp_cwd: None,
        },
    };

    let redacted_name = redacted_command_name(&command);

    assert_eq!(redacted_name, "turn_run");
}

#[test]
fn run_welcome_cli_rejects_missing_config_file() {
    let (mut env, _home, _default_config_path) = isolated_home("loong-welcome-missing");
    let config_path = unique_temp_dir("loong-welcome-missing").join("missing-config.toml");
    env.set("LOONG_CONFIG_PATH", &config_path);

    let error = run_welcome_cli().expect_err("missing config should fail welcome");

    assert!(
        error.contains("Config file not found"),
        "welcome should explain the missing config file: {error}"
    );
    assert!(
        error.contains("loong onboard"),
        "welcome should point users back to onboarding: {error}"
    );
}

#[test]
fn run_welcome_cli_rejects_directory_config_path() {
    let (mut env, _home, _default_config_path) = isolated_home("loong-welcome-dir");
    let config_dir = unique_temp_dir("loong-welcome-dir");
    fs::create_dir_all(&config_dir).expect("create config directory");
    env.set("LOONG_CONFIG_PATH", &config_dir);

    let error = run_welcome_cli().expect_err("directory config path should fail welcome");

    assert!(
        error.contains("Config file not found"),
        "welcome should reject directory config paths as missing config files: {error}"
    );
}

#[test]
fn resolve_welcome_config_path_honors_loong_config_path_override() {
    let mut env = ScopedEnv::new();
    let config_path = unique_temp_dir("loong-welcome-present").join("welcome-config.toml");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    mvp::config::write(
        Some(config_path.to_str().expect("utf8 config path")),
        &mvp::config::LoongConfig::default(),
        true,
    )
    .expect("write explicit config");
    env.set("LOONG_CONFIG_PATH", &config_path);

    let resolved_path =
        resolve_welcome_config_path().expect("new config path env should resolve correctly");

    assert_eq!(resolved_path, config_path);
}

#[test]
fn render_welcome_banner_includes_version_and_next_commands() {
    let config = mvp::config::LoongConfig::default();
    let rendered = render_welcome_banner(Path::new("/tmp/loongclaw's config.toml"), &config);

    assert!(
        rendered.contains(env!("CARGO_PKG_VERSION")),
        "welcome banner should include the current version: {rendered}"
    );
    assert!(
        rendered.contains("welcome back"),
        "welcome banner should promote a dedicated welcome title: {rendered}"
    );
    assert!(
        rendered.contains("start here"),
        "welcome banner should lead with a start-here handoff: {rendered}"
    );
    assert!(
        rendered.contains("loong ask --config '/tmp/loongclaw'\"'\"'s config.toml'"),
        "welcome banner should include a quoted ask command: {rendered}"
    );
    assert!(
        rendered.contains("loong chat --config '/tmp/loongclaw'\"'\"'s config.toml'"),
        "welcome banner should include a quoted chat command: {rendered}"
    );
    assert!(
        rendered.contains("loong personalize"),
        "welcome banner should include a quoted personalize command: {rendered}"
    );
}

#[test]
fn render_welcome_banner_separates_continue_setup_actions() {
    let config = mvp::config::LoongConfig::default();

    let rendered = render_welcome_banner(Path::new("/tmp/loongclaw's config.toml"), &config);

    assert!(
        rendered.contains("continue setup"),
        "welcome banner should expose setup-extension actions as their own group: {rendered}"
    );
    assert!(
        rendered.contains("choose a channel"),
        "welcome banner should keep the channel handoff visible in the continue-setup group: {rendered}"
    );
}
