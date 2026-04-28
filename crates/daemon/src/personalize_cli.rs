#[cfg(test)]
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use loong_app as mvp;
use loong_spec::CliResult;
use time::OffsetDateTime;

use crate::operator_prompt::{
    OPERATOR_CLEAR_INPUT_TOKEN, OperatorPromptUi, SelectInteractionMode, SelectOption,
    StdioOperatorUi, prompt_optional_operator_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonalizeReviewAction {
    Save,
    SkipForNow,
    SuppressFutureSuggestions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersonalizationDraft {
    preferred_name: Option<String>,
    response_density: Option<mvp::config::ResponseDensity>,
    initiative_level: Option<mvp::config::InitiativeLevel>,
    standing_boundaries: Option<String>,
    timezone: Option<String>,
    locale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersonalizeCliOutcome {
    Saved { upgraded_memory_profile: bool },
    Skipped,
    Suppressed,
}

const PREFERRED_NAME_PROMPT: &str = "How should Loong address you? (optional)";
const STANDING_BOUNDARIES_PROMPT: &str =
    "Any standing boundaries Loong should keep in mind? (optional)";
const TIMEZONE_PROMPT: &str = "Which timezone should Loong assume? (optional)";
const LOCALE_PROMPT: &str = "Which locale should Loong default to? (optional)";
const RESPONSE_DENSITY_PROMPT: &str = "How detailed should Loong usually be?";
const INITIATIVE_LEVEL_PROMPT: &str = "How proactive should Loong be?";
const REVIEW_ACTION_PROMPT: &str = "What should Loong do with this draft?";

pub fn run_personalize_cli(config_path: Option<&str>) -> CliResult<()> {
    let mut ui = StdioOperatorUi::default();
    let now = OffsetDateTime::now_utc();
    let _outcome = run_personalize_cli_with_ui(config_path, &mut ui, now)?;
    Ok(())
}

pub(crate) fn run_personalize_cli_with_ui(
    config_path: Option<&str>,
    ui: &mut impl OperatorPromptUi,
    now: OffsetDateTime,
) -> CliResult<PersonalizeCliOutcome> {
    let load_result = mvp::config::load(config_path)?;
    let (resolved_path, mut config) = load_result;
    let existing_personalization = config.memory.trimmed_personalization();
    print_suppressed_recovery_guidance(ui, existing_personalization.as_ref())?;
    let draft = collect_personalization_draft(ui, existing_personalization.as_ref())?;
    let review_action = select_review_action(ui, &draft)?;

    match review_action {
        PersonalizeReviewAction::Save => save_personalization(
            ui,
            &resolved_path,
            &mut config,
            existing_personalization.as_ref(),
            draft,
            now,
        ),
        PersonalizeReviewAction::SkipForNow => {
            ui.print_line("No changes saved.")?;
            Ok(PersonalizeCliOutcome::Skipped)
        }
        PersonalizeReviewAction::SuppressFutureSuggestions => suppress_personalization(
            ui,
            &resolved_path,
            &mut config,
            existing_personalization.as_ref(),
            now,
        ),
    }
}

fn print_suppressed_recovery_guidance(
    ui: &mut impl OperatorPromptUi,
    existing_personalization: Option<&mvp::config::PersonalizationConfig>,
) -> CliResult<()> {
    let Some(personalization) = existing_personalization else {
        return Ok(());
    };

    if !personalization.suppresses_suggestions() {
        return Ok(());
    }

    ui.print_line(
        "Personalize suggestions are currently suppressed. Saving preferences here will re-enable them.",
    )?;

    Ok(())
}

fn collect_personalization_draft(
    ui: &mut impl OperatorPromptUi,
    existing_personalization: Option<&mvp::config::PersonalizationConfig>,
) -> CliResult<PersonalizationDraft> {
    let preferred_name_default = existing_personalization
        .and_then(|personalization| personalization.preferred_name.as_deref());
    let preferred_name = prompt_optional_text(ui, PREFERRED_NAME_PROMPT, preferred_name_default)?;

    let response_density_default =
        existing_personalization.and_then(|personalization| personalization.response_density);
    let response_density = select_response_density(ui, response_density_default)?;

    let initiative_level_default =
        existing_personalization.and_then(|personalization| personalization.initiative_level);
    let initiative_level = select_initiative_level(ui, initiative_level_default)?;

    let standing_boundaries_default = existing_personalization
        .and_then(|personalization| personalization.standing_boundaries.as_deref());
    let standing_boundaries =
        prompt_optional_text(ui, STANDING_BOUNDARIES_PROMPT, standing_boundaries_default)?;

    let timezone_default =
        existing_personalization.and_then(|personalization| personalization.timezone.as_deref());
    let timezone = prompt_optional_text(ui, TIMEZONE_PROMPT, timezone_default)?;

    let locale_default =
        existing_personalization.and_then(|personalization| personalization.locale.as_deref());
    let locale = prompt_optional_text(ui, LOCALE_PROMPT, locale_default)?;

    Ok(PersonalizationDraft {
        preferred_name,
        response_density,
        initiative_level,
        standing_boundaries,
        timezone,
        locale,
    })
}

fn prompt_optional_text(
    ui: &mut impl OperatorPromptUi,
    label: &str,
    current_value: Option<&str>,
) -> CliResult<Option<String>> {
    if let Some(default_value) = current_value {
        let current_value_line = format!("Current value: {default_value}");
        let clear_hint_line =
            format!("Press Enter to keep it, or type {OPERATOR_CLEAR_INPUT_TOKEN} to clear it.");
        ui.print_line(current_value_line.as_str())?;
        ui.print_line(clear_hint_line.as_str())?;
    }

    let selected_value = prompt_optional_operator_text(ui, label, current_value)?;

    Ok(selected_value)
}

fn select_response_density(
    ui: &mut impl OperatorPromptUi,
    current_value: Option<mvp::config::ResponseDensity>,
) -> CliResult<Option<mvp::config::ResponseDensity>> {
    let mut options = Vec::new();
    let concise_option_index = options.len();
    let concise_option = SelectOption {
        label: "concise".to_owned(),
        slug: "concise".to_owned(),
        description: "keep responses brief and tightly scoped".to_owned(),
        recommended: false,
    };
    options.push(concise_option);
    let balanced_option_index = options.len();
    let balanced_option = SelectOption {
        label: "balanced".to_owned(),
        slug: "balanced".to_owned(),
        description: "balance speed, clarity, and context".to_owned(),
        recommended: true,
    };
    options.push(balanced_option);
    let thorough_option_index = options.len();
    let thorough_option = SelectOption {
        label: "thorough".to_owned(),
        slug: "thorough".to_owned(),
        description: "include deeper context and reasoning when useful".to_owned(),
        recommended: false,
    };
    options.push(thorough_option);
    let unset_option_index = if current_value.is_none() {
        let unset_option = SelectOption {
            label: "leave unset".to_owned(),
            slug: "unset".to_owned(),
            description: "do not save a response density preference yet".to_owned(),
            recommended: false,
        };
        options.push(unset_option);
        Some(options.len() - 1)
    } else {
        None
    };
    let clear_option_index = if current_value.is_some() {
        let clear_option = SelectOption {
            label: "clear current value".to_owned(),
            slug: "clear".to_owned(),
            description: "remove the saved response density preference".to_owned(),
            recommended: false,
        };
        options.push(clear_option);
        Some(options.len() - 1)
    } else {
        None
    };
    let default_index = match current_value {
        Some(mvp::config::ResponseDensity::Concise) => Some(concise_option_index),
        Some(mvp::config::ResponseDensity::Balanced) => Some(balanced_option_index),
        Some(mvp::config::ResponseDensity::Thorough) => Some(thorough_option_index),
        None => unset_option_index,
    };
    let selected_index = ui.select_one(
        RESPONSE_DENSITY_PROMPT,
        &options,
        default_index,
        SelectInteractionMode::List,
    )?;
    if Some(selected_index) == unset_option_index {
        return Ok(None);
    }
    if Some(selected_index) == clear_option_index {
        return Ok(None);
    }
    if selected_index == concise_option_index {
        return Ok(Some(mvp::config::ResponseDensity::Concise));
    }
    if selected_index == balanced_option_index {
        return Ok(Some(mvp::config::ResponseDensity::Balanced));
    }
    if selected_index == thorough_option_index {
        return Ok(Some(mvp::config::ResponseDensity::Thorough));
    }

    Err("response density selection out of range".to_owned())
}

fn select_initiative_level(
    ui: &mut impl OperatorPromptUi,
    current_value: Option<mvp::config::InitiativeLevel>,
) -> CliResult<Option<mvp::config::InitiativeLevel>> {
    let mut options = Vec::new();
    let ask_before_acting_option_index = options.len();
    let ask_before_acting_option = SelectOption {
        label: "ask before acting".to_owned(),
        slug: "ask_before_acting".to_owned(),
        description: "confirm before taking non-trivial action".to_owned(),
        recommended: false,
    };
    options.push(ask_before_acting_option);
    let balanced_option_index = options.len();
    let balanced_option = SelectOption {
        label: "balanced".to_owned(),
        slug: "balanced".to_owned(),
        description: "default initiative with selective confirmation".to_owned(),
        recommended: true,
    };
    options.push(balanced_option);
    let high_initiative_option_index = options.len();
    let high_initiative_option = SelectOption {
        label: "high initiative".to_owned(),
        slug: "high_initiative".to_owned(),
        description: "move forward proactively unless risk is high".to_owned(),
        recommended: false,
    };
    options.push(high_initiative_option);
    let unset_option_index = if current_value.is_none() {
        let unset_option = SelectOption {
            label: "leave unset".to_owned(),
            slug: "unset".to_owned(),
            description: "do not save an initiative preference yet".to_owned(),
            recommended: false,
        };
        options.push(unset_option);
        Some(options.len() - 1)
    } else {
        None
    };
    let clear_option_index = if current_value.is_some() {
        let clear_option = SelectOption {
            label: "clear current value".to_owned(),
            slug: "clear".to_owned(),
            description: "remove the saved initiative preference".to_owned(),
            recommended: false,
        };
        options.push(clear_option);
        Some(options.len() - 1)
    } else {
        None
    };
    let default_index = match current_value {
        Some(mvp::config::InitiativeLevel::AskBeforeActing) => Some(ask_before_acting_option_index),
        Some(mvp::config::InitiativeLevel::Balanced) => Some(balanced_option_index),
        Some(mvp::config::InitiativeLevel::HighInitiative) => Some(high_initiative_option_index),
        None => unset_option_index,
    };
    let selected_index = ui.select_one(
        INITIATIVE_LEVEL_PROMPT,
        &options,
        default_index,
        SelectInteractionMode::List,
    )?;
    if Some(selected_index) == unset_option_index {
        return Ok(None);
    }
    if Some(selected_index) == clear_option_index {
        return Ok(None);
    }
    if selected_index == ask_before_acting_option_index {
        return Ok(Some(mvp::config::InitiativeLevel::AskBeforeActing));
    }
    if selected_index == balanced_option_index {
        return Ok(Some(mvp::config::InitiativeLevel::Balanced));
    }
    if selected_index == high_initiative_option_index {
        return Ok(Some(mvp::config::InitiativeLevel::HighInitiative));
    }

    Err("initiative level selection out of range".to_owned())
}

fn select_review_action(
    ui: &mut impl OperatorPromptUi,
    draft: &PersonalizationDraft,
) -> CliResult<PersonalizeReviewAction> {
    let review_lines = render_review_lines(draft);
    for line in review_lines {
        ui.print_line(line.as_str())?;
    }

    let options = vec![
        SelectOption {
            label: "use this draft".to_owned(),
            slug: "save".to_owned(),
            description: "save these preferences for future sessions".to_owned(),
            recommended: true,
        },
        SelectOption {
            label: "not now".to_owned(),
            slug: "skip".to_owned(),
            description: "leave the current config untouched".to_owned(),
            recommended: false,
        },
        SelectOption {
            label: "stop suggesting this".to_owned(),
            slug: "suppress".to_owned(),
            description:
                "stop proactive suggestions without saving this draft; keep any existing saved preferences"
                    .to_owned(),
            recommended: false,
        },
    ];
    let selected_index = ui.select_one(
        REVIEW_ACTION_PROMPT,
        &options,
        Some(0),
        SelectInteractionMode::List,
    )?;

    match selected_index {
        0 => Ok(PersonalizeReviewAction::Save),
        1 => Ok(PersonalizeReviewAction::SkipForNow),
        2 => Ok(PersonalizeReviewAction::SuppressFutureSuggestions),
        _ => Err("review action selection out of range".to_owned()),
    }
}

fn render_review_lines(draft: &PersonalizationDraft) -> Vec<String> {
    let preferred_name = draft.preferred_name.as_deref().unwrap_or("not set");
    let response_density = draft
        .response_density
        .map(|value| value.display_text())
        .unwrap_or("not set");
    let initiative_level = draft
        .initiative_level
        .map(|value| value.display_text())
        .unwrap_or("not set");
    let standing_boundaries = draft.standing_boundaries.as_deref().unwrap_or("not set");
    let timezone = draft.timezone.as_deref().unwrap_or("not set");
    let locale = draft.locale.as_deref().unwrap_or("not set");

    vec![
        "Review how Loong will work with you:".to_owned(),
        format!("- preferred name: {preferred_name}"),
        format!("- response density: {response_density}"),
        format!("- initiative level: {initiative_level}"),
        format!("- standing boundaries: {standing_boundaries}"),
        format!("- timezone: {timezone}"),
        format!("- locale: {locale}"),
    ]
}

fn save_personalization(
    ui: &mut impl OperatorPromptUi,
    resolved_path: &Path,
    config: &mut mvp::config::LoongConfig,
    existing_personalization: Option<&mvp::config::PersonalizationConfig>,
    draft: PersonalizationDraft,
    now: OffsetDateTime,
) -> CliResult<PersonalizeCliOutcome> {
    let existing_has_preferences = existing_personalization
        .is_some_and(mvp::config::PersonalizationConfig::has_operator_preferences);
    let personalization = build_configured_personalization(draft, now);
    if !personalization.has_operator_preferences() {
        if !existing_has_preferences {
            return Err("personalize save requires at least one operator preference".to_owned());
        }

        config.memory.personalization = None;
        let saved_path = write_personalization_config(config, resolved_path)?;
        let cleared_message = format!(
            "Cleared operator preferences from {}.",
            saved_path.display()
        );
        ui.print_line(cleared_message.as_str())?;

        return Ok(PersonalizeCliOutcome::Saved {
            upgraded_memory_profile: false,
        });
    }

    let mut upgraded_memory_profile = false;
    let mut declined_memory_profile_upgrade = false;
    let needs_memory_profile_upgrade =
        config.memory.profile != mvp::config::MemoryProfile::ProfilePlusWindow;
    if needs_memory_profile_upgrade {
        let confirmed = ui.prompt_confirm(
            "Upgrade memory profile to profile_plus_window so these preferences surface in Session Profile?",
            true,
        )?;
        if confirmed {
            config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
            upgraded_memory_profile = true;
        } else {
            declined_memory_profile_upgrade = true;
        }
    }

    config.memory.personalization = Some(personalization);
    let saved_path = write_personalization_config(config, resolved_path)?;

    ui.print_line(format!("Saved operator preferences to {}.", saved_path.display()).as_str())?;
    if upgraded_memory_profile {
        ui.print_line(
            "Memory profile upgraded to profile_plus_window so preferences project into Session Profile.",
        )?;
    }
    if declined_memory_profile_upgrade {
        ui.print_line(
            "Saved preferences without changing memory.profile; they will project once profile_plus_window is enabled.",
        )?;
    }

    Ok(PersonalizeCliOutcome::Saved {
        upgraded_memory_profile,
    })
}

fn build_configured_personalization(
    draft: PersonalizationDraft,
    now: OffsetDateTime,
) -> mvp::config::PersonalizationConfig {
    let updated_at_epoch_seconds = u64::try_from(now.unix_timestamp()).ok();
    let default_personalization = mvp::config::PersonalizationConfig::default();
    let schema_version = default_personalization.schema_version;

    mvp::config::PersonalizationConfig {
        preferred_name: draft.preferred_name,
        response_density: draft.response_density,
        initiative_level: draft.initiative_level,
        standing_boundaries: draft.standing_boundaries,
        timezone: draft.timezone,
        locale: draft.locale,
        prompt_state: mvp::config::PersonalizationPromptState::Configured,
        schema_version,
        updated_at_epoch_seconds,
    }
}

fn suppress_personalization(
    ui: &mut impl OperatorPromptUi,
    resolved_path: &Path,
    config: &mut mvp::config::LoongConfig,
    existing_personalization: Option<&mvp::config::PersonalizationConfig>,
    now: OffsetDateTime,
) -> CliResult<PersonalizeCliOutcome> {
    let personalization = build_suppressed_personalization(existing_personalization, now);

    config.memory.personalization = Some(personalization);
    let saved_path = write_personalization_config(config, resolved_path)?;

    ui.print_line(
        format!(
            "Suppressed future personalize suggestions in {}.",
            saved_path.display()
        )
        .as_str(),
    )?;

    Ok(PersonalizeCliOutcome::Suppressed)
}

fn build_suppressed_personalization(
    existing_personalization: Option<&mvp::config::PersonalizationConfig>,
    now: OffsetDateTime,
) -> mvp::config::PersonalizationConfig {
    let updated_at_epoch_seconds = u64::try_from(now.unix_timestamp()).ok();
    let default_personalization = mvp::config::PersonalizationConfig::default();
    let preserved_personalization = existing_personalization.cloned();
    let has_existing_personalization = preserved_personalization.is_some();
    let mut suppressed_personalization =
        preserved_personalization.unwrap_or(default_personalization);

    suppressed_personalization.prompt_state = mvp::config::PersonalizationPromptState::Suppressed;

    if !has_existing_personalization {
        suppressed_personalization.updated_at_epoch_seconds = updated_at_epoch_seconds;
    }

    suppressed_personalization
}

fn write_personalization_config(
    config: &mvp::config::LoongConfig,
    resolved_path: &Path,
) -> CliResult<PathBuf> {
    let resolved_path_string = resolved_path.display().to_string();
    mvp::config::write(Some(resolved_path_string.as_str()), config, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct TestPromptUi {
        inputs: VecDeque<String>,
        printed_lines: Vec<String>,
        prompt_labels: Vec<String>,
        select_labels: Vec<String>,
        select_option_labels: Vec<Vec<String>>,
        confirm_messages: Vec<String>,
    }

    impl TestPromptUi {
        fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
            let collected_inputs = inputs.into_iter().map(Into::into).collect();
            Self {
                inputs: collected_inputs,
                printed_lines: Vec::new(),
                prompt_labels: Vec::new(),
                select_labels: Vec::new(),
                select_option_labels: Vec::new(),
                confirm_messages: Vec::new(),
            }
        }
    }

    impl OperatorPromptUi for TestPromptUi {
        fn print_line(&mut self, line: &str) -> CliResult<()> {
            self.printed_lines.push(line.to_owned());
            Ok(())
        }

        fn prompt_with_default(&mut self, label: &str, default: &str) -> CliResult<String> {
            self.prompt_labels.push(label.to_owned());
            let next_input = self.inputs.pop_front().unwrap_or_default();
            let trimmed_input = next_input.trim();
            if trimmed_input.is_empty() {
                return Ok(default.to_owned());
            }
            Ok(trimmed_input.to_owned())
        }

        fn prompt_required(&mut self, label: &str) -> CliResult<String> {
            self.prompt_labels.push(label.to_owned());
            let next_input = self.inputs.pop_front().unwrap_or_default();
            Ok(next_input.trim().to_owned())
        }

        fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
            self.prompt_labels.push(label.to_owned());
            let next_input = self.inputs.pop_front().unwrap_or_default();
            Ok(next_input.trim().to_owned())
        }

        fn prompt_confirm(&mut self, message: &str, default: bool) -> CliResult<bool> {
            self.confirm_messages.push(message.to_owned());
            let next_input = self.inputs.pop_front().unwrap_or_default();
            let trimmed_input = next_input.trim().to_ascii_lowercase();
            if trimmed_input.is_empty() {
                return Ok(default);
            }
            Ok(matches!(trimmed_input.as_str(), "y" | "yes"))
        }

        fn select_one(
            &mut self,
            label: &str,
            options: &[SelectOption],
            default: Option<usize>,
            _interaction_mode: SelectInteractionMode,
        ) -> CliResult<usize> {
            self.select_labels.push(label.to_owned());
            self.select_option_labels
                .push(options.iter().map(|option| option.label.clone()).collect());
            let next_input = self.inputs.pop_front().unwrap_or_default();
            let trimmed_input = next_input.trim();
            if trimmed_input.is_empty() {
                return default.ok_or_else(|| "missing default selection".to_owned());
            }

            if let Ok(selected_number) = trimmed_input.parse::<usize>() {
                let selected_index = selected_number.saturating_sub(1);
                if selected_index < options.len() {
                    return Ok(selected_index);
                }
            }

            let matched_index = options
                .iter()
                .position(|option| option.slug.eq_ignore_ascii_case(trimmed_input));
            matched_index.ok_or_else(|| format!("invalid selection: {trimmed_input}"))
        }
    }

    fn fixed_now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_775_095_200).expect("fixed timestamp")
    }

    fn unique_config_path(label: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_millis();
        std::env::temp_dir().join(format!(
            "loong-personalize-{label}-{}-{millis}.toml",
            std::process::id()
        ))
    }

    fn write_default_config(path: &Path) {
        write_config(path, &mvp::config::LoongConfig::default());
    }

    fn write_config(path: &Path, config: &mvp::config::LoongConfig) {
        let path_string = path.display().to_string();
        mvp::config::write(Some(path_string.as_str()), config, true).expect("write config");
    }

    fn personalization_schema_version_for_tests() -> u32 {
        let default_personalization = mvp::config::PersonalizationConfig::default();
        default_personalization.schema_version
    }

    fn configured_personalization_for_tests() -> mvp::config::PersonalizationConfig {
        let schema_version = personalization_schema_version_for_tests();
        mvp::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(mvp::config::ResponseDensity::Balanced),
            initiative_level: Some(mvp::config::InitiativeLevel::AskBeforeActing),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: Some("zh-CN".to_owned()),
            prompt_state: mvp::config::PersonalizationPromptState::Configured,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        }
    }

    fn configured_personalize_config_for_tests() -> mvp::config::LoongConfig {
        let personalization = configured_personalization_for_tests();
        let mut config = mvp::config::LoongConfig::default();
        config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
        config.memory.personalization = Some(personalization);
        config
    }

    #[test]
    fn personalize_cli_save_updates_config_and_memory_profile() {
        let config_path = unique_config_path("save");
        let config_path_string = config_path.display().to_string();
        write_default_config(&config_path);
        let expected_schema_version = personalization_schema_version_for_tests();
        let mut ui = TestPromptUi::with_inputs([
            "Chum",
            "3",
            "3",
            "Ask before destructive actions.",
            "Asia/Shanghai",
            "zh-CN",
            "1",
            "y",
        ]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("save flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("load personalized config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("saved personalization");

        assert_eq!(
            outcome,
            PersonalizeCliOutcome::Saved {
                upgraded_memory_profile: true
            }
        );
        assert_eq!(
            loaded_config.memory.profile,
            mvp::config::MemoryProfile::ProfilePlusWindow
        );
        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(
            personalization.response_density,
            Some(mvp::config::ResponseDensity::Thorough)
        );
        assert_eq!(
            personalization.initiative_level,
            Some(mvp::config::InitiativeLevel::HighInitiative)
        );
        assert_eq!(
            personalization.standing_boundaries.as_deref(),
            Some("Ask before destructive actions.")
        );
        assert_eq!(personalization.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(personalization.locale.as_deref(), Some("zh-CN"));
        assert_eq!(
            personalization.prompt_state,
            mvp::config::PersonalizationPromptState::Configured
        );
        assert_eq!(personalization.schema_version, expected_schema_version);
        assert_eq!(
            personalization.updated_at_epoch_seconds,
            Some(1_775_095_200)
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_skip_leaves_config_untouched() {
        let config_path = unique_config_path("skip");
        let config_path_string = config_path.display().to_string();
        write_default_config(&config_path);
        let mut ui = TestPromptUi::with_inputs(["", "", "", "", "", "", "2"]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("skip flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;

        assert_eq!(outcome, PersonalizeCliOutcome::Skipped);
        assert_eq!(loaded_config.memory.personalization, None);
        assert_eq!(
            loaded_config.memory.profile,
            mvp::config::MemoryProfile::WindowOnly
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_suppress_persists_prompt_state_without_preferences() {
        let config_path = unique_config_path("suppress");
        let config_path_string = config_path.display().to_string();
        write_default_config(&config_path);
        let expected_schema_version = personalization_schema_version_for_tests();
        let mut ui = TestPromptUi::with_inputs(["", "", "", "", "", "", "3"]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("suppress flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("suppressed personalization state");

        assert_eq!(outcome, PersonalizeCliOutcome::Suppressed);
        assert_eq!(personalization.preferred_name, None);
        assert_eq!(personalization.response_density, None);
        assert_eq!(personalization.initiative_level, None);
        assert_eq!(personalization.standing_boundaries, None);
        assert_eq!(personalization.timezone, None);
        assert_eq!(personalization.locale, None);
        assert_eq!(
            personalization.prompt_state,
            mvp::config::PersonalizationPromptState::Suppressed
        );
        assert_eq!(personalization.schema_version, expected_schema_version);
        assert_eq!(
            personalization.updated_at_epoch_seconds,
            Some(1_775_095_200)
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_suppress_preserves_existing_preferences() {
        let config_path = unique_config_path("suppress-preserve");
        let config_path_string = config_path.display().to_string();
        let custom_schema_version = personalization_schema_version_for_tests() + 7;
        let preserved_updated_at_epoch_seconds = Some(1_700_000_000);
        let personalization = mvp::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(mvp::config::ResponseDensity::Balanced),
            initiative_level: Some(mvp::config::InitiativeLevel::AskBeforeActing),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: Some("zh-CN".to_owned()),
            prompt_state: mvp::config::PersonalizationPromptState::Configured,
            schema_version: custom_schema_version,
            updated_at_epoch_seconds: preserved_updated_at_epoch_seconds,
        };
        let mut config = mvp::config::LoongConfig::default();
        config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
        config.memory.personalization = Some(personalization);
        write_config(&config_path, &config);
        let mut ui =
            TestPromptUi::with_inputs(["New Name", "3", "3", "New boundary", "UTC", "en-US", "3"]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("suppress flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("suppressed personalization state");

        assert_eq!(outcome, PersonalizeCliOutcome::Suppressed);
        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(
            personalization.response_density,
            Some(mvp::config::ResponseDensity::Balanced)
        );
        assert_eq!(
            personalization.initiative_level,
            Some(mvp::config::InitiativeLevel::AskBeforeActing)
        );
        assert_eq!(
            personalization.standing_boundaries.as_deref(),
            Some("Ask before destructive actions.")
        );
        assert_eq!(personalization.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(personalization.locale.as_deref(), Some("zh-CN"));
        assert_eq!(
            personalization.prompt_state,
            mvp::config::PersonalizationPromptState::Suppressed
        );
        assert_eq!(personalization.schema_version, custom_schema_version);
        assert_eq!(
            personalization.updated_at_epoch_seconds,
            preserved_updated_at_epoch_seconds
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_declined_memory_profile_upgrade_still_saves_preferences() {
        let config_path = unique_config_path("decline-upgrade");
        let config_path_string = config_path.display().to_string();
        write_default_config(&config_path);
        let mut ui = TestPromptUi::with_inputs([
            "Chum",
            "2",
            "2",
            "Ask before destructive actions.",
            "",
            "",
            "1",
            "n",
        ]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("declined upgrade flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("personalization should still be saved");

        assert_eq!(
            outcome,
            PersonalizeCliOutcome::Saved {
                upgraded_memory_profile: false
            }
        );
        assert_eq!(
            loaded_config.memory.profile,
            mvp::config::MemoryProfile::WindowOnly
        );
        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(
            personalization.response_density,
            Some(mvp::config::ResponseDensity::Balanced)
        );
        assert_eq!(
            personalization.initiative_level,
            Some(mvp::config::InitiativeLevel::Balanced)
        );
        assert_eq!(
            personalization.standing_boundaries.as_deref(),
            Some("Ask before destructive actions.")
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_save_allows_clearing_existing_text_fields() {
        let config_path = unique_config_path("clear-text");
        let config_path_string = config_path.display().to_string();
        let config = configured_personalize_config_for_tests();
        write_config(&config_path, &config);
        let mut ui = TestPromptUi::with_inputs(["-", "", "", "-", "", "", "1"]);

        run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
            .expect("clear-text save flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("saved personalization");

        assert_eq!(personalization.preferred_name, None);
        assert_eq!(personalization.standing_boundaries, None);
        assert_eq!(
            personalization.response_density,
            Some(mvp::config::ResponseDensity::Balanced)
        );
        assert_eq!(
            personalization.initiative_level,
            Some(mvp::config::InitiativeLevel::AskBeforeActing)
        );
        assert_eq!(personalization.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(personalization.locale.as_deref(), Some("zh-CN"));

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_save_allows_clearing_existing_enum_fields() {
        let config_path = unique_config_path("clear-enum");
        let config_path_string = config_path.display().to_string();
        let mut config = configured_personalize_config_for_tests();
        let schema_version = personalization_schema_version_for_tests();
        config.memory.personalization = Some(mvp::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(mvp::config::ResponseDensity::Balanced),
            initiative_level: Some(mvp::config::InitiativeLevel::HighInitiative),
            standing_boundaries: None,
            timezone: None,
            locale: None,
            prompt_state: mvp::config::PersonalizationPromptState::Configured,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        });
        write_config(&config_path, &config);
        let mut ui = TestPromptUi::with_inputs(["", "clear", "clear", "", "", "", "1"]);

        run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
            .expect("clear-enum save flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("saved personalization");

        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(personalization.response_density, None);
        assert_eq!(personalization.initiative_level, None);

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_save_clears_personalization_when_all_existing_fields_are_removed() {
        let config_path = unique_config_path("clear-all");
        let config_path_string = config_path.display().to_string();
        let config = configured_personalize_config_for_tests();
        write_config(&config_path, &config);
        let mut ui = TestPromptUi::with_inputs(["-", "clear", "clear", "-", "-", "-", "1"]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("clear-all save flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;

        assert_eq!(
            outcome,
            PersonalizeCliOutcome::Saved {
                upgraded_memory_profile: false
            }
        );
        assert_eq!(loaded_config.memory.personalization, None);

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_save_from_suppressed_state_prints_reenable_guidance() {
        let config_path = unique_config_path("suppressed-recovery");
        let config_path_string = config_path.display().to_string();
        let mut config = mvp::config::LoongConfig::default();
        let schema_version = personalization_schema_version_for_tests();
        config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
        config.memory.personalization = Some(mvp::config::PersonalizationConfig {
            preferred_name: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            timezone: None,
            locale: None,
            prompt_state: mvp::config::PersonalizationPromptState::Suppressed,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        });
        write_config(&config_path, &config);
        let mut ui = TestPromptUi::with_inputs(["Chum", "", "", "", "", "", "1"]);

        let outcome =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect("suppressed recovery flow should succeed");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("saved personalization");

        assert_eq!(
            outcome,
            PersonalizeCliOutcome::Saved {
                upgraded_memory_profile: false
            }
        );
        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(
            personalization.prompt_state,
            mvp::config::PersonalizationPromptState::Configured
        );
        assert!(
            ui.printed_lines
                .iter()
                .any(|line| { line.contains("currently suppressed") }),
            "recovery flow should explain that the current state is suppressed: {:#?}",
            ui.printed_lines
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_empty_save_without_existing_preferences_is_invalid() {
        let config_path = unique_config_path("empty-save");
        let config_path_string = config_path.display().to_string();
        write_default_config(&config_path);
        let mut ui = TestPromptUi::with_inputs(["", "", "", "", "", "", "1"]);

        let error =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect_err("empty save should stay invalid");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;

        assert!(error.contains("at least one operator preference"));
        assert_eq!(loaded_config.memory.personalization, None);

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_empty_save_does_not_clear_suppressed_state_without_preferences() {
        let config_path = unique_config_path("suppressed-empty-save");
        let config_path_string = config_path.display().to_string();
        let mut config = mvp::config::LoongConfig::default();
        let schema_version = personalization_schema_version_for_tests();
        config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
        config.memory.personalization = Some(mvp::config::PersonalizationConfig {
            preferred_name: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            timezone: None,
            locale: None,
            prompt_state: mvp::config::PersonalizationPromptState::Suppressed,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        });
        write_config(&config_path, &config);
        let mut ui = TestPromptUi::with_inputs(["", "", "", "", "", "", "1"]);

        let error =
            run_personalize_cli_with_ui(Some(config_path_string.as_str()), &mut ui, fixed_now())
                .expect_err("empty suppressed save should stay invalid");
        let load_result =
            mvp::config::load(Some(config_path_string.as_str())).expect("reload config");
        let (_, loaded_config) = load_result;
        let personalization = loaded_config
            .memory
            .personalization
            .expect("suppressed state should remain present");

        assert!(error.contains("at least one operator preference"));
        assert_eq!(
            personalization.prompt_state,
            mvp::config::PersonalizationPromptState::Suppressed
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn render_review_lines_uses_human_readable_initiative_copy() {
        let draft = PersonalizationDraft {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(mvp::config::ResponseDensity::Balanced),
            initiative_level: Some(mvp::config::InitiativeLevel::AskBeforeActing),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: Some("zh-CN".to_owned()),
        };

        let lines = render_review_lines(&draft);

        assert!(
            lines
                .iter()
                .any(|line| line == "- initiative level: ask before acting"),
            "review copy should stay human-readable instead of leaking schema ids: {lines:#?}"
        );
    }

    #[test]
    fn collect_personalization_draft_uses_guidance_prompt_labels() {
        let mut ui = TestPromptUi::with_inputs(["", "", "", "", "", ""]);

        let draft = collect_personalization_draft(&mut ui, None).expect("collect draft");

        assert_eq!(
            draft,
            PersonalizationDraft {
                preferred_name: None,
                response_density: None,
                initiative_level: None,
                standing_boundaries: None,
                timezone: None,
                locale: None,
            }
        );
        assert_eq!(
            ui.prompt_labels,
            vec![
                "How should Loong address you? (optional)",
                "Any standing boundaries Loong should keep in mind? (optional)",
                "Which timezone should Loong assume? (optional)",
                "Which locale should Loong default to? (optional)",
            ],
            "text prompts should read like operator guidance, not raw field labels: {:#?}",
            ui.prompt_labels
        );
        assert_eq!(
            ui.select_labels,
            vec![
                "How detailed should Loong usually be?",
                "How proactive should Loong be?",
            ],
            "selection prompts should stay conversational and operator-facing: {:#?}",
            ui.select_labels
        );
    }

    #[test]
    fn select_review_action_uses_guidance_option_copy() {
        let mut ui = TestPromptUi::with_inputs([""]);
        let draft = PersonalizationDraft {
            preferred_name: None,
            response_density: None,
            initiative_level: None,
            standing_boundaries: None,
            timezone: None,
            locale: None,
        };

        let action = select_review_action(&mut ui, &draft).expect("select review action");

        assert_eq!(action, PersonalizeReviewAction::Save);
        assert_eq!(
            ui.select_labels,
            vec!["What should Loong do with this draft?"],
            "review action prompt should sound like guidance instead of a raw form action: {:#?}",
            ui.select_labels
        );
        assert_eq!(
            ui.select_option_labels,
            vec![vec![
                "use this draft".to_owned(),
                "not now".to_owned(),
                "stop suggesting this".to_owned(),
            ]],
            "review action options should stay operator-facing: {:#?}",
            ui.select_option_labels
        );
    }
}
