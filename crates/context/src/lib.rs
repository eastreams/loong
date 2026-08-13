//! Durable per-session agent context.

mod context;
mod item;
mod log;

pub use context::{ContextSnapshot, ContextStore, OpenError};
pub use item::{ContextItem, Role};
