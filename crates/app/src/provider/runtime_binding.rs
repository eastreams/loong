use loong_contracts::GovernedSessionMode;

use crate::AppContext;

#[derive(Clone, Copy, Default)]
pub enum ProviderRuntimeBinding<'a> {
    Context(&'a AppContext),
    #[default]
    AdvisoryOnly,
}

impl<'a> ProviderRuntimeBinding<'a> {
    pub fn context(self) -> Option<&'a AppContext> {
        match self {
            Self::Context(ctx) => Some(ctx),
            Self::AdvisoryOnly => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Context(_) => "context",
            Self::AdvisoryOnly => "advisory_only",
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
    use crate::context::bootstrap_test_app_context;

    use super::ProviderRuntimeBinding;

    #[test]
    fn provider_runtime_binding_labels_are_stable() {
        let app_context = bootstrap_test_app_context("runtime-binding-test", 60)
            .expect("app context should bootstrap");
        let binding = ProviderRuntimeBinding::Context(&app_context);

        assert_eq!(
            ProviderRuntimeBinding::AdvisoryOnly.as_str(),
            "advisory_only"
        );
        assert_eq!(binding.as_str(), "context");
    }
}
