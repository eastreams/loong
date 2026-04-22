use super::*;

#[test]
fn resolve_validate_output_defaults_to_text() {
    let resolved = resolve_validate_output(false, None).expect("resolve default output");
    assert_eq!(resolved, ValidateConfigOutput::Text);
}

#[test]
fn resolve_validate_output_uses_json_flag_legacy_alias() {
    let resolved = resolve_validate_output(true, None).expect("resolve json output");
    assert_eq!(resolved, ValidateConfigOutput::Json);
}

#[test]
fn resolve_validate_output_accepts_explicit_problem_json() {
    let resolved = resolve_validate_output(false, Some(ValidateConfigOutput::ProblemJson))
        .expect("resolve problem-json output");
    assert_eq!(resolved, ValidateConfigOutput::ProblemJson);
}

#[test]
fn resolve_validate_output_rejects_conflicting_json_and_output_flags() {
    let error = resolve_validate_output(true, Some(ValidateConfigOutput::Json))
        .expect_err("conflicting flags should fail");
    assert!(error.contains("conflicts"));
}

#[test]
fn validation_summary_treats_warning_only_diagnostics_as_valid() {
    let summary = summarize_validation_diagnostics(&[validation_diagnostic_with_severity(
        "warn",
        "config.provider_selection.implicit_active",
    )]);

    assert!(summary.valid);
    assert_eq!(summary.error_count, 0);
    assert_eq!(summary.warning_count, 1);
}

#[test]
fn validation_summary_counts_error_and_warning_diagnostics_separately() {
    let summary = summarize_validation_diagnostics(&[
        validation_diagnostic_with_severity("error", "config.env_pointer.dollar_prefix"),
        validation_diagnostic_with_severity("warn", "config.provider_selection.implicit_active"),
    ]);

    assert!(!summary.valid);
    assert_eq!(summary.error_count, 1);
    assert_eq!(summary.warning_count, 1);
}
