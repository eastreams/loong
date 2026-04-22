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

    #[path = "managed_bridge_fixtures.rs"]
    mod managed_bridge_fixtures;
    pub(crate) use managed_bridge_fixtures::*;

    #[path = "import_cli.rs"]
    mod import_cli;
    #[path = "managed_bridge_parity.rs"]
    mod managed_bridge_parity;
    #[path = "migrate_cli.rs"]
    mod migrate_cli;
    #[path = "migration.rs"]
    mod migration;
    #[path = "onboard_cli.rs"]
    mod onboard_cli;
}
