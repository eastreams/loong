use reqwest::header::{HeaderName, HeaderValue};

use crate::CliResult;

pub(crate) fn build_webhook_auth_header_from_parts(
    auth_token: Option<&str>,
    auth_header_name: &str,
    auth_token_prefix: &str,
) -> CliResult<Option<(HeaderName, HeaderValue)>> {
    let Some(auth_token) = auth_token else {
        return Ok(None);
    };

    let trimmed_token = auth_token.trim();
    if trimmed_token.is_empty() {
        return Err("webhook auth_token is empty".to_owned());
    }

    let header_name_raw = auth_header_name.trim();
    if header_name_raw.is_empty() {
        return Err("webhook auth_header_name is empty".to_owned());
    }

    let header_name = HeaderName::from_bytes(header_name_raw.as_bytes())
        .map_err(|error| format!("webhook auth_header_name is invalid: {error}"))?;
    let header_value_raw = format!("{auth_token_prefix}{trimmed_token}");
    let header_value = HeaderValue::from_str(header_value_raw.as_str())
        .map_err(|error| format!("webhook auth header value is invalid: {error}"))?;

    Ok(Some((header_name, header_value)))
}

#[cfg(test)]
mod tests {
    use super::build_webhook_auth_header_from_parts;

    #[test]
    fn build_webhook_auth_header_from_parts_rejects_blank_auth_token() {
        let error = build_webhook_auth_header_from_parts(Some("   "), "Authorization", "Bearer ")
            .expect_err("blank auth token should fail");

        assert!(
            error.contains("webhook auth_token is empty"),
            "unexpected error: {error}"
        );
    }
}
