use super::*;

#[cfg(not(feature = "memory-sqlite"))]
pub(super) fn render_cli_chat_feature_unavailable_lines_with_width(
    role: &str,
    detail: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_feature_unavailable_message_spec(role, detail);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(not(feature = "memory-sqlite"))]
fn build_cli_chat_feature_unavailable_message_spec(role: &str, detail: &str) -> TuiMessageSpec {
    let sections = vec![TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Warning,
        title: Some("feature unavailable".to_owned()),
        lines: vec![detail.to_owned()],
    }];

    TuiMessageSpec {
        role: role.to_owned(),
        caption: Some("unavailable".to_owned()),
        sections,
        footer_lines: vec![
            "Feature gated in this build; /help shows the available chat surface.".to_owned(),
        ],
    }
}

pub(super) fn tui_plain_item(key: &str, value: String) -> TuiKeyValueSpec {
    TuiKeyValueSpec::Plain {
        key: key.to_owned(),
        value,
    }
}

pub(super) fn tui_csv_item(key: &str, values: Vec<String>) -> TuiKeyValueSpec {
    TuiKeyValueSpec::Csv {
        key: key.to_owned(),
        values,
    }
}

pub(super) fn csv_values_or_dash(values: Vec<String>) -> Vec<String> {
    if values.is_empty() {
        return vec!["-".to_owned()];
    }

    values
}

pub(super) fn collect_rollup_values(
    counts: &std::collections::BTreeMap<String, u32>,
) -> Vec<String> {
    counts
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect()
}

pub(super) fn bool_yes_no_value(value: bool) -> String {
    if value {
        return "yes".to_owned();
    }

    "no".to_owned()
}

pub(super) fn recovery_callout_tone(recovery_needed: bool) -> TuiCalloutTone {
    if recovery_needed {
        return TuiCalloutTone::Warning;
    }

    TuiCalloutTone::Success
}

pub(super) fn safe_lane_health_tone(severity: &str) -> TuiCalloutTone {
    if severity == "critical" || severity == "warn" {
        return TuiCalloutTone::Warning;
    }

    if severity == "ok" {
        return TuiCalloutTone::Success;
    }

    TuiCalloutTone::Info
}
