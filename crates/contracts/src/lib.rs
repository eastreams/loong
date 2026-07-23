//! Shared data types used across Loong crates.
//!
//! This crate may define serialization, schemas, and data errors. Keep I/O,
//! async runtime code, policy evaluation, and application state out of it.

pub mod capability;
pub mod policy;
pub mod tool;
