use crate::CliResult;

use super::{
    ChannelAccountIdentity, ChannelAccountIdentitySource, ChannelAcpConfig,
    ChannelDefaultAccountSelection, ChannelDefaultAccountSelectionSource,
    ChannelResolvedAccountRoute,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedConfiguredAccount {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) account_key: Option<String>,
}

pub(super) fn default_channel_account_identity() -> ChannelAccountIdentity {
    ChannelAccountIdentity {
        id: "default".to_owned(),
        label: "default".to_owned(),
        source: ChannelAccountIdentitySource::Default,
    }
}

pub(super) fn resolve_configured_account_identity(raw: Option<&str>) -> Option<(String, String)> {
    let label = raw.map(str::trim).filter(|value| !value.is_empty())?;
    if !label.chars().any(|value| value.is_ascii_alphanumeric()) {
        return None;
    }
    Some((normalize_channel_account_id(label), label.to_owned()))
}

pub(super) fn resolve_telegram_bot_id_from_token(token: &str) -> Option<&str> {
    let bot_id = token.split(':').next()?.trim();
    if bot_id.is_empty() || !bot_id.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    Some(bot_id)
}

pub(super) fn resolve_string_with_legacy_env(
    raw: Option<&str>,
    env_key: Option<&str>,
) -> Option<String> {
    let inline = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if inline.is_some() {
        return inline;
    }

    let env_name = env_key.map(str::trim).filter(|value| !value.is_empty())?;
    let env_value = std::env::var(env_name).ok()?;
    let trimmed_value = env_value.trim();
    if trimmed_value.is_empty() {
        return None;
    }
    Some(trimmed_value.to_owned())
}

pub(super) fn resolve_string_list_with_legacy_env(
    raw: Option<&[String]>,
    env_key: Option<&str>,
) -> Vec<String> {
    let inline = raw.map(normalize_inline_string_list).unwrap_or_default();
    if !inline.is_empty() {
        return inline;
    }

    let env_name = env_key.map(str::trim).filter(|value| !value.is_empty());
    let Some(env_name) = env_name else {
        return Vec::new();
    };
    let env_value = std::env::var(env_name).ok();
    let Some(env_value) = env_value else {
        return Vec::new();
    };
    parse_env_string_list(env_value.as_str())
}

pub(super) fn normalize_inline_string_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_env_string_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    trimmed
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn normalize_channel_account_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "default".to_owned();
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_was_separator = false;
    for value in trimmed.chars() {
        if value.is_ascii_alphanumeric() {
            normalized.push(value.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }
        if matches!(value, '_' | '-') {
            if !normalized.is_empty() && !last_was_separator {
                normalized.push(value);
                last_was_separator = true;
            }
            continue;
        }
        if !normalized.is_empty() && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }

    while matches!(normalized.chars().last(), Some('-' | '_')) {
        normalized.pop();
    }

    if normalized.is_empty() {
        "default".to_owned()
    } else {
        normalized
    }
}

pub(super) fn configured_account_ids<'a, I>(keys: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut ids = keys
        .into_iter()
        .map(|value| normalize_channel_account_id(value))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn normalize_optional_account_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_channel_account_id)
}

fn resolve_default_configured_account_selection_from_ids(
    ids: &[String],
    preferred: Option<&str>,
    fallback: &str,
) -> ChannelDefaultAccountSelection {
    if let Some(preferred) = normalize_optional_account_id(preferred)
        && !ids.is_empty()
        && ids.iter().any(|value| value == &preferred)
    {
        return ChannelDefaultAccountSelection {
            id: preferred,
            source: ChannelDefaultAccountSelectionSource::ExplicitDefault,
        };
    }
    if ids.is_empty() {
        return ChannelDefaultAccountSelection {
            id: normalize_channel_account_id(fallback),
            source: ChannelDefaultAccountSelectionSource::RuntimeIdentity,
        };
    }
    if ids.iter().any(|value| value == "default") {
        return ChannelDefaultAccountSelection {
            id: "default".to_owned(),
            source: ChannelDefaultAccountSelectionSource::MappedDefault,
        };
    }
    ChannelDefaultAccountSelection {
        id: ids
            .first()
            .cloned()
            .unwrap_or_else(|| normalize_channel_account_id(fallback)),
        source: ChannelDefaultAccountSelectionSource::Fallback,
    }
}

