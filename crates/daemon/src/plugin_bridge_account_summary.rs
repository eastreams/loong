use crate::mvp;

pub(crate) fn plugin_bridge_account_summary(
    surface: &mvp::channel::ChannelSurface,
) -> Option<String> {
    let has_plugin_bridge_contract = surface.catalog.plugin_bridge_contract.is_some();

    if !has_plugin_bridge_contract {
        return None;
    }

    let enabled_snapshots = plugin_bridge_enabled_snapshots(surface);
    let has_multiple_enabled_accounts = enabled_snapshots.len() > 1;

    if !has_multiple_enabled_accounts {
        return None;
    }

    let has_blocker = enabled_snapshots
        .iter()
        .any(|snapshot| plugin_bridge_snapshot_blocker_reason(snapshot).is_some());

    if !has_blocker {
        return None;
    }

    let entries = enabled_snapshots
        .into_iter()
        .map(plugin_bridge_account_summary_entry)
        .collect::<Vec<_>>();
    let summary = entries.join("; ");

    Some(summary)
}

pub(crate) fn plugin_bridge_snapshot_blocker_reason(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> Option<String> {
    for operation in &snapshot.operations {
        let is_disabled = operation.health == mvp::channel::ChannelOperationHealth::Disabled;

        if is_disabled {
            continue;
        }

        let is_misconfigured =
            operation.health == mvp::channel::ChannelOperationHealth::Misconfigured;

        if is_misconfigured {
            return Some(operation.detail.clone());
        }

        let is_unsupported = operation.health == mvp::channel::ChannelOperationHealth::Unsupported;

        if !is_unsupported {
            continue;
        }

        let supports_external_plugin =
            snapshot.compiled && snapshot_has_external_plugin_owner(snapshot);

        if supports_external_plugin {
            continue;
        }

        return Some(operation.detail.clone());
    }

    None
}

pub(crate) fn plugin_bridge_account_prefix(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> String {
    let configured_account_label = snapshot.configured_account_label.as_str();
    let mut prefix = format!("configured_account={configured_account_label}");

    if snapshot.is_default_account {
        prefix.push_str(" (default)");
    }

    prefix
}

pub(crate) fn plugin_bridge_account_summary_entry(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> String {
    let prefix = plugin_bridge_account_prefix(snapshot);
    let blocker = plugin_bridge_snapshot_blocker_reason(snapshot);

    if let Some(blocker) = blocker {
        return format!("{prefix}: {blocker}");
    }

    format!("{prefix}: ready")
}

fn plugin_bridge_enabled_snapshots(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<&mvp::channel::ChannelStatusSnapshot> {
    let mut snapshots = surface
        .configured_accounts
        .iter()
        .filter(|snapshot| snapshot.enabled)
        .collect::<Vec<_>>();

    snapshots.sort_by(|left, right| {
        let left_default_rank = if left.is_default_account { 0 } else { 1 };
        let right_default_rank = if right.is_default_account { 0 } else { 1 };
        let left_label = left.configured_account_label.as_str();
        let right_label = right.configured_account_label.as_str();

        left_default_rank
            .cmp(&right_default_rank)
            .then_with(|| left_label.cmp(right_label))
    });

    snapshots
}

fn snapshot_has_external_plugin_owner(snapshot: &mvp::channel::ChannelStatusSnapshot) -> bool {
    snapshot
        .notes
        .iter()
        .any(|note| note == "bridge_runtime_owner=external_plugin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_bridge_account_summary_renders_mixed_multi_account_surface() {
        let surface = mvp::channel::ChannelSurface {
            catalog: mvp::channel::ChannelCatalogEntry {
                id: "weixin",
                label: "weixin",
                selection_order: 0,
                selection_label: "weixin",
                blurb: "plugin bridge",
                implementation_status:
                    mvp::channel::ChannelCatalogImplementationStatus::PluginBacked,
                capabilities: Vec::new(),
                aliases: Vec::new(),
                transport: "plugin_bridge",
                onboarding: mvp::channel::ChannelOnboardingDescriptor {
                    strategy: mvp::channel::ChannelOnboardingStrategy::PluginBridge,
                    setup_hint: "plugin bridge",
                    status_command: "loong doctor",
                    repair_command: None,
                },
                plugin_bridge_contract: Some(mvp::channel::ChannelPluginBridgeContract {
                    manifest_channel_id: "weixin",
                    required_setup_surface: "channel",
                    runtime_owner: "external_plugin",
                    supported_operations: Vec::new(),
                    recommended_metadata_keys: Vec::new(),
                    stable_targets: Vec::new(),
                    account_scope_note: None,
                }),
                supported_target_kinds: Vec::new(),
                operations: Vec::new(),
            },
            configured_accounts: vec![
                channel_snapshot(
                    "ops",
                    true,
                    vec![
                        ready_operation("bridge send"),
                        ready_operation("bridge reply loop"),
                    ],
                    vec!["bridge_runtime_owner=external_plugin".to_owned()],
                ),
                channel_snapshot(
                    "backup",
                    false,
                    vec![misconfigured_operation(
                        "bridge send",
                        "bridge_url is missing",
                    )],
                    vec!["bridge_runtime_owner=external_plugin".to_owned()],
                ),
            ],
            default_configured_account_id: Some("ops".to_owned()),
            plugin_bridge_discovery: None,
        };

        let summary = plugin_bridge_account_summary(&surface);

        assert_eq!(
            summary.as_deref(),
            Some(
                "configured_account=ops (default): ready; configured_account=backup: bridge_url is missing"
            )
        );
    }

    #[test]
    fn plugin_bridge_snapshot_blocker_reason_ignores_external_plugin_unsupported_contract() {
        let snapshot = channel_snapshot(
            "ops",
            true,
            vec![unsupported_operation(
                "bridge runtime owned by external plugin",
            )],
            vec!["bridge_runtime_owner=external_plugin".to_owned()],
        );

        let blocker = plugin_bridge_snapshot_blocker_reason(&snapshot);

        assert_eq!(blocker, None);
    }

    #[test]
    fn plugin_bridge_snapshot_blocker_reason_keeps_unsupported_without_external_plugin_owner() {
        let snapshot = channel_snapshot(
            "ops",
            true,
            vec![unsupported_operation(
                "bridge runtime owned by external plugin",
            )],
            Vec::new(),
        );

        let blocker = plugin_bridge_snapshot_blocker_reason(&snapshot);

        assert_eq!(
            blocker.as_deref(),
            Some("bridge runtime owned by external plugin")
        );
    }

    fn channel_snapshot(
        configured_account_label: &str,
        is_default_account: bool,
        operations: Vec<mvp::channel::ChannelOperationStatus>,
        notes: Vec<String>,
    ) -> mvp::channel::ChannelStatusSnapshot {
        mvp::channel::ChannelStatusSnapshot {
            id: "weixin",
            configured_account_id: configured_account_label.to_owned(),
            configured_account_label: configured_account_label.to_owned(),
            is_default_account,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "weixin",
            aliases: Vec::new(),
            transport: "plugin_bridge",
            compiled: true,
            enabled: true,
            api_base_url: None,
            notes,
            reserved_runtime_fields: Vec::new(),
            operations,
        }
    }

    fn ready_operation(label: &'static str) -> mvp::channel::ChannelOperationStatus {
        mvp::channel::ChannelOperationStatus {
            id: "send",
            label,
            command: "channels send weixin",
            health: mvp::channel::ChannelOperationHealth::Ready,
            detail: "ready".to_owned(),
            issues: Vec::new(),
            runtime: None,
        }
    }

    fn misconfigured_operation(
        label: &'static str,
        detail: &str,
    ) -> mvp::channel::ChannelOperationStatus {
        mvp::channel::ChannelOperationStatus {
            id: "send",
            label,
            command: "channels send weixin",
            health: mvp::channel::ChannelOperationHealth::Misconfigured,
            detail: detail.to_owned(),
            issues: Vec::new(),
            runtime: None,
        }
    }

    fn unsupported_operation(detail: &str) -> mvp::channel::ChannelOperationStatus {
        mvp::channel::ChannelOperationStatus {
            id: "serve",
            label: "bridge reply loop",
            command: "channels serve weixin",
            health: mvp::channel::ChannelOperationHealth::Unsupported,
            detail: detail.to_owned(),
            issues: Vec::new(),
            runtime: None,
        }
    }
}
