use super::*;

const TWITCH_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["twitch.enabled", "twitch.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TWITCH_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "access_token",
        label: "user access token",
        config_paths: &[
            "twitch.access_token",
            "twitch.accounts.<account>.access_token",
        ],
        env_pointer_paths: &[
            "twitch.access_token_env",
            "twitch.accounts.<account>.access_token_env",
        ],
        default_env_var: Some(TWITCH_ACCESS_TOKEN_ENV),
    };
const TWITCH_CHANNEL_NAMES_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "channel_names",
        label: "channel names",
        config_paths: &[
            "twitch.channel_names",
            "twitch.accounts.<account>.channel_names",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TWITCH_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    &[TWITCH_ENABLED_REQUIREMENT, TWITCH_ACCESS_TOKEN_REQUIREMENT];
const TWITCH_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TWITCH_ENABLED_REQUIREMENT,
    TWITCH_ACCESS_TOKEN_REQUIREMENT,
    TWITCH_CHANNEL_NAMES_REQUIREMENT,
];
const TWITCH_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "chat send",
    command: "twitch-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: TWITCH_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TWITCH_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "chat listener",
    command: "twitch-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: TWITCH_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "twitch",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: TWITCH_SEND_OPERATION,
        serve: TWITCH_SERVE_OPERATION,
    };

pub(super) const TWITCH_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
pub(super) const TWITCH_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure a Twitch user access token in loongclaw.toml under twitch or twitch.accounts.<account>; outbound chat sends are shipped via the Twitch Chat API, while inbound EventSub or chat-listener support remains planned",
        status_command: "loongclaw doctor",
        repair_command: Some("loongclaw doctor --fix"),
    };

pub(super) fn build_twitch_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-twitch");
    let default_selection = config.twitch.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;

    config
        .twitch
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            let resolution_result = config
                .twitch
                .resolve_account(Some(configured_account_id.as_str()));

            match resolution_result {
                Ok(resolved) => build_twitch_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                ),
                Err(error) => build_invalid_twitch_snapshot(
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

fn build_twitch_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedTwitchChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.access_token().is_none() {
        send_issues.push("access_token is missing".to_owned());
    }

    let api_base_url = resolved.resolved_api_base_url();
    validate_http_url("api_base_url", api_base_url.as_str(), &mut send_issues);
    let status_api_base_url = redact_endpoint_status_url(Some(api_base_url.clone()))
        .unwrap_or_else(|| api_base_url.clone());

    let oauth_base_url = resolved.resolved_oauth_base_url();
    validate_http_url("oauth_base_url", oauth_base_url.as_str(), &mut send_issues);
    let status_oauth_base_url = redact_endpoint_status_url(Some(oauth_base_url.clone()))
        .unwrap_or_else(|| oauth_base_url.clone());

    let send_operation = if !compiled {
        unsupported_operation(
            TWITCH_SEND_OPERATION,
            "binary built without feature `channel-twitch`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TWITCH_SEND_OPERATION,
            "disabled by twitch account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(TWITCH_SEND_OPERATION, send_issues)
    } else {
        ready_operation(TWITCH_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            TWITCH_SERVE_OPERATION,
            "binary built without feature `channel-twitch`".to_owned(),
        )
    } else {
        unsupported_operation(
            TWITCH_SERVE_OPERATION,
            "twitch EventSub or chat-listener serve support is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("oauth_base_url={status_oauth_base_url}"),
    ];
    if !resolved.channel_names.is_empty() {
        let future_serve_channel_names = resolved.channel_names.join(",");
        notes.push(format!(
            "future_serve_channel_names={future_serve_channel_names}"
        ));
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

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
        api_base_url: Some(status_api_base_url),
        notes,
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_twitch_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            TWITCH_SEND_OPERATION,
            "binary built without feature `channel-twitch`".to_owned(),
        )
    } else {
        misconfigured_operation(TWITCH_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            TWITCH_SERVE_OPERATION,
            "binary built without feature `channel-twitch`".to_owned(),
        )
    } else {
        unsupported_operation(
            TWITCH_SERVE_OPERATION,
            "twitch EventSub or chat-listener serve support is not implemented yet".to_owned(),
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
        operations: vec![send_operation, serve_operation],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitch_status_reports_ready_send_and_stub_serve() {
        let mut config = LoongClawConfig::default();
        config.twitch.enabled = true;
        config.twitch.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "twitch-user-token".to_owned(),
        ));
        config.twitch.channel_names = vec!["streamer-a".to_owned()];

        let snapshots = channel_status_snapshots(&config);
        let twitch = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "twitch")
            .expect("twitch snapshot");
        let send = twitch.operation("send").expect("twitch send operation");
        let serve = twitch.operation("serve").expect("twitch serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert_eq!(
            twitch.api_base_url.as_deref(),
            Some("https://api.twitch.tv/helix")
        );
        assert!(
            twitch
                .notes
                .iter()
                .any(|note| note == "oauth_base_url=https://id.twitch.tv/oauth2"),
            "status notes should expose the resolved oauth base url"
        );
        assert!(
            twitch
                .notes
                .iter()
                .any(|note| note == "future_serve_channel_names=streamer-a"),
            "status notes should retain future serve channel names"
        );
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_none());
    }

    #[test]
    fn twitch_status_redacts_override_urls_in_snapshot_output() {
        let mut config = LoongClawConfig::default();
        config.twitch.enabled = true;
        config.twitch.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "twitch-user-token".to_owned(),
        ));
        config.twitch.api_base_url =
            Some("https://user:secret@api.twitch.test/helix?token=secret".to_owned());
        config.twitch.oauth_base_url =
            Some("https://oauth:secret@id.twitch.test/oauth2?client=secret".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let twitch = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "twitch")
            .expect("twitch snapshot");

        assert_eq!(
            twitch.api_base_url.as_deref(),
            Some("https://api.twitch.test/helix")
        );
        assert!(
            twitch
                .notes
                .iter()
                .any(|note| note == "oauth_base_url=https://id.twitch.test/oauth2"),
            "status notes should redact override oauth urls"
        );
        assert!(
            twitch.notes.iter().all(|note| !note.contains("secret")),
            "status output should not leak override credentials"
        );
    }
}