pub(super) fn resolve_default_configured_account_selection<'a, I>(
    keys: I,
    preferred: Option<&str>,
    fallback: &str,
) -> ChannelDefaultAccountSelection
where
    I: IntoIterator<Item = &'a String>,
{
    let ids = configured_account_ids(keys);
    resolve_default_configured_account_selection_from_ids(ids.as_slice(), preferred, fallback)
}

pub(super) fn resolve_channel_account_route<'a, I>(
    keys: I,
    preferred: Option<&str>,
    fallback: &str,
    requested_account_id: Option<&str>,
    selected_configured_account_id: &str,
) -> ChannelResolvedAccountRoute
where
    I: IntoIterator<Item = &'a String>,
{
    let ids = configured_account_ids(keys);
    let default_selection =
        resolve_default_configured_account_selection_from_ids(ids.as_slice(), preferred, fallback);
    ChannelResolvedAccountRoute {
        requested_account_id: normalize_optional_account_id(requested_account_id),
        configured_account_count: ids.len(),
        selected_configured_account_id: normalize_channel_account_id(
            selected_configured_account_id,
        ),
        default_account_source: default_selection.source,
    }
}

pub(super) fn resolve_channel_acp_config(
    base: &ChannelAcpConfig,
    account_override: Option<&ChannelAcpConfig>,
) -> ChannelAcpConfig {
    account_override.cloned().unwrap_or_else(|| base.clone())
}

pub(super) fn resolve_account_for_session_account_id<R>(
    session_account_id: Option<&str>,
    resolve_direct: impl FnOnce() -> CliResult<R>,
    configured_ids: impl FnOnce() -> Vec<String>,
    resolve_configured: impl Fn(&str) -> CliResult<R>,
    runtime_account_id: impl Fn(&R) -> &str,
) -> CliResult<R> {
    let Some(requested) = normalize_optional_account_id(session_account_id) else {
        return resolve_direct();
    };

    match resolve_direct() {
        Ok(resolved) => Ok(resolved),
        Err(original_error) => {
            for configured_id in configured_ids() {
                let resolved = resolve_configured(configured_id.as_str())?;
                if normalize_channel_account_id(runtime_account_id(&resolved)) == requested {
                    return Ok(resolved);
                }
            }
            Err(original_error)
        }
    }
}

pub(super) fn resolve_configured_account_selection<'a, I>(
    keys: I,
    requested_account_id: Option<&str>,
    preferred_default_account_id: Option<&str>,
    fallback_id: &str,
) -> CliResult<ResolvedConfiguredAccount>
where
    I: IntoIterator<Item = &'a String>,
{
    let entries = keys
        .into_iter()
        .filter_map(|value| {
            let raw_key = value.to_owned();
            let label = value.trim();
            if label.is_empty() {
                return None;
            }
            Some((
                normalize_channel_account_id(label),
                label.to_owned(),
                raw_key,
            ))
        })
        .collect::<Vec<_>>();
    let configured_ids = entries
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect::<Vec<_>>();

    if let Some(requested) = requested_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_channel_account_id)
    {
        if entries.is_empty() {
            return Ok(ResolvedConfiguredAccount {
                label: requested.clone(),
                id: requested,
                account_key: None,
            });
        }
        let Some((id, label, raw_key)) = entries.iter().find(|(id, _, _)| *id == requested) else {
            return Err(format!(
                "requested account `{requested}` is not configured (configured accounts: {})",
                configured_ids.join(", ")
            ));
        };
        return Ok(ResolvedConfiguredAccount {
            id: id.clone(),
            label: label.clone(),
            account_key: Some(raw_key.clone()),
        });
    }

    let default_id = resolve_default_configured_account_selection(
        entries.iter().map(|(_, _, raw_key)| raw_key),
        preferred_default_account_id,
        fallback_id,
    )
    .id;
    if let Some((id, label, raw_key)) = entries.iter().find(|(id, _, _)| *id == default_id) {
        return Ok(ResolvedConfiguredAccount {
            id: id.clone(),
            label: label.clone(),
            account_key: Some(raw_key.clone()),
        });
    }

    Ok(ResolvedConfiguredAccount {
        id: default_id.clone(),
        label: default_id,
        account_key: None,
    })
}
