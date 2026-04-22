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
    #[path = "ask_and_spec_cli_root.rs"]
    mod ask_and_spec_cli_root;
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
}
