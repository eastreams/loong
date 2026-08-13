//! Per-session agent context storage.
//!
//! This crate owns storage mechanics.

pub mod disk;
pub mod memory;

mod log;
mod store;

pub use store::{ContextSnapshot, ContextStore};
