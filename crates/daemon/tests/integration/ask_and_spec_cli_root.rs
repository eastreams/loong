use super::*;

#[test]
fn cli_uses_loong_program_name() {
    assert_eq!(cli_command_name(), "loong");
}

#[test]
fn cli_import_help_explains_explicit_power_user_flow() {
    let help = render_cli_help(["import"]);

    assert!(
        help.contains("Power-user import flow"),
        "import help should explain when to use the explicit import command: {help}"
    );
    assert!(
        help.contains("--source-path"),
        "import help should surface the path-level disambiguation flag: {help}"
    );
    assert!(
        help.contains("loong onboard"),
        "import help should direct guided users back to onboard: {help}"
    );
    assert!(
        help.contains(&format!(
            "--provider <{}>",
            mvp::config::PROVIDER_SELECTOR_PLACEHOLDER
        )),
        "import help should expose the shared provider selector placeholder: {help}"
    );
    assert!(
        help.contains(mvp::config::PROVIDER_SELECTOR_HUMAN_SUMMARY),
        "import help should reuse the shared provider selector summary: {help}"
    );
}

#[test]
fn cli_migrate_help_explains_explicit_config_import_flow() {
    let help = render_cli_help(["migrate"]);

    assert!(
        help.contains("Power-user config import flow"),
        "migrate help should explain when to use the explicit config import command: {help}"
    );
    assert!(
        help.contains("--mode <MODE>"),
        "migrate help should surface the required mode flag: {help}"
    );
    assert!(
        help.contains("discover"),
        "migrate help should list supported migration modes: {help}"
    );
    assert!(
        help.contains("loong onboard"),
        "migrate help should direct guided users back to onboard: {help}"
    );
}

#[test]
fn cli_onboard_help_mentions_detected_reusable_settings() {
    let help = render_cli_help(["onboard"]);

    assert!(
        help.contains("detect"),
        "onboard help should mention that it detects reusable settings: {help}"
    );
    assert!(
        help.contains("provider, channels, or workspace guidance"),
        "onboard help should explain the kinds of detected settings it can reuse: {help}"
    );
    assert!(
        help.contains(&format!(
            "--provider <{}>",
            mvp::config::PROVIDER_SELECTOR_PLACEHOLDER
        )),
        "onboard help should expose the shared provider selector placeholder: {help}"
    );
    assert!(
        help.contains(mvp::config::PROVIDER_SELECTOR_HUMAN_SUMMARY),
        "onboard help should reuse the shared provider selector summary: {help}"
    );
}

#[test]
fn cli_ask_help_mentions_one_shot_assistant_usage() {
    let help = render_cli_help(["ask"]);

    assert!(
        help.contains("one-shot"),
        "ask help should describe the non-interactive one-shot flow: {help}"
    );
    assert!(
        help.contains("--message <MESSAGE>"),
        "ask help should require an inline message input: {help}"
    );
    assert!(
        help.contains("loong chat"),
        "ask help should point users to chat for the interactive path: {help}"
    );
}

#[test]
fn cli_runtime_restore_help_mentions_dry_run_default() {
    let help = render_cli_help(["runtime-restore"]);

    assert!(
        help.contains("Dry-run by default"),
        "runtime-restore help should explain the default dry-run behavior: {help}"
    );
    assert!(
        help.contains("--apply"),
        "runtime-restore help should explain how to perform mutations: {help}"
    );
}

#[test]
fn ask_cli_accepts_message_session_and_acp_flags() {
    let cli = try_parse_cli([
        "loong",
        "ask",
        "--message",
        "Summarize this repository",
        "--session",
        "telegram:42",
        "--acp",
        "--acp-event-stream",
        "--acp-bootstrap-mcp-server",
        "filesystem",
        "--acp-cwd",
        "/workspace/project",
    ])
    .expect("ask CLI should parse one-shot flags");

    match cli.command {
        Some(Commands::Ask {
            message,
            session,
            acp,
            acp_event_stream,
            acp_bootstrap_mcp_server,
            acp_cwd,
            ..
        }) => {
            assert_eq!(message, "Summarize this repository");
            assert_eq!(session.as_deref(), Some("telegram:42"));
            assert!(acp);
            assert!(acp_event_stream);
            assert_eq!(acp_bootstrap_mcp_server, vec!["filesystem".to_owned()]);
            assert_eq!(acp_cwd.as_deref(), Some("/workspace/project"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn ask_cli_accepts_latest_session_selector() {
    let cli = try_parse_cli([
        "loong",
        "ask",
        "--message",
        "Summarize this repository",
        "--session",
        "latest",
    ])
    .expect("ask CLI should accept the latest session selector");

    match cli.command {
        Some(Commands::Ask {
            message, session, ..
        }) => {
            assert_eq!(message, "Summarize this repository");
            assert_eq!(session.as_deref(), Some("latest"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn init_spec_cli_accepts_plugin_trust_guard_preset() {
    let cli = try_parse_cli([
        "loong",
        "init-spec",
        "--output",
        "/tmp/plugin-trust-guard.json",
        "--preset",
        "plugin-trust-guard",
    ])
    .expect("init-spec CLI should parse plugin trust guard preset");

    match cli.command {
        Some(Commands::InitSpec { output, preset }) => {
            assert_eq!(output, "/tmp/plugin-trust-guard.json");
            assert_eq!(preset, InitSpecPreset::PluginTrustGuard);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn run_spec_cli_accepts_render_summary_flag() {
    let cli = try_parse_cli([
        "loong",
        "run-spec",
        "--spec",
        "/tmp/tool-search-trusted.json",
        "--render-summary",
    ])
    .expect("run-spec CLI should parse render summary flag");

    match cli.command {
        Some(Commands::RunSpec {
            spec,
            print_audit,
            render_summary,
            ..
        }) => {
            assert_eq!(spec, "/tmp/tool-search-trusted.json");
            assert!(!print_audit);
            assert!(render_summary);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn ask_cli_requires_message_flag() {
    let error = try_parse_cli(["loong", "ask"]).expect_err("ask without --message should fail");
    let rendered = error.to_string();

    assert!(
        rendered.contains("--message <MESSAGE>"),
        "parse failure should mention the required message flag: {rendered}"
    );
}
