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

    #[path = "channel_catalog_json.rs"]
    mod channel_catalog_json;
    #[path = "channel_catalog_text.rs"]
    mod channel_catalog_text;
    #[path = "channel_plugin_bridge_json.rs"]
    mod channel_plugin_bridge_json;
    #[path = "channel_plugin_bridge_text.rs"]
    mod channel_plugin_bridge_text;
    #[path = "doctor_feishu.rs"]
    mod doctor_feishu;
    #[path = "feishu_cli.rs"]
    mod feishu_cli;
    #[path = "multi_channel_serve_cli.rs"]
    mod multi_channel_serve_cli;
}
