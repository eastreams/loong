use loong_contracts::GovernedSessionMode;

use crate::KernelContext;

#[derive(Clone, Copy, Default)]
pub enum ProviderRuntimeBinding<'a> {
    Kernel(&'a KernelContext),
    #[default]
    AdvisoryOnly,
}

impl<'a> ProviderRuntimeBinding<'a> {
    pub fn kernel(kernel_ctx: &'a KernelContext) -> Self {
        Self::Kernel(kernel_ctx)
    }

    pub const fn advisory_only() -> Self {
        Self::AdvisoryOnly
    }

    pub const fn direct() -> Self {
        Self::AdvisoryOnly
    }

    pub fn kernel_context(self) -> Option<&'a KernelContext> {
        match self {
            Self::Kernel(kernel_ctx) => Some(kernel_ctx),
            Self::AdvisoryOnly => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kernel(_) => "kernel",
            Self::AdvisoryOnly => "advisory_only",
        }
    }

    pub const fn is_kernel_bound(self) -> bool {
        matches!(self, Self::Kernel(_))
    }

    pub const fn session_mode(self) -> GovernedSessionMode {
        match self {
            Self::Kernel(_) => GovernedSessionMode::MutatingCapable,
            Self::AdvisoryOnly => GovernedSessionMode::AdvisoryOnly,
        }
    }

    pub const fn allows_mutation(self) -> bool {
        matches!(self, Self::Kernel(_))
    }
}

#[cfg(test)]
mod tests {
    use crate::context::bootstrap_test_kernel_context;

    use super::ProviderRuntimeBinding;

    #[test]
    fn provider_runtime_binding_labels_are_stable() {
        let kernel_context = bootstrap_test_kernel_context("runtime-binding-test", 60)
            .expect("kernel context should bootstrap");
        let binding = ProviderRuntimeBinding::kernel(&kernel_context);

        assert_eq!(ProviderRuntimeBinding::direct().as_str(), "advisory_only");
        assert_eq!(binding.as_str(), "kernel");
    }
}
