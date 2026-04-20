use super::tlon::{TLON_SEND_OPERATION, TLON_SERVE_OPERATION};
use super::*;

fn normalize_tlon_status_ship(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("ship is empty".to_owned());
    }

    let ship_body = trimmed.trim_start_matches('~');
    if ship_body.is_empty() {
        return Err("ship is empty".to_owned());
    }

    let has_invalid_character = ship_body.chars().any(|value| {
        let is_letter = value.is_ascii_alphabetic();
        let is_separator = value == '-';
        !is_letter && !is_separator
    });
    if has_invalid_character {
        return Err("ship must contain only letters and `-`".to_owned());
    }

    let normalized_ship = ship_body.to_ascii_lowercase();
    let ship = format!("~{normalized_ship}");
    Ok(ship)
}

fn normalize_tlon_status_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("url is empty".to_owned());
    }

    let has_scheme = trimmed.contains("://");
    let candidate = if has_scheme {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };

    let parsed_url = reqwest::Url::parse(candidate.as_str())
        .map_err(|error| format!("url is invalid: {error}"))?;
    let scheme = parsed_url.scheme();
    let is_http = scheme == "http";
    let is_https = scheme == "https";
    if !is_http && !is_https {
        return Err(format!("url must use http or https, got {scheme}"));
    }
    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        return Err("url must not include credentials".to_owned());
    }

    let path = parsed_url.path();
    let has_non_root_path = path != "/" && !path.is_empty();
    let has_query = parsed_url.query().is_some();
    let has_fragment = parsed_url.fragment().is_some();
    if has_non_root_path || has_query || has_fragment {
        return Err("url must not include a path, query, or fragment".to_owned());
    }

    let hostname = parsed_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "url hostname is invalid".to_owned())?;
    let normalized_hostname = hostname.to_ascii_lowercase();
    let normalized_hostname = normalized_hostname.trim_end_matches('.');
    if normalized_hostname.is_empty() {
        return Err("url hostname is invalid".to_owned());
    }

    let port = parsed_url.port();
    let is_ipv6 = normalized_hostname.contains(':');
    let host = if let Some(port) = port {
        if is_ipv6 {
            format!("[{normalized_hostname}]:{port}")
        } else {
            format!("{normalized_hostname}:{port}")
        }
    } else if is_ipv6 {
        format!("[{normalized_hostname}]")
    } else {
        normalized_hostname.to_owned()
    };

    let normalized_url = format!("{scheme}://{host}");
    Ok(normalized_url)
}

fn build_tlon_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedTlonChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let ship = resolved.ship();
    let normalized_ship = ship.as_deref().map(normalize_tlon_status_ship).transpose();
    if ship.is_none() {
        send_issues.push("ship is missing".to_owned());
    }
    if let Err(error) = normalized_ship.as_ref() {
        send_issues.push(error.clone());
    }

    let url = resolved.url();
    let normalized_url = url.as_deref().map(normalize_tlon_status_url).transpose();
    if url.is_none() {
        send_issues.push("url is missing".to_owned());
    }
    if let Err(error) = normalized_url.as_ref() {
        send_issues.push(error.clone());
    }

    if resolved.code().is_none() {
        send_issues.push("code is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            TLON_SEND_OPERATION,
            "binary built without feature `channel-tlon`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TLON_SEND_OPERATION,
            "disabled by tlon account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(TLON_SEND_OPERATION, send_issues)
    } else {
        ready_operation(TLON_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            TLON_SERVE_OPERATION,
            "binary built without feature `channel-tlon`".to_owned(),
        )
    } else {
        unsupported_operation(
            TLON_SERVE_OPERATION,
            "tlon inbound serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if let Ok(Some(ship)) = normalized_ship.as_ref() {
        notes.push(format!("ship={ship}"));
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    let api_base_url = normalized_url.ok().flatten();

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_tlon_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            TLON_SEND_OPERATION,
            "binary built without feature `channel-tlon`".to_owned(),
        )
    } else {
        misconfigured_operation(TLON_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            TLON_SERVE_OPERATION,
            "binary built without feature `channel-tlon`".to_owned(),
        )
    } else {
        unsupported_operation(
            TLON_SERVE_OPERATION,
            "tlon inbound serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
    ];
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_tlon_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-tlon");
    let default_selection = config.tlon.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .tlon
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .tlon
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_tlon_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                ),
                Err(error) => build_invalid_tlon_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}
