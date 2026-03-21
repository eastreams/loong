pub mod acp;
pub mod channel;
#[cfg(feature = "feishu-integration")]
pub use channel::feishu;
pub mod chat;
pub mod config;
pub mod context;
pub mod conversation;
pub mod memory;
pub mod migration;
pub mod presentation;
pub mod prompt;
pub mod provider;
pub mod runtime_env;
pub mod session;
pub mod tools;

mod process_env;
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::missing_panics_doc
)]
#[doc(hidden)]
pub mod test_support;

pub use context::KernelContext;
/// Result type for MVP CLI operations.
pub type CliResult<T> = Result<T, String>;
