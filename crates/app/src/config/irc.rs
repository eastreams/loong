use std::collections::BTreeMap;

use loong_contracts::SecretRef;

use crate::CliResult;

use super::shared::{
    ConfigValidationCode, ConfigValidationIssue, ConfigValidationSeverity,
    EnvPointerValidationHint, validate_env_pointer_field, validate_secret_ref_env_pointer_field,
};

pub(crate) const IRC_SERVER_ENV: &str = "IRC_SERVER";
pub(crate) const IRC_NICKNAME_ENV: &str = "IRC_NICKNAME";
pub(crate) const IRC_PASSWORD_ENV: &str = "IRC_PASSWORD";

pub(super) fn default_irc_server_env() -> Option<String> {
    Some(IRC_SERVER_ENV.to_owned())
}

pub(super) fn default_irc_nickname_env() -> Option<String> {
    Some(IRC_NICKNAME_ENV.to_owned())
}

pub(super) fn default_irc_password_env() -> Option<String> {
    Some(IRC_PASSWORD_ENV.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrcServerTransport {
    Plain,
    Tls,
}

impl IrcServerTransport {
    pub(crate) const fn default_port(self) -> u16 {
        match self {
            Self::Plain => 6667,
            Self::Tls => 6697,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IrcServerEndpoint {
    pub(crate) transport: IrcServerTransport,
    pub(crate) host: String,
    pub(crate) port: u16,
}

pub(crate) fn parse_irc_server_endpoint(raw: &str) -> CliResult<IrcServerEndpoint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("irc server is empty".to_owned());
    }

    if trimmed.contains("://") {
        let url = reqwest::Url::parse(trimmed)
            .map_err(|error| format!("irc server url is invalid: {error}"))?;
        let transport = match url.scheme() {
            "irc" => IrcServerTransport::Plain,
            "ircs" => IrcServerTransport::Tls,
            scheme => {
                return Err(format!(
                    "unsupported irc server scheme `{scheme}`; expected `irc` or `ircs`"
                ));
            }
        };

        if !url.username().is_empty() || url.password().is_some() {
            return Err("irc server url must not include credentials".to_owned());
        }
        if url.query().is_some() {
            return Err("irc server url must not include a query".to_owned());
        }
        if url.fragment().is_some() {
            return Err("irc server url must not include a fragment".to_owned());
        }

        let path = url.path();
        if !(path.is_empty() || path == "/") {
            return Err("irc server url must not include a path".to_owned());
        }

        let host = url
            .host_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "irc server url is missing a host".to_owned())?;
        let port = url.port().unwrap_or_else(|| transport.default_port());
        if port == 0 {
            return Err("irc server port must be between 1 and 65535".to_owned());
        }
        return Ok(IrcServerEndpoint {
            transport,
            host: host.to_owned(),
            port,
        });
    }

    if trimmed.contains(':') {
        return Err(
            "bare `host:port` is not supported for irc server; use `irc://host:port` or `ircs://host:port`"
                .to_owned(),
        );
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err("bare irc server must not contain whitespace".to_owned());
    }
    if trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        return Err(
            "bare irc server must be a hostname only; use `irc://` or `ircs://` for url-style values"
                .to_owned(),
        );
    }

    Ok(IrcServerEndpoint {
        transport: IrcServerTransport::Plain,
        host: trimmed.to_owned(),
        port: IrcServerTransport::Plain.default_port(),
    })
}

pub(super) fn validate_irc_server_field(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    server: Option<String>,
) {
    let Some(server) = server else {
        return;
    };

    let parse_result = parse_irc_server_endpoint(server.as_str());
    let Err(invalid_reason) = parse_result else {
        return;
    };

    let suggested_fix =
        "use a bare host or an explicit `irc://host[:port]` / `ircs://host[:port]` endpoint";
    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert("invalid_reason".to_owned(), invalid_reason);
    extra_message_variables.insert("suggested_fix".to_owned(), suggested_fix.to_owned());

    issues.push(ConfigValidationIssue {
        severity: ConfigValidationSeverity::Error,
        code: ConfigValidationCode::InvalidValue,
        field_path: field_path.to_owned(),
        inline_field_path: field_path.to_owned(),
        example_env_name: IRC_SERVER_ENV.to_owned(),
        suggested_env_name: None,
        extra_message_variables,
    });
}

pub(super) fn validate_irc_nickname_field(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    nickname: Option<String>,
) {
    let Some(nickname) = nickname else {
        return;
    };

    let contains_whitespace = nickname.chars().any(char::is_whitespace);
    let contains_null = nickname.contains('\0');
    let invalid_reason = if contains_whitespace {
        Some("nickname must not contain whitespace")
    } else if contains_null {
        Some("nickname contains forbidden control characters")
    } else {
        None
    };
    let Some(invalid_reason) = invalid_reason else {
        return;
    };

    let suggested_fix = "use a single-token IRC nickname (for example: `loong_bot`)";
    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert("invalid_reason".to_owned(), invalid_reason.to_owned());
    extra_message_variables.insert("suggested_fix".to_owned(), suggested_fix.to_owned());

    issues.push(ConfigValidationIssue {
        severity: ConfigValidationSeverity::Error,
        code: ConfigValidationCode::InvalidValue,
        field_path: field_path.to_owned(),
        inline_field_path: field_path.to_owned(),
        example_env_name: IRC_NICKNAME_ENV.to_owned(),
        suggested_env_name: None,
        extra_message_variables,
    });
}

pub(super) fn validate_irc_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("server_env") {
        IRC_SERVER_ENV
    } else if field_path.ends_with("nickname_env") {
        IRC_NICKNAME_ENV
    } else {
        IRC_PASSWORD_ENV
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

pub(super) fn validate_irc_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: IRC_PASSWORD_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}
