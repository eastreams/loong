use crate::KernelContext;

#[derive(Clone, Copy, Default)]
pub enum ProviderRuntimeBinding<'a> {
    Kernel(&'a KernelContext),
    #[default]
    Direct,
}

impl<'a> ProviderRuntimeBinding<'a> {
    pub fn kernel(kernel_ctx: &'a KernelContext) -> Self {
        Self::Kernel(kernel_ctx)
    }

    pub const fn direct() -> Self {
        Self::Direct
    }

    pub fn kernel_context(self) -> Option<&'a KernelContext> {
        match self {
            Self::Kernel(kernel_ctx) => Some(kernel_ctx),
            Self::Direct => None,
        }
    }

    pub const fn is_kernel_bound(self) -> bool {
        matches!(self, Self::Kernel(_))
    }
}
