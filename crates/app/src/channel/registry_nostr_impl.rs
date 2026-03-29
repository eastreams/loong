use std::path::Path;

use super::*;

pub(super) const NOSTR_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["nostr.enabled", "nostr.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };

pub(super) const NOSTR_RELAY_URLS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "relay_urls",
        label: "relay urls",
        config_paths: &["nostr.relay_urls", "nostr.accounts.<account>.relay_urls"],
        env_pointer_paths: &[
            "nostr.relay_urls_env",
            "nostr.accounts.<account>.relay_urls_env",
        ],
        default_env_var: Some(NOSTR_RELAY_URLS_ENV),
    };

pub(super) const NOSTR_PRIVATE_KEY_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "private_key",
        label: "private key",
        config_paths: &["nostr.private_key", "nostr.accounts.<account>.private_key"],
        env_pointer_paths: &[
            "nostr.private_key_env",
            "nostr.accounts.<account>.private_key_env",
        ],
        default_env_var: Some(NOSTR_PRIVATE_KEY_ENV),
    };

pub(super) const NOSTR_ALLOWED_PUBKEYS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_pubkeys",
        label: "allowed pubkeys",
        config_paths: &[
            "nostr.allowed_pubkeys",
            "nostr.accounts.<account>.allowed_pubkeys",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };

pub(super) const NOSTR_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NOSTR_ENABLED_REQUIREMENT,
    NOSTR_RELAY_URLS_REQUIREMENT,
    NOSTR_PRIVATE_KEY_REQUIREMENT,
];

pub(super) const NOSTR_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NOSTR_ENABLED_REQUIREMENT,
    NOSTR_RELAY_URLS_REQUIREMENT,
    NOSTR_PRIVATE_KEY_REQUIREMENT,
    NOSTR_ALLOWED_PUBKEYS_REQUIREMENT,
];

pub(super) const NOSTR_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "relay publish",
    command: "nostr-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: NOSTR_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};

pub(super) const NOSTR_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "relay subscriber",
    command: "nostr-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: NOSTR_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};

pub const NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "nostr",
        default_send_target_kind: ChannelCatalogTargetKind::Address,
        send: NOSTR_SEND_OPERATION,
        serve: NOSTR_SERVE_OPERATION,
    };

pub(super) const NOSTR_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];

pub(super) const NOSTR_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure relay_urls and private_key in loongclaw.toml under nostr or nostr.accounts.<account>; outbound signed note publish is shipped, while inbound relay subscription support remains planned",
        status_command: "loongclaw doctor",
        repair_command: Some("loongclaw doctor --fix"),
    };

