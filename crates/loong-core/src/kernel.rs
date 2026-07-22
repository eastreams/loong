use crate::policy::{context::ContextFactory, engine::PolicyEngine};

pub trait Kernel<C: ContextFactory>: Sync {
    // Minimal cross-crate kernel contract for access facades. Add governance
    // methods here only when multiple concrete kernels need the same stable
    // API; audit and tool orchestration stay at their owning runtime boundary.
    fn policy_engine(&self) -> &impl PolicyEngine<C>;
}
