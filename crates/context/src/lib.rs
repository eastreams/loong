//! Per-session agent context storage.

pub mod disk;
pub mod memory;

mod item;
mod log;
mod store;

pub use item::{ContextItem, Role};
pub use store::{ContextSnapshot, ContextStore};
