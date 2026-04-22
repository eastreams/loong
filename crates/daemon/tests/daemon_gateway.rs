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

    #[path = "architecture.rs"]
    mod architecture;
    #[path = "gateway_api_acp.rs"]
    mod gateway_api_acp;
    #[path = "gateway_api_events.rs"]
    mod gateway_api_events;
    #[path = "gateway_api_health.rs"]
    mod gateway_api_health;
    #[path = "gateway_api_turn.rs"]
    mod gateway_api_turn;
    #[path = "gateway_owner_state.rs"]
    mod gateway_owner_state;
    #[path = "gateway_read_models.rs"]
    mod gateway_read_models;
    #[path = "logging.rs"]
    mod logging;
}
