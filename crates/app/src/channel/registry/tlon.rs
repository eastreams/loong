use super::*;

const TLON_SHIP_ENV: &str = "TLON_SHIP";
const TLON_URL_ENV: &str = "TLON_URL";
const TLON_CODE_ENV: &str = "TLON_CODE";

const TLON_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["tlon.enabled", "tlon.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TLON_SHIP_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "ship",
        label: "ship",
        config_paths: &["tlon.ship", "tlon.accounts.<account>.ship"],
        env_pointer_paths: &["tlon.ship_env", "tlon.accounts.<account>.ship_env"],
        default_env_var: Some(TLON_SHIP_ENV),
    };
const TLON_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "url",
        label: "ship url",
        config_paths: &["tlon.url", "tlon.accounts.<account>.url"],
        env_pointer_paths: &["tlon.url_env", "tlon.accounts.<account>.url_env"],
        default_env_var: Some(TLON_URL_ENV),
    };
const TLON_CODE_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "code",
        label: "login code",
        config_paths: &["tlon.code", "tlon.accounts.<account>.code"],
        env_pointer_paths: &["tlon.code_env", "tlon.accounts.<account>.code_env"],
        default_env_var: Some(TLON_CODE_ENV),
    };
const TLON_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TLON_ENABLED_REQUIREMENT,
    TLON_SHIP_REQUIREMENT,
    TLON_URL_REQUIREMENT,
    TLON_CODE_REQUIREMENT,
];
const TLON_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TLON_ENABLED_REQUIREMENT,
    TLON_SHIP_REQUIREMENT,
    TLON_URL_REQUIREMENT,
    TLON_CODE_REQUIREMENT,
];

pub(super) const TLON_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "ship message send",
    command: "tlon-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: TLON_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
pub(super) const TLON_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "ship event service",
    command: "tlon-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: TLON_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "tlon",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: TLON_SEND_OPERATION,
        serve: TLON_SERVE_OPERATION,
    };

pub(super) const TLON_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];

pub(super) const TLON_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure a Tlon ship account in loongclaw.toml under tlon or tlon.accounts.<account>; outbound ship sends are shipped for DMs and chat groups, while inbound serve support remains planned",
        status_command: "loongclaw doctor",
        repair_command: Some("loongclaw doctor --fix"),
    };

pub(super) const TLON_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "tlon",
        runtime: None,
        snapshot_builder: Some(build_tlon_snapshots),
        selection_order: 205,
        selection_label: "urbit ship bot",
        blurb: "Shipped Tlon outbound surface with config-backed Urbit DMs and group sends through a ship-backed poke API; inbound serve support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Tlon",
        aliases: &["urbit"],
        transport: "tlon_urbit_ship_api",
        onboarding: TLON_ONBOARDING_DESCRIPTOR,
        operations: TLON_OPERATIONS,
    };

pub(super) fn build_tlon_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
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
            let resolution_result = config
                .tlon
                .resolve_account(Some(configured_account_id.as_str()));

            match resolution_result {
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

fn build_tlon_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedTlonChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let ship = resolved.ship();
    let normalized_ship_result = ship.as_deref().map(normalize_tlon_status_ship).transpose();

    if ship.is_none() {
        send_issues.push("ship is missing".to_owned());
    }

    if let Err(error) = normalized_ship_result.as_ref() {
        send_issues.push(error.clone());
    }

    let url = resolved.url();
    let normalized_url_result = url.as_deref().map(normalize_tlon_status_url).transpose();

    if url.is_none() {
        send_issues.push("url is missing".to_owned());
    }

    if let Err(error) = normalized_url_result.as_ref() {
        send_issues.push(error.clone());
    }

    if resolved.code().is_none() {
        send_issues.push("code is missing".to_owned());
    }

    let send_operation = if !compiled {
        let detail = "binary built without feature `channel-tlon`".to_owned();
        unsupported_operation(TLON_SEND_OPERATION, detail)
    } else if !resolved.enabled {
        let detail = "disabled by tlon account configuration".to_owned();
        disabled_operation(TLON_SEND_OPERATION, detail)
    } else if !send_issues.is_empty() {
        misconfigured_operation(TLON_SEND_OPERATION, send_issues)
    } else {
        ready_operation(TLON_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        let detail = "binary built without feature `channel-tlon`".to_owned();
        unsupported_operation(TLON_SERVE_OPERATION, detail)
    } else {
        let detail = "tlon inbound serve runtime is not implemented yet".to_owned();
        unsupported_operation(TLON_SERVE_OPERATION, detail)
    };

    let mut notes = Vec::new();
    let configured_account_id = resolved.configured_account_id.clone();
    let configured_account_label = resolved.configured_account_label.clone();
    let account_id = resolved.account.id.clone();
    let account_label = resolved.account.label.clone();

    notes.push(format!("configured_account_id={configured_account_id}"));
    notes.push(format!("configured_account={configured_account_label}"));
    notes.push(format!("account_id={account_id}"));
    notes.push(format!("account={account_label}"));

    let normalized_ship = normalized_ship_result.ok().flatten();

    if let Some(ship) = normalized_ship {
        notes.push(format!("ship={ship}"));
    }

    if is_default_account {
        notes.push("default_account=true".to_owned());
    }

    let default_account_source_text = default_account_source.as_str();
    notes.push(format!(
        "default_account_source={default_account_source_text}"
    ));

    let api_base_url = normalized_url_result.ok().flatten();

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id,
        configured_account_label,
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url,
        notes,
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
        let detail = "binary built without feature `channel-tlon`".to_owned();
        unsupported_operation(TLON_SEND_OPERATION, detail)
    } else {
        let issues = vec![error.clone()];
        misconfigured_operation(TLON_SEND_OPERATION, issues)
    };

    let serve_operation = if !compiled {
        let detail = "binary built without feature `channel-tlon`".to_owned();
        unsupported_operation(TLON_SERVE_OPERATION, detail)
    } else {
        let detail = "tlon inbound serve runtime is not implemented yet".to_owned();
        unsupported_operation(TLON_SERVE_OPERATION, detail)
    };

    let mut notes = Vec::new();
    let configured_account_id_text = configured_account_id.to_owned();
    let selection_error = error;

    notes.push(format!(
        "configured_account_id={configured_account_id_text}"
    ));
    notes.push(format!("selection_error={selection_error}"));

    if is_default_account {
        notes.push("default_account=true".to_owned());
    }

    let default_account_source_text = default_account_source.as_str();
    notes.push(format!(
        "default_account_source={default_account_source_text}"
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: configured_account_id_text.clone(),
        configured_account_label: configured_account_id_text,
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        operations: vec![send_operation, serve_operation],
    }
}

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

    let has_username = !parsed_url.username().is_empty();
    let has_password = parsed_url.password().is_some();

    if has_username || has_password {
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
