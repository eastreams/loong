use crate::policy::engine::PolicyEngine;

pub trait Kernel: Sync {
    /// The context
    type Cx<'a>;
    type PolicyEngine<'b>: for<'a> PolicyEngine<Cx<'a> = Self::Cx<'a>>
    where
        Self: 'b;

    // TODO: move methods here
    fn policy_engine(&self) -> &Self::PolicyEngine<'_>;
}
