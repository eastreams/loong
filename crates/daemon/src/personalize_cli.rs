#[cfg(test)]
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use time::OffsetDateTime;

use crate::operator_prompt::{
    OperatorPromptUi, SelectInteractionMode, SelectOption, StdioOperatorUi,
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
    SkippedMemoryProfileUpgrade,
}

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
    let draft = collect_personalization_draft(ui, existing_personalization.as_ref())?;
    let review_action = select_review_action(ui, &draft)?;

    match review_action {
        PersonalizeReviewAction::Save => {
            save_personalization(ui, &resolved_path, &mut config, draft, now)
        }
        PersonalizeReviewAction::SkipForNow => {
            ui.print_line("No changes saved.")?;
            Ok(PersonalizeCliOutcome::Skipped)
        }
        PersonalizeReviewAction::SuppressFutureSuggestions => {
            suppress_personalization(ui, &resolved_path, &mut config, now)
        }
    }
}

fn collect_personalization_draft(
    ui: &mut impl OperatorPromptUi,
    existing_personalization: Option<&mvp::config::PersonalizationConfig>,
) -> CliResult<PersonalizationDraft> {
    let preferred_name_default = existing_personalization
        .and_then(|personalization| personalization.preferred_name.as_deref());
    let preferred_name =
        prompt_optional_text(ui, "Preferred name (optional)", preferred_name_default)?;

    let response_density_default =
        existing_personalization.and_then(|personalization| personalization.response_density);
    let response_density = select_response_density(ui, response_density_default)?;

    let initiative_level_default =
        existing_personalization.and_then(|personalization| personalization.initiative_level);
    let initiative_level = select_initiative_level(ui, initiative_level_default)?;

    let standing_boundaries_default = existing_personalization
        .and_then(|personalization| personalization.standing_boundaries.as_deref());
    let standing_boundaries = prompt_optional_text(
        ui,
        "Standing boundaries (optional)",
        standing_boundaries_default,
    )?;

    let timezone_default =
        existing_personalization.and_then(|personalization| personalization.timezone.as_deref());
    let timezone = prompt_optional_text(ui, "Timezone (optional)", timezone_default)?;

    let locale_default =
        existing_personalization.and_then(|personalization| personalization.locale.as_deref());
    let locale = prompt_optional_text(ui, "Locale (optional)", locale_default)?;

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
    let raw_value = match current_value {
        Some(default_value) => ui.prompt_with_default(label, default_value)?,
        None => ui.prompt_allow_empty(label)?,
    };
    let trimmed_value = raw_value.trim();
    if trimmed_value.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed_value.to_owned()))
}

