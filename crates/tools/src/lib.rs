#![forbid(unsafe_code)]

//! Concrete builtin tool implementations.
//!
//! Tool traits, registries, policy, and access facades live in core/app/kernel
//! crates. This crate is intentionally narrow so adding a builtin tool does not
//! turn the implementation crate back into an abstraction layer.

pub mod file;
