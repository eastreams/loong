pub mod action;
/// The rules for actions.
pub mod policy;

/// The common recursive context factory.
pub trait ContextFactory {
    type Cx<'a>: Sync;
}
