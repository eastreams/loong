use super::*;
use dialoguer::{Confirm, Error as DialoguerError, FuzzySelect, Input, Select};

pub(super) fn map_rich_prompt_error(action: &str, error: DialoguerError) -> String {
    let error: io::Error = error.into();
    if error.kind() == io::ErrorKind::Interrupted {
        return "onboarding cancelled: prompt aborted".to_owned();
    }
    format!("{action} failed: {error}")
}

pub(super) fn prompt_with_default_rich(label: &str, default: &str) -> CliResult<String> {
    let term = rich_prompt_term();
    prompt_with_default_rich_on(&term, label, default)
}

pub(super) fn prompt_with_default_rich_on(
    term: &Term,
    label: &str,
    default: &str,
) -> CliResult<String> {
    let theme = rich_prompt_theme();
    let value = Input::<String>::with_theme(&theme)
        .with_prompt(label)
        .default(default.to_owned())
        .report(false)
        .interact_text_on(term)
        .map_err(|error| map_rich_prompt_error("interactive prompt", error))?;
    let value = ensure_onboard_input_not_cancelled(value)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(default.to_owned());
    }
    Ok(trimmed.to_owned())
}

pub(super) fn prompt_required_rich(label: &str) -> CliResult<String> {
    let term = rich_prompt_term();
    prompt_required_rich_on(&term, label)
}

pub(super) fn prompt_required_rich_on(term: &Term, label: &str) -> CliResult<String> {
    let theme = rich_prompt_theme();
    let value = Input::<String>::with_theme(&theme)
        .with_prompt(label)
        .report(false)
        .interact_text_on(term)
        .map_err(|error| map_rich_prompt_error("interactive prompt", error))?;
    let value = ensure_onboard_input_not_cancelled(value)?;
    Ok(value.trim().to_owned())
}

pub(super) fn prompt_allow_empty_rich(label: &str) -> CliResult<String> {
    let term = rich_prompt_term();
    prompt_allow_empty_rich_on(&term, label)
}

pub(super) fn prompt_allow_empty_rich_on(term: &Term, label: &str) -> CliResult<String> {
    let theme = rich_prompt_theme();
    let value = Input::<String>::with_theme(&theme)
        .with_prompt(label)
        .allow_empty(true)
        .report(false)
        .interact_text_on(term)
        .map_err(|error| map_rich_prompt_error("interactive prompt", error))?;
    let value = ensure_onboard_input_not_cancelled(value)?;
    Ok(value.trim().to_owned())
}

pub(super) fn prompt_confirm_rich(message: &str, default: bool) -> CliResult<bool> {
    let term = rich_prompt_term();
    let theme = rich_prompt_theme();
    Confirm::with_theme(&theme)
        .with_prompt(message)
        .default(default)
        .report(false)
        .interact_on_opt(&term)
        .map_err(|error| map_rich_prompt_error("interactive confirmation", error))?
        .ok_or_else(|| "onboarding cancelled: prompt aborted".to_owned())
}

pub(super) fn select_one_rich(
    label: &str,
    options: &[SelectOption],
    default: Option<usize>,
    interaction_mode: SelectInteractionMode,
) -> CliResult<usize> {
    let default = validate_select_one_state(options.len(), default)?;
    let items = options
        .iter()
        .map(render_select_option_item)
        .collect::<Vec<_>>();
    let term = rich_prompt_term();
    let theme = rich_prompt_theme();
    let selection = match interaction_mode {
        SelectInteractionMode::List => {
            let prompt = Select::with_theme(&theme)
                .with_prompt(label)
                .items(&items)
                .report(false);
            let prompt = if let Some(idx) = default {
                prompt.default(idx)
            } else {
                prompt
            };
            prompt
                .interact_on_opt(&term)
                .map_err(|error| map_rich_prompt_error("interactive selection", error))?
        }
        SelectInteractionMode::Search => {
            let prompt = FuzzySelect::with_theme(&theme)
                .with_prompt(label)
                .items(&items)
                .report(false);
            let prompt = if let Some(idx) = default {
                prompt.default(idx)
            } else {
                prompt
            };
            prompt
                .interact_on_opt(&term)
                .map_err(|error| map_rich_prompt_error("interactive model search", error))?
        }
    };
    selection.ok_or_else(|| "onboarding cancelled: prompt aborted".to_owned())
}

pub(super) fn render_select_option_item(option: &SelectOption) -> String {
    let mut rendered = option.label.clone();
    if !option.description.trim().is_empty() {
        rendered.push_str(" - ");
        rendered.push_str(option.description.trim());
    }
    if option.recommended {
        rendered.push_str(" (recommended)");
    }
    rendered
}

