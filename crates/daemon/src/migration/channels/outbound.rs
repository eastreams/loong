use loong_app as mvp;

use super::{
    ChannelCheckLevel, ChannelDoctorCheck, ChannelPreflightCheck, ChannelPreview,
    build_channel_preview, surface_is_outbound_only,
};
use crate::migration::ImportSurfaceLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutboundAccountSummary {
    label: String,
    ready_for_send: bool,
    send_detail: String,
    reserved_runtime_fields: Vec<String>,
}

pub(super) fn collect_previews(
    config: &mvp::config::LoongConfig,
    source: &str,
) -> Vec<ChannelPreview> {
    configured_outbound_surfaces(config)
        .into_iter()
        .filter_map(|surface| build_preview(&surface, source))
        .collect()
}

pub(super) fn collect_preflight_checks(
    config: &mvp::config::LoongConfig,
) -> Vec<ChannelPreflightCheck> {
    configured_outbound_surfaces(config)
        .into_iter()
        .filter_map(|surface| build_preflight_check(&surface))
        .collect()
}

pub(super) fn collect_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<ChannelDoctorCheck> {
    configured_outbound_surfaces(config)
        .into_iter()
        .map(|surface| ChannelDoctorCheck {
            name: channel_surface_name(surface.catalog.id),
            level: if surface_passes_preflight(&surface) {
                ChannelCheckLevel::Pass
            } else {
                ChannelCheckLevel::Fail
            },
            detail: surface_detail(&surface),
        })
        .collect()
}

pub(super) fn enabled_channels_have_blockers(config: &mvp::config::LoongConfig) -> bool {
    let checks = collect_preflight_checks(config);

    checks
        .into_iter()
        .any(|check| check.level != ChannelCheckLevel::Pass)
}

fn configured_outbound_surfaces(
    config: &mvp::config::LoongConfig,
) -> Vec<mvp::channel::ChannelSurface> {
    let inventory = mvp::channel::channel_inventory(config);

    inventory
        .channel_surfaces
        .into_iter()
        .filter(surface_is_outbound_only)
        .filter(surface_is_materially_configured)
        .collect()
}

fn surface_is_materially_configured(surface: &mvp::channel::ChannelSurface) -> bool {
    surface
        .configured_accounts
        .iter()
        .any(snapshot_is_materially_configured)
}

fn snapshot_is_materially_configured(snapshot: &mvp::channel::ChannelStatusSnapshot) -> bool {
    if snapshot.enabled {
        return true;
    }

    snapshot
        .operation(mvp::channel::CHANNEL_OPERATION_SEND_ID)
        .is_some_and(|operation| operation.health != mvp::channel::ChannelOperationHealth::Disabled)
}

fn build_preview(surface: &mvp::channel::ChannelSurface, source: &str) -> Option<ChannelPreview> {
    let channel_id = surface.catalog.id;
    let channel_label = channel_label(channel_id);
    let surface_name = channel_surface_name(channel_id);
    let level = preview_level(surface);
    let detail = surface_detail(surface);

    Some(build_channel_preview(
        channel_id,
        channel_label,
        surface_name,
        source.to_owned(),
        level,
        detail,
    ))
}

fn build_preflight_check(surface: &mvp::channel::ChannelSurface) -> Option<ChannelPreflightCheck> {
    Some(ChannelPreflightCheck {
        name: channel_surface_name(surface.catalog.id),
        level: preflight_level(surface),
        detail: surface_detail(surface),
    })
}

fn preview_level(surface: &mvp::channel::ChannelSurface) -> ImportSurfaceLevel {
    if surface_passes_preflight(surface) {
        return ImportSurfaceLevel::Ready;
    }

    ImportSurfaceLevel::Review
}

fn preflight_level(surface: &mvp::channel::ChannelSurface) -> ChannelCheckLevel {
    if surface_passes_preflight(surface) {
        return ChannelCheckLevel::Pass;
    }

    ChannelCheckLevel::Warn
}

