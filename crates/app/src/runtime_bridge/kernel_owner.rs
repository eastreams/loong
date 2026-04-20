use crate::CliResult;
use crate::KernelContext;
use crate::config::LoongConfig;
use crate::context::DEFAULT_TOKEN_TTL_S;
use crate::context::bootstrap_kernel_context_with_config;
use crate::conversation::ConversationRuntimeBinding;
use crate::provider::ProviderRuntimeBinding;

/// Owns the kernel-governed runtime context for a host surface.
///
/// This seam intentionally stays small.
/// It owns `KernelContext`, and it vends borrowed bindings on demand.
/// It does not try to own config loading, runtime environment mutation,
/// or conversation-runtime construction.
#[derive(Clone)]
pub struct RuntimeKernelOwner {
    kernel_context: KernelContext,
}

impl RuntimeKernelOwner {
    #[must_use]
    pub fn new(kernel_context: KernelContext) -> Self {
        Self { kernel_context }
    }

    pub fn bootstrap(agent_id: &str, config: &LoongConfig) -> CliResult<Self> {
        let ttl_seconds = DEFAULT_TOKEN_TTL_S;
        let kernel_context = bootstrap_kernel_context_with_config(agent_id, ttl_seconds, config)?;
        let owner = Self::new(kernel_context);
        Ok(owner)
    }

    #[must_use]
    pub fn kernel_context(&self) -> &KernelContext {
        &self.kernel_context
    }

    #[must_use]
    pub fn cloned_kernel_context(&self) -> KernelContext {
        self.kernel_context.clone()
    }

    #[must_use]
    pub fn conversation_binding(&self) -> ConversationRuntimeBinding<'_> {
        let kernel_context = self.kernel_context();
        ConversationRuntimeBinding::kernel(kernel_context)
    }

    #[must_use]
    pub fn provider_binding(&self) -> ProviderRuntimeBinding<'_> {
        let kernel_context = self.kernel_context();
        ProviderRuntimeBinding::kernel(kernel_context)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::AuditMode;
    use crate::config::LoongConfig;
    use crate::runtime_bridge::RuntimeKernelOwner;

    #[test]
    fn runtime_kernel_owner_bootstrap_provides_kernel_bound_bindings() {
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::InMemory;

        let owner = RuntimeKernelOwner::bootstrap("runtime-kernel-owner-test", &config)
            .expect("bootstrap runtime kernel owner");

        let conversation_binding = owner.conversation_binding();
        let provider_binding = owner.provider_binding();

        assert!(conversation_binding.is_kernel_bound());
        assert!(provider_binding.is_kernel_bound());
        assert_eq!(
            owner.kernel_context().agent_id(),
            "runtime-kernel-owner-test"
        );
    }

    #[test]
    fn runtime_kernel_owner_cloned_context_preserves_identity() {
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::InMemory;

        let owner = RuntimeKernelOwner::bootstrap("runtime-kernel-owner-clone", &config)
            .expect("bootstrap runtime kernel owner");

        let cloned_kernel_context = owner.cloned_kernel_context();
        let original_token_id = owner.kernel_context().token.token_id.clone();
        let cloned_token_id = cloned_kernel_context.token.token_id.clone();

        assert_eq!(original_token_id, cloned_token_id);
        assert_eq!(
            owner.kernel_context().pack_id(),
            cloned_kernel_context.pack_id()
        );
        assert_eq!(
            owner.kernel_context().agent_id(),
            cloned_kernel_context.agent_id()
        );
    }
}