pub(super) fn build_nostr_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-nostr");
    let default_selection = config.nostr.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;

    config
        .nostr
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .nostr
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_nostr_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                ),
                Err(error) => build_invalid_nostr_snapshot(
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

fn validate_websocket_url(field: &str, value: &str, issues: &mut Vec<String>) {
    let parsed_url = reqwest::Url::parse(value);
    let url = match parsed_url {
        Ok(url) => url,
        Err(error) => {
            let issue = format!("{field} is invalid: {error}");
            issues.push(issue);
            return;
        }
    };

    let scheme = url.scheme();
    let is_ws = scheme == "ws";
    let is_wss = scheme == "wss";
    if is_ws || is_wss {
        return;
    }

    let issue = format!("{field} must use ws or wss, got {scheme}");
    issues.push(issue);
}

fn build_nostr_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedNostrChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let relay_urls = resolved.relay_urls();
    if relay_urls.is_empty() {
        send_issues.push("relay_urls is empty".to_owned());
    }
    for (relay_url_index, relay_url) in relay_urls.iter().enumerate() {
        let relay_url_field = format!("relay_urls[{relay_url_index}]");
        validate_websocket_url(
            relay_url_field.as_str(),
            relay_url.as_str(),
            &mut send_issues,
        );
    }

    let normalized_private_key_hex = resolved.normalized_private_key_hex();
    let mut private_key_is_invalid = false;
    let private_key_hex = match normalized_private_key_hex {
        Ok(value) => value,
        Err(error) => {
            private_key_is_invalid = true;
            send_issues.push(format!("private_key is invalid: {error}"));
            None
        }
    };
    let public_key_hex = if private_key_is_invalid || private_key_hex.is_none() {
        None
    } else {
        match resolved.public_key_hex() {
            Ok(value) => value,
            Err(error) => {
                send_issues.push(format!("public_key is invalid: {error}"));
                None
            }
        }
    };
    if private_key_hex.is_none() && !private_key_is_invalid {
        send_issues.push("private_key is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            NOSTR_SEND_OPERATION,
            "binary built without feature `channel-nostr`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            NOSTR_SEND_OPERATION,
            "disabled by nostr account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(NOSTR_SEND_OPERATION, send_issues)
    } else {
        ready_operation(NOSTR_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            NOSTR_SERVE_OPERATION,
            "binary built without feature `channel-nostr`".to_owned(),
        )
    } else {
        unsupported_operation(
            NOSTR_SERVE_OPERATION,
            "nostr relay subscriber runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("relay_count={}", relay_urls.len()),
    ];
    if let Some(public_key_hex) = public_key_hex {
        notes.push(format!("public_key={public_key_hex}"));
    }
    let allowed_pubkeys = resolved.allowed_pubkeys();
    if !allowed_pubkeys.is_empty() {
        notes.push(format!("allowed_pubkeys_count={}", allowed_pubkeys.len()));
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
        api_base_url: None,
        notes,
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_nostr_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            NOSTR_SEND_OPERATION,
            "binary built without feature `channel-nostr`".to_owned(),
        )
    } else {
        misconfigured_operation(NOSTR_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            NOSTR_SERVE_OPERATION,
            "binary built without feature `channel-nostr`".to_owned(),
        )
    } else {
        unsupported_operation(
            NOSTR_SERVE_OPERATION,
            "nostr relay subscriber runtime is not implemented yet".to_owned(),
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

    fn deterministic_test_nostr_private_key_hex() -> String {
        let private_key_bytes = [0x11_u8; 32];
        hex::encode(private_key_bytes)
    }

    #[test]
    fn channel_catalog_includes_nostr_config_backed_surface() {
        let catalog = list_channel_catalog();
        let nostr = catalog
            .iter()
            .find(|entry| entry.id == "nostr")
            .expect("nostr catalog entry");

        assert_eq!(
            nostr.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(nostr.selection_order, 190);
        assert_eq!(nostr.transport, "nostr_relays");
        assert_eq!(
            nostr.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(nostr.operations[0].command, "nostr-send");
        assert_eq!(nostr.operations[1].command, "nostr-serve");
        assert_eq!(
            nostr.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "relay_urls", "private_key"]
        );
    }

    #[test]
    fn nostr_status_requires_relay_urls_and_private_key_for_send() {
        let mut config = LoongClawConfig::default();
        config.nostr.enabled = true;

        let snapshots = channel_status_snapshots(&config);
        let nostr = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "nostr")
            .expect("nostr snapshot");
        let send = nostr.operation("send").expect("nostr send operation");
        let serve = nostr.operation("serve").expect("nostr serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("relay_urls is empty")),
            "send issues should require configured relays"
        );
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("private_key is missing")),
            "send issues should require a signing key"
        );
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert!(nostr.api_base_url.is_none());
    }

    #[test]
    fn nostr_status_rejects_non_websocket_relay_urls() {
        let mut config = LoongClawConfig::default();
        config.nostr.enabled = true;
        config.nostr.relay_urls = vec!["https://relay.example.test".to_owned()];
        config.nostr.private_key = Some(loongclaw_contracts::SecretRef::Inline(
            deterministic_test_nostr_private_key_hex(),
        ));

        let snapshots = channel_status_snapshots(&config);
        let nostr = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "nostr")
            .expect("nostr snapshot");
        let send = nostr.operation("send").expect("nostr send operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("relay_urls[0] must use ws or wss")),
            "send issues should reject non-websocket relay urls"
        );
    }

    #[test]
    fn nostr_status_reports_invalid_private_key_without_missing_duplicate() {
        let mut config = LoongClawConfig::default();
        config.nostr.enabled = true;
        config.nostr.relay_urls = vec!["wss://relay.example.test".to_owned()];
        config.nostr.private_key = Some(loongclaw_contracts::SecretRef::Inline("00".repeat(32)));

        let snapshots = channel_status_snapshots(&config);
        let nostr = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "nostr")
            .expect("nostr snapshot");
        let send = nostr.operation("send").expect("nostr send operation");
        let invalid_issue_count = send
            .issues
            .iter()
            .filter(|issue| issue.contains("private_key is invalid"))
            .count();
        let has_missing_issue = send
            .issues
            .iter()
            .any(|issue| issue.contains("private_key is missing"));

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert_eq!(invalid_issue_count, 1);
        assert!(!has_missing_issue);
    }
}
