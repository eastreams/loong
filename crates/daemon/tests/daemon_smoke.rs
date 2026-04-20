#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    unused_imports,
    dead_code,
    unsafe_code,
    clippy::disallowed_methods,
    clippy::undocumented_unsafe_blocks,
    clippy::wildcard_enum_match_arm
)]

mod support;
use support::*;

mod integration {
    use super::*;

    #[path = "architecture.rs"]
    mod architecture;
    #[path = "cli_tests.rs"]
    mod cli_tests;
    #[path = "gateway_api_health.rs"]
    mod gateway_api_health;
    #[path = "gateway_read_models.rs"]
    mod gateway_read_models;
    #[path = "runtime_snapshot_cli.rs"]
    mod runtime_snapshot_cli;
    #[path = "status_cli.rs"]
    mod status_cli;
    #[path = "work_unit_cli.rs"]
    mod work_unit_cli;

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
    }
}
