pub mod channel;
pub mod chat;
pub mod config;
pub mod context;
pub mod conversation;
pub mod memory;
pub mod provider;
pub mod tools;

pub use context::KernelContext;

/// Result type for MVP CLI operations.
pub type CliResult<T> = Result<T, String>;
