use loong_access::fs::access::{FsAccess, HasFsAccess};
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
}

impl<'a, K> HasFsAccess<'a, K> for AccessCx<'a, K>
where
    K: Kernel,
{
    fn fs(self) -> FsAccess<'a, K> {
        FsAccess::new(self.kernel, self.policy_context)
    }
}

#[cfg(test)]
mod tests;
