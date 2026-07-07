use crate::policy::{context::ActionContext, engine::PolicyEngine};

pub trait Kernel: Sync {
    /// Unified invocation context shared by action construction, policy, audit,
    /// and tool access during one plane execution.
    type Cx<'a>: ActionContext;
    type PolicyEngine: for<'a> PolicyEngine<Cx<'a> = Self::Cx<'a>>;

    // TODO: move methods here, add audit
    fn policy_engine(&self) -> &Self::PolicyEngine;
}
