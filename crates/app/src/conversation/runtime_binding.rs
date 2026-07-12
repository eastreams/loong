use loong_contracts::GovernedSessionMode;

use crate::AppContext;

#[derive(Clone, Default)]
pub enum OwnedConversationRuntimeBinding {
    Context(Box<AppContext>),
    #[default]
    AdvisoryOnly,
}

impl OwnedConversationRuntimeBinding {
    pub fn from_borrowed(binding: ConversationRuntimeBinding<'_>) -> Self {
        match binding {
            ConversationRuntimeBinding::Context(ctx) => Self::Context(Box::new(ctx.clone())),
            ConversationRuntimeBinding::AdvisoryOnly => Self::AdvisoryOnly,
        }
    }

    /// Own a context without exposing the boxed enum representation to callers.
    pub fn with_context(ctx: AppContext) -> Self {
        Self::Context(Box::new(ctx))
    }

    pub fn as_borrowed(&self) -> ConversationRuntimeBinding<'_> {
        match self {
            Self::Context(ctx) => ConversationRuntimeBinding::Context(ctx.as_ref()),
            Self::AdvisoryOnly => ConversationRuntimeBinding::AdvisoryOnly,
        }
    }

    pub fn context(&self) -> Option<&AppContext> {
        match self {
            Self::Context(ctx) => Some(ctx.as_ref()),
            Self::AdvisoryOnly => None,
        }
    }

    pub const fn is_context_bound(&self) -> bool {
        matches!(self, Self::Context(_))
    }

    pub const fn session_mode(&self) -> GovernedSessionMode {
        match self {
            Self::Context(_) => GovernedSessionMode::MutatingCapable,
            Self::AdvisoryOnly => GovernedSessionMode::AdvisoryOnly,
        }
    }

    pub const fn allows_mutation(&self) -> bool {
        matches!(self, Self::Context(_))
    }
}

#[derive(Clone, Copy, Default)]
pub enum ConversationRuntimeBinding<'a> {
    Context(&'a AppContext),
    #[default]
    AdvisoryOnly,
}

impl<'a> ConversationRuntimeBinding<'a> {
    pub fn from_optional_context(ctx: Option<&'a AppContext>) -> Self {
        match ctx {
            Some(ctx) => Self::Context(ctx),
            None => Self::AdvisoryOnly,
        }
    }

    pub fn context(self) -> Option<&'a AppContext> {
        match self {
            Self::Context(ctx) => Some(ctx),
            Self::AdvisoryOnly => None,
        }
    }

    pub const fn is_context_bound(self) -> bool {
        matches!(self, Self::Context(_))
    }

    pub const fn session_mode(self) -> GovernedSessionMode {
        match self {
            Self::Context(_) => GovernedSessionMode::MutatingCapable,
            Self::AdvisoryOnly => GovernedSessionMode::AdvisoryOnly,
        }
    }

    pub const fn allows_mutation(self) -> bool {
        matches!(self, Self::Context(_))
    }
}

#[cfg(test)]
mod tests {
    use super::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};

    #[test]
    fn owned_conversation_runtime_binding_round_trips_kernel_binding() {
        let app_ctx = crate::context::bootstrap_test_app_context(
            "owned-conversation-runtime-binding-kernel",
            60,
        )
        .expect("test app context");

        let owned = OwnedConversationRuntimeBinding::from_borrowed(
            ConversationRuntimeBinding::Context(&app_ctx),
        );

        assert!(owned.is_context_bound());
        let borrowed = owned.as_borrowed();
        assert!(borrowed.is_context_bound());
        assert_eq!(
            borrowed.session_mode(),
            ConversationRuntimeBinding::Context(&app_ctx).session_mode()
        );

        let roundtrip_ctx = owned
            .context()
            .expect("owned context binding should expose app context");
        assert_eq!(roundtrip_ctx.token(), app_ctx.token());
        assert!(std::ptr::eq(roundtrip_ctx.runtime(), app_ctx.runtime()));
    }

    #[test]
    fn owned_conversation_runtime_binding_round_trips_advisory_binding() {
        let owned = OwnedConversationRuntimeBinding::from_borrowed(
            ConversationRuntimeBinding::AdvisoryOnly,
        );

        assert!(!owned.is_context_bound());
        assert!(owned.context().is_none());
        assert_eq!(
            owned.as_borrowed().session_mode(),
            ConversationRuntimeBinding::AdvisoryOnly.session_mode()
        );
    }
}