fn surface_passes_preflight(surface: &mvp::channel::ChannelSurface) -> bool {
    let enabled_accounts = enabled_accounts(surface);
    if enabled_accounts.is_empty() {
        return false;
    }

    enabled_accounts
        .iter()
        .all(|snapshot| send_operation_is_ready(snapshot))
}

fn send_operation_is_ready(snapshot: &mvp::channel::ChannelStatusSnapshot) -> bool {
    snapshot
        .operation(mvp::channel::CHANNEL_OPERATION_SEND_ID)
        .is_some_and(|operation| operation.health == mvp::channel::ChannelOperationHealth::Ready)
}

fn enabled_accounts(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<&mvp::channel::ChannelStatusSnapshot> {
    surface
        .configured_accounts
        .iter()
        .filter(|snapshot| snapshot.enabled)
        .collect()
}

fn surface_detail(surface: &mvp::channel::ChannelSurface) -> String {
    let enabled_accounts = enabled_accounts(surface);
    let enabled_account_count = enabled_accounts.len();
    let ready_send_count = enabled_accounts
        .iter()
        .filter(|snapshot| send_operation_is_ready(snapshot))
        .count();
    let account_summaries = build_enabled_account_summaries(surface);
    let account_status_detail = account_status_detail(&account_summaries);
    let reserved_fields_detail = reserved_runtime_fields_detail(&account_summaries);

    if enabled_account_count == 0 {
        return "configured but disabled".to_owned();
    }

    if ready_send_count == enabled_account_count {
        let mut parts = Vec::new();
        let ready_detail =
            format!("enabled · direct send ready on {enabled_account_count} account(s)");

        parts.push(ready_detail);

        if let Some(account_status_detail) = account_status_detail {
            parts.push(account_status_detail);
        }

        if let Some(reserved_fields_detail) = reserved_fields_detail {
            parts.push(reserved_fields_detail);
        }

        parts.push("outbound-only surface".to_owned());

        return parts.join(" · ");
    }

    let mut parts = Vec::new();

    if ready_send_count > 0 {
        let ready_detail = format!(
            "enabled · direct send ready on {ready_send_count}/{enabled_account_count} account(s)"
        );

        parts.push(ready_detail);
    } else {
        parts.push("enabled".to_owned());
    }

    if let Some(account_status_detail) = account_status_detail {
        parts.push(account_status_detail);
    }

    if let Some(reserved_fields_detail) = reserved_fields_detail {
        parts.push(reserved_fields_detail);
    }

    parts.join(" · ")
}

fn build_enabled_account_summaries(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<OutboundAccountSummary> {
    let enabled_accounts = enabled_accounts(surface);
    let mut summaries = Vec::new();

    for snapshot in enabled_accounts {
        let summary = build_enabled_account_summary(snapshot);

        summaries.push(summary);
    }

    summaries
}

fn build_enabled_account_summary(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> OutboundAccountSummary {
    let label = configured_account_display_name(snapshot);
    let ready_for_send = send_operation_is_ready(snapshot);
    let send_detail = send_operation_detail(snapshot);
    let reserved_runtime_fields = collect_reserved_runtime_fields(snapshot);

    OutboundAccountSummary {
        label,
        ready_for_send,
        send_detail,
        reserved_runtime_fields,
    }
}

fn configured_account_display_name(snapshot: &mvp::channel::ChannelStatusSnapshot) -> String {
    let configured_account_label = snapshot.configured_account_label.trim();

    if configured_account_label.is_empty() {
        return snapshot.configured_account_id.clone();
    }

    let configured_account_id = snapshot.configured_account_id.as_str();

    if configured_account_label == configured_account_id {
        return configured_account_label.to_owned();
    }

    let display_name = format!("{configured_account_label} ({configured_account_id})");

    display_name
}

fn send_operation_detail(snapshot: &mvp::channel::ChannelStatusSnapshot) -> String {
    let send_operation = snapshot.operation(mvp::channel::CHANNEL_OPERATION_SEND_ID);

    let Some(send_operation) = send_operation else {
        return "direct send requires review".to_owned();
    };

    let send_detail = send_operation.detail.trim();

    if send_detail.is_empty() {
        return "direct send requires review".to_owned();
    }

    send_detail.to_owned()
}

fn collect_reserved_runtime_fields(snapshot: &mvp::channel::ChannelStatusSnapshot) -> Vec<String> {
    snapshot.reserved_runtime_fields.clone()
}

fn account_status_detail(account_summaries: &[OutboundAccountSummary]) -> Option<String> {
    if account_summaries.is_empty() {
        return None;
    }

    let mut status_segments = Vec::new();

    for summary in account_summaries {
        let segment = if summary.ready_for_send {
            format!("{} ready", summary.label)
        } else {
            format!("{} needs review ({})", summary.label, summary.send_detail)
        };

        status_segments.push(segment);
    }

    let prefix = if status_segments.len() == 1 {
        "account"
    } else {
        "accounts"
    };
    let joined_segments = status_segments.join("; ");
    let detail = format!("{prefix}: {joined_segments}");

    Some(detail)
}

fn reserved_runtime_fields_detail(account_summaries: &[OutboundAccountSummary]) -> Option<String> {
    let mut reserved_segments = Vec::new();

    for summary in account_summaries {
        if summary.reserved_runtime_fields.is_empty() {
            continue;
        }

        let joined_fields = summary.reserved_runtime_fields.join(", ");
        let segment = format!("{} [{}]", summary.label, joined_fields);

        reserved_segments.push(segment);
    }

    if reserved_segments.is_empty() {
        return None;
    }

    let joined_segments = reserved_segments.join("; ");
    let detail = format!("reserved future runtime fields: {joined_segments}");

    Some(detail)
}

fn channel_label(channel_id: &'static str) -> &'static str {
    let descriptor = mvp::config::channel_descriptor(channel_id);

    match descriptor {
        Some(descriptor) => descriptor.label,
        None => channel_id,
    }
}

fn channel_surface_name(channel_id: &'static str) -> &'static str {
    let descriptor = mvp::config::channel_descriptor(channel_id);

    match descriptor {
        Some(descriptor) => descriptor.surface_label,
        None => channel_label(channel_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_operation(
        health: mvp::channel::ChannelOperationHealth,
    ) -> mvp::channel::ChannelOperationStatus {
        mvp::channel::ChannelOperationStatus {
            id: mvp::channel::CHANNEL_OPERATION_SEND_ID,
            label: "direct send",
            command: "channels send discord",
            health,
            detail: "ready".to_owned(),
            issues: Vec::new(),
            runtime: None,
        }
    }

    fn outbound_snapshot(
        reserved_runtime_fields: Vec<String>,
    ) -> mvp::channel::ChannelStatusSnapshot {
        mvp::channel::ChannelStatusSnapshot {
            id: "discord",
            configured_account_id: "ops".to_owned(),
            configured_account_label: "ops".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Discord",
            aliases: vec!["discord-bot"],
            transport: "discord_http_api",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://discord.com/api/v10".to_owned()),
            notes: Vec::new(),
            reserved_runtime_fields,
            operations: vec![send_operation(mvp::channel::ChannelOperationHealth::Ready)],
        }
    }

    #[test]
    fn build_enabled_account_summary_reads_structured_reserved_runtime_fields() {
        let snapshot = outbound_snapshot(vec![
            "application_id".to_owned(),
            "allowed_guild_ids:2".to_owned(),
        ]);

        let summary = build_enabled_account_summary(&snapshot);

        assert_eq!(
            summary.reserved_runtime_fields,
            vec![
                "application_id".to_owned(),
                "allowed_guild_ids:2".to_owned()
            ]
        );
    }

    #[test]
    fn reserved_runtime_fields_detail_uses_structured_fields_without_note_protocol() {
        let snapshot = outbound_snapshot(vec![
            "application_id".to_owned(),
            "allowed_guild_ids:2".to_owned(),
        ]);
        let summary = build_enabled_account_summary(&snapshot);
        let detail = reserved_runtime_fields_detail(&[summary]);

        assert_eq!(
            detail,
            Some(
                "reserved future runtime fields: ops [application_id, allowed_guild_ids:2]"
                    .to_owned()
            )
        );
    }
}
