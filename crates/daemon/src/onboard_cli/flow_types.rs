#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnboardHeaderStyle {
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuidedPromptPath {
    NativePromptPack,
    InlineOverride,
}

impl GuidedPromptPath {
    pub(crate) const fn total_steps(self) -> usize {
        match self {
            GuidedPromptPath::NativePromptPack => 7,
            GuidedPromptPath::InlineOverride => 6,
        }
    }

    pub(crate) const fn index(self, step: GuidedOnboardStep) -> usize {
        match (self, step) {
            (_, GuidedOnboardStep::Provider) => 1,
            (_, GuidedOnboardStep::Model) => 2,
            (_, GuidedOnboardStep::CredentialEnv) => 3,
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::PromptCustomization) => 4,
            (_, GuidedOnboardStep::WebSearchProvider) => match self {
                GuidedPromptPath::NativePromptPack => 5,
                GuidedPromptPath::InlineOverride => 4,
            },
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::Review) => 6,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::PromptCustomization) => 4,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::Review) => 5,
        }
    }

    pub(crate) const fn label(self, step: GuidedOnboardStep) -> &'static str {
        match step {
            GuidedOnboardStep::Provider => "provider",
            GuidedOnboardStep::Model => "model",
            GuidedOnboardStep::CredentialEnv => "credential source",
            GuidedOnboardStep::PromptCustomization => match self {
                GuidedPromptPath::NativePromptPack => "prompt addendum",
                GuidedPromptPath::InlineOverride => "system prompt",
            },
            GuidedOnboardStep::WebSearchProvider => "web search",
            GuidedOnboardStep::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuidedOnboardStep {
    Provider,
    Model,
    CredentialEnv,
    PromptCustomization,
    WebSearchProvider,
    Review,
}

impl GuidedOnboardStep {
    pub(crate) fn progress_line(self, path: GuidedPromptPath) -> String {
        format!(
            "step {} of {} · {}",
            path.index(self),
            path.total_steps(),
            path.label(self)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewFlowStyle {
    Guided(GuidedPromptPath),
    QuickCurrentSetup,
    QuickDetectedSetup,
}

impl ReviewFlowStyle {
    pub(crate) const fn review_kind(self) -> crate::onboard::presentation::ReviewFlowKind {
        match self {
            ReviewFlowStyle::Guided(_) => crate::onboard::presentation::ReviewFlowKind::Guided,
            ReviewFlowStyle::QuickCurrentSetup => {
                crate::onboard::presentation::ReviewFlowKind::QuickCurrentSetup
            }
            ReviewFlowStyle::QuickDetectedSetup => {
                crate::onboard::presentation::ReviewFlowKind::QuickDetectedSetup
            }
        }
    }

    pub(crate) fn progress_line(self) -> String {
        match self {
            ReviewFlowStyle::Guided(prompt_path) => {
                GuidedOnboardStep::Review.progress_line(prompt_path)
            }
            ReviewFlowStyle::QuickCurrentSetup | ReviewFlowStyle::QuickDetectedSetup => {
                crate::onboard::presentation::review_flow_copy(self.review_kind())
                    .progress_line
                    .to_owned()
            }
        }
    }

    pub(crate) const fn header_subtitle(self) -> &'static str {
        crate::onboard::presentation::review_flow_copy(self.review_kind()).header_subtitle
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardScreenOption {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) detail_lines: Vec<String>,
    pub(crate) recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebSearchCredentialSelection {
    KeepCurrent,
    ClearConfigured,
    UseEnv(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartingPointFitHint {
    pub(crate) key: &'static str,
    pub(crate) detail: String,
    pub(crate) domain: Option<crate::migration::SetupDomainKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnboardShortcutKind {
    CurrentSetup,
    DetectedSetup,
}

impl OnboardShortcutKind {
    pub(crate) const fn presentation_kind(self) -> crate::onboard::presentation::ShortcutKind {
        match self {
            OnboardShortcutKind::CurrentSetup => {
                crate::onboard::presentation::ShortcutKind::CurrentSetup
            }
            OnboardShortcutKind::DetectedSetup => {
                crate::onboard::presentation::ShortcutKind::DetectedSetup
            }
        }
    }

    pub(crate) const fn review_flow_style(self) -> ReviewFlowStyle {
        match self {
            OnboardShortcutKind::CurrentSetup => ReviewFlowStyle::QuickCurrentSetup,
            OnboardShortcutKind::DetectedSetup => ReviewFlowStyle::QuickDetectedSetup,
        }
    }

    pub(crate) const fn subtitle(self) -> &'static str {
        crate::onboard::presentation::shortcut_copy(self.presentation_kind()).subtitle
    }

    pub(crate) const fn title(self) -> &'static str {
        crate::onboard::presentation::shortcut_copy(self.presentation_kind()).title
    }

    pub(crate) const fn summary_line(self) -> &'static str {
        crate::onboard::presentation::shortcut_copy(self.presentation_kind()).summary_line
    }

    pub(crate) const fn primary_label(self) -> &'static str {
        crate::onboard::presentation::shortcut_copy(self.presentation_kind()).primary_label
    }

    pub(crate) const fn default_choice_description(self) -> &'static str {
        crate::onboard::presentation::shortcut_copy(self.presentation_kind())
            .default_choice_description
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnboardShortcutChoice {
    UseShortcut,
    AdjustSettings,
}
