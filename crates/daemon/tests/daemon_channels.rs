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

    #[path = "doctor_feishu.rs"]
    mod doctor_feishu;
    #[path = "feishu_cli.rs"]
    mod feishu_cli;
    #[path = "multi_channel_serve_cli.rs"]
    mod multi_channel_serve_cli;
}
