use loong_contracts::SecretRef;

use crate::CliResult;

use super::super::shared::{
    ConfigValidationIssue, EnvPointerValidationHint, validate_env_pointer_field,
    validate_secret_ref_env_pointer_field,
};
use super::{
    EMAIL_IMAP_PASSWORD_ENV, EMAIL_IMAP_USERNAME_ENV, EMAIL_SMTP_PASSWORD_ENV,
    EMAIL_SMTP_USERNAME_ENV, EmailSmtpEndpoint,
};

pub(crate) fn parse_email_smtp_endpoint(raw: &str) -> CliResult<EmailSmtpEndpoint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("email smtp_host is empty".to_owned());
    }

    if trimmed.contains("://") {
        let parsed_url = reqwest::Url::parse(trimmed)
            .map_err(|error| format!("email smtp_host url is invalid: {error}"))?;
        let scheme = parsed_url.scheme();
        if scheme != "smtp" && scheme != "smtps" {
            return Err(format!(
                "email smtp_host url must use smtp:// or smtps://, got {scheme}://"
            ));
        }

        let host = parsed_url
            .host_str()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if host.is_none() {
            return Err("email smtp_host url is missing a host".to_owned());
        }

        return Ok(EmailSmtpEndpoint::ConnectionUrl(trimmed.to_owned()));
    }

    if trimmed.chars().any(char::is_whitespace) {
        return Err("email smtp_host must not contain whitespace".to_owned());
    }
    if trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        return Err(
            "email smtp_host must be a bare host or a full smtp:// or smtps:// URL".to_owned(),
        );
    }
    if trimmed.contains(':') {
        return Err(
            "email smtp_host with an explicit port must use a full smtp:// or smtps:// URL"
                .to_owned(),
        );
    }

    Ok(EmailSmtpEndpoint::RelayHost(trimmed.to_owned()))
}

pub(super) fn validate_email_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("imap_username_env") {
        EMAIL_IMAP_USERNAME_ENV
    } else if field_path.ends_with("imap_password_env") {
        EMAIL_IMAP_PASSWORD_ENV
    } else if field_path.ends_with("smtp_password_env") {
        EMAIL_SMTP_PASSWORD_ENV
    } else {
        EMAIL_SMTP_USERNAME_ENV
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

pub(super) fn validate_email_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("imap_username") {
        EMAIL_IMAP_USERNAME_ENV
    } else if field_path.ends_with("imap_password") {
        EMAIL_IMAP_PASSWORD_ENV
    } else if field_path.ends_with("smtp_password") {
        EMAIL_SMTP_PASSWORD_ENV
    } else {
        EMAIL_SMTP_USERNAME_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}
