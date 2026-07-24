#![no_std]
//! Actions, grants, and policy interfaces.
//!
//! Domain backends, access APIs, runtime or session state, and application setup
//! belong in other crates.

extern crate alloc;

pub mod action;
/// The rules for actions.
pub mod policy;

/// The common recursive context factory.
pub trait ContextFactory {
    type Cx<'a>: Sync;
}
