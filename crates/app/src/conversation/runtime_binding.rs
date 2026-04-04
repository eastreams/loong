use loongclaw_contracts::GovernedSessionMode;

use crate::KernelContext;

#[derive(Clone, Default)]
pub enum OwnedConversationRuntimeBinding {
    Kernel(KernelContext),
    #[default]
    Direct,
}

impl OwnedConversationRuntimeBinding {
    pub fn from_borrowed(binding: ConversationRuntimeBinding<'_>) -> Self {
        match binding {
            ConversationRuntimeBinding::Kernel(kernel_ctx) => Self::Kernel(kernel_ctx.clone()),
            ConversationRuntimeBinding::Direct => Self::Direct,
        }
    }

    pub fn kernel(kernel_ctx: KernelContext) -> Self {
        Self::Kernel(kernel_ctx)
    }

    pub const fn advisory_only() -> Self {
        Self::Direct
    }

    pub const fn direct() -> Self {
        Self::Direct
    }

    pub fn as_borrowed(&self) -> ConversationRuntimeBinding<'_> {
        match self {
            Self::Kernel(kernel_ctx) => ConversationRuntimeBinding::Kernel(kernel_ctx),
            Self::Direct => ConversationRuntimeBinding::Direct,
        }
    }

    pub fn kernel_context(&self) -> Option<&KernelContext> {
        match self {
            Self::Kernel(kernel_ctx) => Some(kernel_ctx),
            Self::Direct => None,
        }
    }

    pub const fn is_kernel_bound(&self) -> bool {
        matches!(self, Self::Kernel(_))
    }

    pub const fn session_mode(&self) -> GovernedSessionMode {
        match self {
            Self::Kernel(_) => GovernedSessionMode::MutatingCapable,
            Self::Direct => GovernedSessionMode::AdvisoryOnly,
        }
    }

    pub const fn allows_mutation(&self) -> bool {
        matches!(self, Self::Kernel(_))
    }
}

#[derive(Clone, Copy, Default)]
pub enum ConversationRuntimeBinding<'a> {
    Kernel(&'a KernelContext),
    #[default]
    Direct,
}

impl<'a> ConversationRuntimeBinding<'a> {
    pub fn from_optional_kernel_context(kernel_ctx: Option<&'a KernelContext>) -> Self {
        match kernel_ctx {
            Some(kernel_ctx) => Self::Kernel(kernel_ctx),
            None => Self::Direct,
        }
    }

    pub fn kernel(kernel_ctx: &'a KernelContext) -> Self {
        Self::Kernel(kernel_ctx)
    }

    pub const fn advisory_only() -> Self {
        Self::Direct
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

    pub const fn session_mode(self) -> GovernedSessionMode {
        match self {
            Self::Kernel(_) => GovernedSessionMode::MutatingCapable,
            Self::Direct => GovernedSessionMode::AdvisoryOnly,
        }
    }

    pub const fn allows_mutation(self) -> bool {
        matches!(self, Self::Kernel(_))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};

    #[test]
    fn owned_conversation_runtime_binding_round_trips_kernel_binding() {
        let kernel_ctx = crate::context::bootstrap_test_kernel_context(
            "owned-conversation-runtime-binding-kernel",
            60,
        )
        .expect("test kernel context");

        let owned = OwnedConversationRuntimeBinding::from_borrowed(
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        );

        assert!(owned.is_kernel_bound());
        let borrowed = owned.as_borrowed();
        assert!(borrowed.is_kernel_bound());
        assert_eq!(
            borrowed.session_mode(),
            ConversationRuntimeBinding::kernel(&kernel_ctx).session_mode()
        );

        let roundtrip_ctx = owned
            .kernel_context()
            .expect("owned kernel binding should expose kernel context");
        assert_eq!(roundtrip_ctx.token, kernel_ctx.token);
        assert!(Arc::ptr_eq(&roundtrip_ctx.kernel, &kernel_ctx.kernel));
    }

    #[test]
    fn owned_conversation_runtime_binding_round_trips_advisory_binding() {
        let owned = OwnedConversationRuntimeBinding::from_borrowed(
            ConversationRuntimeBinding::advisory_only(),
        );

        assert!(!owned.is_kernel_bound());
        assert!(owned.kernel_context().is_none());
        assert_eq!(
            owned.as_borrowed().session_mode(),
            ConversationRuntimeBinding::advisory_only().session_mode()
        );
    }
}
