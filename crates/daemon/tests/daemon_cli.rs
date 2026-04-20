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

    #[path = "channel_surfaces_json.rs"]
    mod channel_surfaces_json;
    #[path = "channel_surfaces_text.rs"]
    mod channel_surfaces_text;
    #[path = "latest_selector_process_support.rs"]
    mod latest_selector_process_support;
    #[path = "memory_surfaces.rs"]
    mod memory_surfaces;
    #[path = "root_cli.rs"]
    mod root_cli;

    #[path = "ask_cli.rs"]
    mod ask_cli;
    #[path = "chat_cli.rs"]
    mod chat_cli;
    #[path = "cli_tests.rs"]
    mod cli_tests;
    #[path = "mcp.rs"]
    mod mcp;
    #[path = "memory_context_benchmark_cli.rs"]
    mod memory_context_benchmark_cli;
    #[path = "personalize_cli.rs"]
    mod personalize_cli;
    #[path = "plugins_cli.rs"]
    mod plugins_cli;
    #[path = "session_search_cli.rs"]
    mod session_search_cli;
    #[path = "sessions_cli.rs"]
    mod sessions_cli;
    #[path = "skills_cli.rs"]
    mod skills_cli;
    #[path = "status_cli.rs"]
    mod status_cli;
    #[path = "tasks_cli.rs"]
    mod tasks_cli;
}
