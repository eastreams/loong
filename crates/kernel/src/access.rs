use loong_access::fs::access::FsAccess;
use loong_core::kernel::Kernel;

pub struct AccessCx<'a, K>
where
    K: Kernel,
{
    kernel: &'a K,
    policy_context: K::Cx<'a>,
}

impl<'a, K> AccessCx<'a, K>
where
    K: Kernel,
{
    #[inline(always)]
    #[must_use]
    pub fn new(kernel: &'a K, policy_context: K::Cx<'a>) -> Self {
        Self {
            kernel,
            policy_context,
        }
    }

    #[inline(always)]
    #[must_use]
    pub fn fs(self) -> FsAccess<'a, K> {
        FsAccess::new(self.kernel, self.policy_context)
    }
}

#[cfg(test)]
mod tests;
