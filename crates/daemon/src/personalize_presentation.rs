use crate::onboard_cli::SelectOption;

pub(crate) const PERSONALIZE_COMMAND_ABOUT: &str =
    "Teach Loong your working style for future sessions";
pub(crate) const PERSONALIZE_COMMAND_LONG_ABOUT: &str = "Teach Loong your working style for future sessions.\n\nThis command stores advisory preferences such as preferred name, response density, initiative level, and standing boundaries. Rerun it any time to update or clear saved preferences. It does not replace runtime identity files, and it does not change the primary setup path. If you do not have a config yet, run `loong onboard` first.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonalizePromptKind {
    PreferredName,
    StandingBoundaries,
    Timezone,
    Locale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonalizeSelectKind {
    ResponseDensity,
    InitiativeLevel,
    ReviewAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonalizeReviewChoiceKind {
    Save,
    Skip,
    Suppress,
}

pub(crate) const fn personalize_action_label() -> &'static str {
    "teach Loong your working style"
}

pub(crate) const fn personalize_action_title() -> &'static str {
    "Teach Loong your working style"
}

pub(crate) const fn personalize_review_intro() -> &'static str {
    "Review how Loong will work with you:"
}

pub(crate) const fn personalize_prompt_label(kind: PersonalizePromptKind) -> &'static str {
    match kind {
        PersonalizePromptKind::PreferredName => "How should Loong address you? (optional)",
        PersonalizePromptKind::StandingBoundaries => {
            "Any standing boundaries Loong should keep in mind? (optional)"
        }
        PersonalizePromptKind::Timezone => "Which timezone should Loong assume? (optional)",
        PersonalizePromptKind::Locale => "Which locale should Loong default to? (optional)",
    }
}

pub(crate) const fn personalize_select_label(kind: PersonalizeSelectKind) -> &'static str {
    match kind {
        PersonalizeSelectKind::ResponseDensity => "How detailed should Loong usually be?",
        PersonalizeSelectKind::InitiativeLevel => "How proactive should Loong be?",
        PersonalizeSelectKind::ReviewAction => "What should Loong do with this draft?",
    }
}

pub(crate) const fn personalize_review_choice_label(
    kind: PersonalizeReviewChoiceKind,
) -> &'static str {
    match kind {
        PersonalizeReviewChoiceKind::Save => "use this draft",
        PersonalizeReviewChoiceKind::Skip => "not now",
        PersonalizeReviewChoiceKind::Suppress => "stop suggesting this",
    }
}

pub(crate) const fn personalize_review_choice_description(
    kind: PersonalizeReviewChoiceKind,
) -> &'static str {
    match kind {
        PersonalizeReviewChoiceKind::Save => "save these preferences for future sessions",
        PersonalizeReviewChoiceKind::Skip => "leave the current config untouched",
        PersonalizeReviewChoiceKind::Suppress => {
            "stop proactive suggestions without saving this draft; keep any existing saved preferences"
        }
    }
}

pub(crate) fn response_density_select_options(has_current_value: bool) -> Vec<SelectOption> {
    let mut options = vec![
        SelectOption {
            label: "concise".to_owned(),
            slug: "concise".to_owned(),
            description: "keep responses brief and tightly scoped".to_owned(),
            recommended: false,
        },
        SelectOption {
            label: "balanced".to_owned(),
            slug: "balanced".to_owned(),
            description: "balance speed, clarity, and context".to_owned(),
            recommended: true,
        },
        SelectOption {
            label: "thorough".to_owned(),
            slug: "thorough".to_owned(),
            description: "include deeper context and reasoning when useful".to_owned(),
            recommended: false,
        },
    ];

    if has_current_value {
        options.push(SelectOption {
            label: "clear current value".to_owned(),
            slug: "clear".to_owned(),
            description: "remove the saved response density preference".to_owned(),
            recommended: false,
        });
    } else {
        options.push(SelectOption {
            label: "leave unset".to_owned(),
            slug: "unset".to_owned(),
            description: "do not save a response density preference yet".to_owned(),
            recommended: false,
        });
    }

    options
}

pub(crate) fn initiative_level_select_options(has_current_value: bool) -> Vec<SelectOption> {
    let mut options = vec![
        SelectOption {
            label: "ask before acting".to_owned(),
            slug: "ask_before_acting".to_owned(),
            description: "confirm before taking non-trivial action".to_owned(),
            recommended: false,
        },
        SelectOption {
            label: "balanced".to_owned(),
            slug: "balanced".to_owned(),
            description: "default initiative with selective confirmation".to_owned(),
            recommended: true,
        },
        SelectOption {
            label: "high initiative".to_owned(),
            slug: "high_initiative".to_owned(),
            description: "move forward proactively unless risk is high".to_owned(),
            recommended: false,
        },
    ];

    if has_current_value {
        options.push(SelectOption {
            label: "clear current value".to_owned(),
            slug: "clear".to_owned(),
            description: "remove the saved initiative preference".to_owned(),
            recommended: false,
        });
    } else {
        options.push(SelectOption {
            label: "leave unset".to_owned(),
            slug: "unset".to_owned(),
            description: "do not save an initiative preference yet".to_owned(),
            recommended: false,
        });
    }

    options
}

pub(crate) fn review_action_select_options() -> Vec<SelectOption> {
    vec![
        SelectOption {
            label: personalize_review_choice_label(PersonalizeReviewChoiceKind::Save).to_owned(),
            slug: "save".to_owned(),
            description: personalize_review_choice_description(PersonalizeReviewChoiceKind::Save)
                .to_owned(),
            recommended: true,
        },
        SelectOption {
            label: personalize_review_choice_label(PersonalizeReviewChoiceKind::Skip).to_owned(),
            slug: "skip".to_owned(),
            description: personalize_review_choice_description(PersonalizeReviewChoiceKind::Skip)
                .to_owned(),
            recommended: false,
        },
        SelectOption {
            label: personalize_review_choice_label(PersonalizeReviewChoiceKind::Suppress)
                .to_owned(),
            slug: "suppress".to_owned(),
            description: personalize_review_choice_description(
                PersonalizeReviewChoiceKind::Suppress,
            )
            .to_owned(),
            recommended: false,
        },
    ]
}

pub(crate) const fn personalize_skip_message() -> &'static str {
    "No changes saved."
}

pub(crate) const fn personalize_suppressed_recovery_guidance() -> &'static str {
    "Personalize suggestions are currently suppressed. Saving here will re-enable them."
}

pub(crate) const fn personalize_memory_profile_upgrade_prompt() -> &'static str {
    "Let Loong surface these preferences in Session Profile by upgrading memory profile?"
}

pub(crate) const fn personalize_memory_profile_upgraded_message() -> &'static str {
    "Memory profile upgraded to profile_plus_window so Loong can surface these preferences in Session Profile."
}

pub(crate) const fn personalize_memory_profile_deferred_message() -> &'static str {
    "Saved these preferences without changing memory.profile; Loong will surface them once profile_plus_window is enabled."
}

pub(crate) fn personalize_saved_message(path: &str) -> String {
    format!("Saved how Loong should work with you to {path}.")
}

pub(crate) fn personalize_cleared_message(path: &str) -> String {
    format!("Cleared how Loong should work with you from {path}.")
}

pub(crate) fn personalize_suppressed_message(path: &str) -> String {
    format!("Stopped future personalize suggestions in {path}.")
}