fn select_response_density(
    ui: &mut impl OperatorPromptUi,
    current_value: Option<mvp::config::ResponseDensity>,
) -> CliResult<Option<mvp::config::ResponseDensity>> {
    let options = vec![
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
    let default_index = match current_value {
        Some(mvp::config::ResponseDensity::Concise) => Some(0),
        Some(mvp::config::ResponseDensity::Balanced) => Some(1),
        Some(mvp::config::ResponseDensity::Thorough) => Some(2),
        None => Some(1),
    };
    let selected_index = ui.select_one(
        "Response density",
        &options,
        default_index,
        SelectInteractionMode::List,
    )?;
    let selected_value = match selected_index {
        0 => mvp::config::ResponseDensity::Concise,
        1 => mvp::config::ResponseDensity::Balanced,
        2 => mvp::config::ResponseDensity::Thorough,
        _ => return Err("response density selection out of range".to_owned()),
    };
    Ok(Some(selected_value))
}

fn select_initiative_level(
    ui: &mut impl OperatorPromptUi,
    current_value: Option<mvp::config::InitiativeLevel>,
) -> CliResult<Option<mvp::config::InitiativeLevel>> {
    let options = vec![
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
    let default_index = match current_value {
        Some(mvp::config::InitiativeLevel::AskBeforeActing) => Some(0),
        Some(mvp::config::InitiativeLevel::Balanced) => Some(1),
        Some(mvp::config::InitiativeLevel::HighInitiative) => Some(2),
        None => Some(1),
    };
    let selected_index = ui.select_one(
        "Initiative level",
        &options,
        default_index,
        SelectInteractionMode::List,
    )?;
    let selected_value = match selected_index {
        0 => mvp::config::InitiativeLevel::AskBeforeActing,
        1 => mvp::config::InitiativeLevel::Balanced,
        2 => mvp::config::InitiativeLevel::HighInitiative,
        _ => return Err("initiative level selection out of range".to_owned()),
    };
    Ok(Some(selected_value))
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
            label: "save".to_owned(),
            slug: "save".to_owned(),
            description: "persist these preferences into advisory session profile state".to_owned(),
            recommended: true,
        },
        SelectOption {
            label: "skip for now".to_owned(),
            slug: "skip".to_owned(),
            description: "leave the current config untouched".to_owned(),
            recommended: false,
        },
        SelectOption {
            label: "suppress future suggestions".to_owned(),
            slug: "suppress".to_owned(),
            description: "persist a do-not-suggest state without saving preferences".to_owned(),
            recommended: false,
        },
    ];
    let selected_index = ui.select_one(
        "Review action",
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
        .map(|value| value.as_str())
        .unwrap_or("not set");
    let initiative_level = draft
        .initiative_level
        .map(|value| value.as_str())
        .unwrap_or("not set");
    let standing_boundaries = draft.standing_boundaries.as_deref().unwrap_or("not set");
    let timezone = draft.timezone.as_deref().unwrap_or("not set");
    let locale = draft.locale.as_deref().unwrap_or("not set");

    vec![
        "Review operator preferences:".to_owned(),
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
    config: &mut mvp::config::LoongClawConfig,
    draft: PersonalizationDraft,
    now: OffsetDateTime,
) -> CliResult<PersonalizeCliOutcome> {
    let personalization = build_configured_personalization(draft, now);
    if !personalization.has_operator_preferences() {
        return Err("personalize save requires at least one operator preference".to_owned());
    }

    let mut upgraded_memory_profile = false;
    let needs_memory_profile_upgrade =
        config.memory.profile != mvp::config::MemoryProfile::ProfilePlusWindow;
    if needs_memory_profile_upgrade {
        let confirmed = ui.prompt_confirm(
            "Upgrade memory profile to profile_plus_window so these preferences surface in Session Profile?",
            true,
        )?;
        if !confirmed {
            ui.print_line("No changes saved.")?;
            return Ok(PersonalizeCliOutcome::SkippedMemoryProfileUpgrade);
        }
        config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
        upgraded_memory_profile = true;
    }

    config.memory.personalization = Some(personalization);
    let saved_path = write_personalization_config(config, resolved_path)?;

    ui.print_line(format!("Saved operator preferences to {}.", saved_path.display()).as_str())?;
    if upgraded_memory_profile {
        ui.print_line(
            "Memory profile upgraded to profile_plus_window so preferences project into Session Profile.",
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

    mvp::config::PersonalizationConfig {
        preferred_name: draft.preferred_name,
        response_density: draft.response_density,
        initiative_level: draft.initiative_level,
        standing_boundaries: draft.standing_boundaries,
        timezone: draft.timezone,
        locale: draft.locale,
        prompt_state: mvp::config::PersonalizationPromptState::Configured,
        schema_version: 1,
        updated_at_epoch_seconds,
    }
}

fn suppress_personalization(
    ui: &mut impl OperatorPromptUi,
    resolved_path: &Path,
    config: &mut mvp::config::LoongClawConfig,
    now: OffsetDateTime,
) -> CliResult<PersonalizeCliOutcome> {
    let updated_at_epoch_seconds = u64::try_from(now.unix_timestamp()).ok();
    let personalization = mvp::config::PersonalizationConfig {
        preferred_name: None,
        response_density: None,
        initiative_level: None,
        standing_boundaries: None,
        timezone: None,
        locale: None,
        prompt_state: mvp::config::PersonalizationPromptState::Suppressed,
        schema_version: 1,
        updated_at_epoch_seconds,
    };

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

fn write_personalization_config(
    config: &mvp::config::LoongClawConfig,
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
    }

    impl TestPromptUi {
        fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
            let collected_inputs = inputs.into_iter().map(Into::into).collect();
            Self {
                inputs: collected_inputs,
                printed_lines: Vec::new(),
            }
        }
    }

    impl OperatorPromptUi for TestPromptUi {
        fn print_line(&mut self, line: &str) -> CliResult<()> {
            self.printed_lines.push(line.to_owned());
            Ok(())
        }

        fn prompt_with_default(&mut self, _label: &str, default: &str) -> CliResult<String> {
            let next_input = self.inputs.pop_front().unwrap_or_default();
            let trimmed_input = next_input.trim();
            if trimmed_input.is_empty() {
                return Ok(default.to_owned());
            }
            Ok(trimmed_input.to_owned())
        }

        fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
            let next_input = self.inputs.pop_front().unwrap_or_default();
            Ok(next_input.trim().to_owned())
        }

        fn prompt_allow_empty(&mut self, _label: &str) -> CliResult<String> {
            let next_input = self.inputs.pop_front().unwrap_or_default();
            Ok(next_input.trim().to_owned())
        }

        fn prompt_confirm(&mut self, _message: &str, default: bool) -> CliResult<bool> {
            let next_input = self.inputs.pop_front().unwrap_or_default();
            let trimmed_input = next_input.trim().to_ascii_lowercase();
            if trimmed_input.is_empty() {
                return Ok(default);
            }
            Ok(matches!(trimmed_input.as_str(), "y" | "yes"))
        }

        fn select_one(
            &mut self,
            _label: &str,
            options: &[SelectOption],
            default: Option<usize>,
            _interaction_mode: SelectInteractionMode,
        ) -> CliResult<usize> {
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
            "loongclaw-personalize-{label}-{}-{millis}.toml",
            std::process::id()
        ))
    }

    fn write_default_config(path: &Path) {
        let path_string = path.display().to_string();
        mvp::config::write(
            Some(path_string.as_str()),
            &mvp::config::LoongClawConfig::default(),
            true,
        )
        .expect("write default config");
    }

    #[test]
    fn personalize_cli_save_updates_config_and_memory_profile() {
        let config_path = unique_config_path("save");
        let config_path_string = config_path.display().to_string();
        write_default_config(&config_path);
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
        assert_eq!(personalization.schema_version, 1);
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
        assert_eq!(personalization.schema_version, 1);
        assert_eq!(
            personalization.updated_at_epoch_seconds,
            Some(1_775_095_200)
        );

        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn personalize_cli_declined_memory_profile_upgrade_keeps_config_untouched() {
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

        assert_eq!(outcome, PersonalizeCliOutcome::SkippedMemoryProfileUpgrade);
        assert_eq!(loaded_config.memory.personalization, None);
        assert_eq!(
            loaded_config.memory.profile,
            mvp::config::MemoryProfile::WindowOnly
        );

        let _ = std::fs::remove_file(config_path);
    }
}
