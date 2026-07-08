use crate::policy::{context::ContextFactory, engine::PolicyEngine};

pub trait Kernel<C: ContextFactory>: Sync {
    type PolicyEngine: PolicyEngine<C>;

    // TODO: move methods here, add audit
    fn policy_engine(&self) -> &Self::PolicyEngine;
}