pub(super) fn summarize_select_option_description(detail_lines: &[String]) -> String {
    detail_lines
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(super) fn select_options_from_screen_options(
    options: &[OnboardScreenOption],
) -> Vec<SelectOption> {
    options
        .iter()
        .map(|option| SelectOption {
            label: option.label.clone(),
            slug: option.key.clone(),
            description: summarize_select_option_description(&option.detail_lines),
            recommended: option.recommended,
        })
        .collect()
}

pub(super) fn tui_choices_from_screen_options(
    options: &[OnboardScreenOption],
) -> Vec<TuiChoiceSpec> {
    options
        .iter()
        .map(|option| TuiChoiceSpec {
            key: option.key.clone(),
            label: option.label.clone(),
            detail_lines: option.detail_lines.clone(),
            recommended: option.recommended,
        })
        .collect()
}

pub(super) fn select_screen_option(
    ui: &mut impl OnboardUi,
    label: &str,
    options: &[OnboardScreenOption],
    default_key: Option<&str>,
) -> CliResult<usize> {
    let select_options = select_options_from_screen_options(options);
    let default_idx =
        default_key.and_then(|key| options.iter().position(|option| option.key == key));
    ui.select_one(
        label,
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )
}

pub(super) fn build_onboard_entry_screen_options(
    options: &[OnboardEntryOption],
) -> Vec<OnboardScreenOption> {
    options
        .iter()
        .enumerate()
        .map(|(index, option)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: option.label.to_owned(),
            detail_lines: vec![option.detail.clone()],
            recommended: option.recommended,
        })
        .collect()
}

pub(super) fn build_starting_point_selection_screen_options(
    sorted_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<OnboardScreenOption> {
    let mut options = sorted_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: onboard_starting_point_label(Some(candidate.source_kind), &candidate.source),
            detail_lines: summarize_starting_point_detail_lines(candidate, width),
            recommended: matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::RecommendedPlan
            ),
        })
        .collect::<Vec<_>>();
    options.push(OnboardScreenOption {
        key: "0".to_owned(),
        label: crate::onboard_presentation::start_fresh_option_label().to_owned(),
        detail_lines: start_fresh_starting_point_detail_lines(),
        recommended: false,
    });
    options
}

pub(super) fn build_onboard_shortcut_screen_options(
    shortcut_kind: OnboardShortcutKind,
) -> Vec<OnboardScreenOption> {
    vec![
        OnboardScreenOption {
            key: "1".to_owned(),
            label: shortcut_kind.primary_label().to_owned(),
            detail_lines: vec![crate::onboard_presentation::shortcut_continue_detail().to_owned()],
            recommended: true,
        },
        OnboardScreenOption {
            key: "2".to_owned(),
            label: crate::onboard_presentation::adjust_settings_label().to_owned(),
            detail_lines: vec![crate::onboard_presentation::shortcut_adjust_detail().to_owned()],
            recommended: false,
        },
    ]
}

pub(super) fn build_existing_config_write_screen_options() -> Vec<OnboardScreenOption> {
    vec![
        OnboardScreenOption {
            key: "o".to_owned(),
            label: "Replace existing config".to_owned(),
            detail_lines: vec!["overwrite the current file with this onboarding draft".to_owned()],
            recommended: false,
        },
        OnboardScreenOption {
            key: "b".to_owned(),
            label: "Create backup and replace".to_owned(),
            detail_lines: vec![
                "save a timestamped .bak copy first, then write the new config".to_owned(),
            ],
            recommended: true,
        },
        OnboardScreenOption {
            key: "c".to_owned(),
            label: "Cancel".to_owned(),
            detail_lines: vec!["leave the existing config untouched".to_owned()],
            recommended: false,
        },
    ]
}

pub(super) fn validate_select_one_state(
    options_len: usize,
    default: Option<usize>,
) -> CliResult<Option<usize>> {
    if options_len == 0 {
        return Err("no selection options available".to_owned());
    }
    if let Some(idx) = default
        && idx >= options_len
    {
        return Err(format!(
            "default selection index {idx} out of range 0..{}",
            options_len - 1
        ));
    }
    Ok(default)
}

pub(super) fn select_option_input_slug(option: &SelectOption) -> &str {
    if option.slug == ONBOARD_CUSTOM_MODEL_OPTION_SLUG {
        "custom"
    } else {
        option.slug.as_str()
    }
}

pub(super) fn parse_select_one_input(trimmed: &str, options: &[SelectOption]) -> Option<usize> {
    if let Ok(selected) = trimmed.parse::<usize>()
        && (1..=options.len()).contains(&selected)
    {
        return Some(selected - 1);
    }

    let direct_match = options.iter().position(|option| {
        option.slug.eq_ignore_ascii_case(trimmed)
            || select_option_input_slug(option).eq_ignore_ascii_case(trimmed)
    });

    if direct_match.is_some() {
        return direct_match;
    }

    parse_prompt_personality_select_input(trimmed, options)
}

pub(super) fn render_select_one_invalid_input_message(options: &[SelectOption]) -> String {
    format!(
        "invalid selection. enter a number between 1 and {}, or one of: {}",
        options.len(),
        options
            .iter()
            .map(select_option_input_slug)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn resolve_select_one_eof(default: Option<usize>) -> CliResult<usize> {
    default.ok_or_else(|| {
        "onboarding cancelled: stdin closed while waiting for required selection".to_owned()
    })
}

pub(super) fn parse_prompt_personality_select_input(
    trimmed: &str,
    options: &[SelectOption],
) -> Option<usize> {
    let prompt_personality_options = select_options_are_prompt_personalities(options);

    if !prompt_personality_options {
        return None;
    }

    let personality = parse_prompt_personality(trimmed)?;
    let canonical_slug = prompt_personality_id(personality);

    options
        .iter()
        .position(|option| option.slug.eq_ignore_ascii_case(canonical_slug))
}

pub(super) fn select_options_are_prompt_personalities(options: &[SelectOption]) -> bool {
    for option in options {
        let parsed_personality = parse_prompt_personality(&option.slug);

        if parsed_personality.is_none() {
            return false;
        }
    }

    true
}
