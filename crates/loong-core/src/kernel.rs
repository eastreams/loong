use crate::policy::engine::PolicyEngine;

pub trait Kernel {
    type Context<'a>;
    type PolicyEngine: for<'a> PolicyEngine<Cx<'a> = Self::Context<'a>>;
}
