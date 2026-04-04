use loongclaw_spec::CliResult;

pub use crate::onboard_cli::{OnboardUi as OperatorPromptUi, SelectInteractionMode, SelectOption};

pub(crate) use crate::onboard_cli::StdioOnboardUi as StdioOperatorUi;

pub(crate) const OPERATOR_CLEAR_INPUT_TOKEN: &str = "-";

pub(crate) fn is_operator_clear_input(raw: &str) -> bool {
    let trimmed_value = raw.trim();
    trimmed_value == OPERATOR_CLEAR_INPUT_TOKEN
}

pub(crate) fn prompt_optional_operator_text(
    ui: &mut impl OperatorPromptUi,
    label: &str,
    current_value: Option<&str>,
) -> CliResult<Option<String>> {
    let raw_value = ui.prompt_allow_empty(label)?;
    let trimmed_value = raw_value.trim();

    if trimmed_value.is_empty() {
        let preserved_value = current_value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        return Ok(preserved_value);
    }

    if is_operator_clear_input(trimmed_value) {
        return Ok(None);
    }

    Ok(Some(trimmed_value.to_owned()))
}
