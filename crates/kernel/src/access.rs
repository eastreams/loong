use loong_access::{FsAccess, HasFsAccess};
use loong_core::policy::engine::{HasPolicyEngine, PolicyEngine};

pub struct AccessCx<'a, K>
where
    K: HasPolicyEngine,
{
    kernel: &'a K,
    policy_context: <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>,
}

impl<'a, K> AccessCx<'a, K>
where
    K: HasPolicyEngine,
{
    #[inline(always)]
    #[must_use]
    pub fn new(
        kernel: &'a K,
        policy_context: <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>,
    ) -> Self {
        Self {
            kernel,
            policy_context,
        }
    }
}

impl<'a, K> HasFsAccess<'a, K> for AccessCx<'a, K>
where
    K: HasPolicyEngine,
{
    fn fs(self) -> FsAccess<'a, K> {
        FsAccess::new(self.kernel, self.policy_context)
    }
}

#[cfg(test)]
mod tests;
