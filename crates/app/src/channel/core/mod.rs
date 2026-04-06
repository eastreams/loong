// crates/app/src/channel/core/mod.rs

pub(in crate::channel) mod http;
pub(in crate::channel) mod types;
pub(in crate::channel) mod webhook_auth;

pub use http::{ChannelOutboundHttpPolicy, SsrfSafeResolver};
pub use types::*;
pub use webhook_auth::build_webhook_auth_header_from_parts;
