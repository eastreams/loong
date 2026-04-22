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

    #[path = "acp.rs"]
    mod acp;
    #[path = "programmatic.rs"]
    mod programmatic;
    #[path = "runtime_capability_cli.rs"]
    mod runtime_capability_cli;
    #[path = "runtime_experiment_cli.rs"]
    mod runtime_experiment_cli;
    #[path = "runtime_restore_cli.rs"]
    mod runtime_restore_cli;
    #[path = "runtime_snapshot_cli.rs"]
    mod runtime_snapshot_cli;
    #[path = "runtime_trajectory_cli.rs"]
    mod runtime_trajectory_cli;
    #[path = "spec_runtime.rs"]
    mod spec_runtime;
    mod spec_runtime_bridge;
    #[path = "trajectory_export_cli.rs"]
    mod trajectory_export_cli;
    #[path = "work_unit_cli.rs"]
    mod work_unit_cli;
}
