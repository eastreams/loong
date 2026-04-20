use super::*;

pub(super) fn default_tlon_ship_env() -> Option<String> {
    Some(TLON_SHIP_ENV.to_owned())
}

pub(super) fn default_tlon_url_env() -> Option<String> {
    Some(TLON_URL_ENV.to_owned())
}

pub(super) fn default_tlon_code_env() -> Option<String> {
    Some(TLON_CODE_ENV.to_owned())
}

pub(super) fn validate_tlon_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("ship_env") {
        TLON_SHIP_ENV
    } else if field_path.ends_with("url_env") {
        TLON_URL_ENV
    } else {
        TLON_CODE_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_tlon_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: TLON_CODE_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}
