use std::path::Path;

use crate::channel::runtime::state;
use crate::config::ChannelDefaultAccountSelectionSource;

use super::{
    ChannelCatalogOperation, ChannelOperationHealth, ChannelOperationRuntime,
    ChannelOperationStatus, ChannelPlatform, ChannelRegistryDescriptor, ChannelStatusSnapshot,
};

enum CompiledInvalidBehavior {
    Misconfigured,
    Unsupported(&'static str),
}

fn unsupported_feature_detail(feature_name: &'static str) -> String {
    format!("binary built without feature `{feature_name}`")
}

fn invalid_selection_notes(
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: &str,
) -> Vec<String> {
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
    notes
}

fn build_invalid_operation(
    compiled: bool,
    operation: ChannelCatalogOperation,
    feature_name: &'static str,
    error: &str,
    behavior: CompiledInvalidBehavior,
) -> ChannelOperationStatus {
    if !compiled {
        return unsupported_operation(operation, unsupported_feature_detail(feature_name));
    }

    match behavior {
        CompiledInvalidBehavior::Misconfigured => {
            misconfigured_operation(operation, vec![error.to_owned()])
        }
        CompiledInvalidBehavior::Unsupported(detail) => {
            unsupported_operation(operation, detail.to_owned())
        }
    }
}

fn build_invalid_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: &str,
    operations: Vec<ChannelOperationStatus>,
) -> ChannelStatusSnapshot {
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
        notes: invalid_selection_notes(
            configured_account_id,
            is_default_account,
            default_account_source,
            error,
        ),
        reserved_runtime_fields: Vec::new(),
        operations,
    }
}

fn build_invalid_dual_misconfigured_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    send_operation: ChannelCatalogOperation,
    serve_operation: ChannelCatalogOperation,
    feature_name: &'static str,
) -> ChannelStatusSnapshot {
    build_invalid_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error.as_str(),
        vec![
            build_invalid_operation(
                compiled,
                send_operation,
                feature_name,
                error.as_str(),
                CompiledInvalidBehavior::Misconfigured,
            ),
            build_invalid_operation(
                compiled,
                serve_operation,
                feature_name,
                error.as_str(),
                CompiledInvalidBehavior::Misconfigured,
            ),
        ],
    )
}

fn build_invalid_send_misconfigured_serve_unsupported_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    send_operation: ChannelCatalogOperation,
    serve_operation: ChannelCatalogOperation,
    feature_name: &'static str,
    serve_detail: &'static str,
) -> ChannelStatusSnapshot {
    build_invalid_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error.as_str(),
        vec![
            build_invalid_operation(
                compiled,
                send_operation,
                feature_name,
                error.as_str(),
                CompiledInvalidBehavior::Misconfigured,
            ),
            build_invalid_operation(
                compiled,
                serve_operation,
                feature_name,
                error.as_str(),
                CompiledInvalidBehavior::Unsupported(serve_detail),
            ),
        ],
    )
}

fn build_invalid_runtime_backed_dual_misconfigured_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    send_operation: ChannelCatalogOperation,
    serve_operation: ChannelCatalogOperation,
    feature_name: &'static str,
    platform: ChannelPlatform,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let send_operation = attach_runtime(
        platform,
        send_operation,
        build_invalid_operation(
            compiled,
            send_operation,
            feature_name,
            error.as_str(),
            CompiledInvalidBehavior::Misconfigured,
        ),
        configured_account_id,
        configured_account_id,
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        platform,
        serve_operation,
        build_invalid_operation(
            compiled,
            serve_operation,
            feature_name,
            error.as_str(),
            CompiledInvalidBehavior::Misconfigured,
        ),
        configured_account_id,
        configured_account_id,
        runtime_dir,
        now_ms,
    );

    build_invalid_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error.as_str(),
        vec![send_operation, serve_operation],
    )
}

pub(super) fn build_invalid_telegram_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::TELEGRAM_SEND_OPERATION,
        super::TELEGRAM_SERVE_OPERATION,
        "channel-telegram",
    )
}

pub(super) fn build_invalid_feishu_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::FEISHU_SEND_OPERATION,
        super::FEISHU_SERVE_OPERATION,
        "channel-feishu",
    )
}

pub(super) fn build_invalid_matrix_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::MATRIX_SEND_OPERATION,
        super::MATRIX_SERVE_OPERATION,
        "channel-matrix",
    )
}

pub(super) fn build_invalid_wecom_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::WECOM_SEND_OPERATION,
        super::WECOM_SERVE_OPERATION,
        "channel-wecom",
    )
}

pub(super) fn build_invalid_discord_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::DISCORD_SEND_OPERATION,
        super::DISCORD_SERVE_OPERATION,
        "channel-discord",
        "discord serve runtime is not implemented yet",
    )
}

pub(super) fn build_invalid_slack_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::SLACK_SEND_OPERATION,
        super::SLACK_SERVE_OPERATION,
        "channel-slack",
        "slack serve runtime is not implemented yet",
    )
}

pub(super) fn build_invalid_line_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    build_invalid_runtime_backed_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::LINE_SEND_OPERATION,
        super::LINE_SERVE_OPERATION,
        "channel-line",
        ChannelPlatform::Line,
        runtime_dir,
        now_ms,
    )
}

pub(super) fn build_invalid_dingtalk_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::DINGTALK_SEND_OPERATION,
        super::DINGTALK_SERVE_OPERATION,
        "channel-dingtalk",
        "dingtalk custom robot surface is outbound-only",
    )
}

pub(super) fn build_invalid_whatsapp_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::WHATSAPP_SEND_OPERATION,
        super::WHATSAPP_SERVE_OPERATION,
        "channel-whatsapp",
    )
}

pub(super) fn build_invalid_email_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::EMAIL_SEND_OPERATION,
        super::EMAIL_SERVE_OPERATION,
        "channel-email",
        "email IMAP reply-loop serve runtime is not implemented yet",
    )
}

pub(super) fn build_invalid_webhook_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    build_invalid_runtime_backed_dual_misconfigured_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::WEBHOOK_SEND_OPERATION,
        super::WEBHOOK_SERVE_OPERATION,
        "channel-webhook",
        ChannelPlatform::Webhook,
        runtime_dir,
        now_ms,
    )
}

pub(super) fn build_invalid_google_chat_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::GOOGLE_CHAT_SEND_OPERATION,
        super::GOOGLE_CHAT_SERVE_OPERATION,
        "channel-google-chat",
        "google chat incoming webhook surface is outbound-only",
    )
}

pub(super) fn build_invalid_signal_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::SIGNAL_SEND_OPERATION,
        super::SIGNAL_SERVE_OPERATION,
        "channel-signal",
        "signal serve runtime is not implemented yet",
    )
}

pub(super) fn build_invalid_irc_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::IRC_SEND_OPERATION,
        super::IRC_SERVE_OPERATION,
        "channel-irc",
        "irc relay-loop serve is not implemented yet",
    )
}

pub(super) fn build_invalid_teams_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::TEAMS_SEND_OPERATION,
        super::TEAMS_SERVE_OPERATION,
        "channel-teams",
        "microsoft teams incoming webhook surface is outbound-only today",
    )
}

pub(super) fn build_invalid_imessage_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::IMESSAGE_SEND_OPERATION,
        super::IMESSAGE_SERVE_OPERATION,
        "channel-imessage",
        "imessage bridge sync runtime is not implemented yet",
    )
}

pub(super) fn build_invalid_mattermost_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::MATTERMOST_SEND_OPERATION,
        super::MATTERMOST_SERVE_OPERATION,
        "channel-mattermost",
        "mattermost serve runtime is not implemented yet",
    )
}

pub(super) fn build_invalid_nextcloud_talk_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::NEXTCLOUD_TALK_SEND_OPERATION,
        super::NEXTCLOUD_TALK_SERVE_OPERATION,
        "channel-nextcloud-talk",
        "nextcloud talk bot callback serve is not implemented yet",
    )
}

pub(super) fn build_invalid_synology_chat_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    build_invalid_send_misconfigured_serve_unsupported_snapshot(
        descriptor,
        compiled,
        configured_account_id,
        is_default_account,
        default_account_source,
        error,
        super::SYNOLOGY_CHAT_SEND_OPERATION,
        super::SYNOLOGY_CHAT_SERVE_OPERATION,
        "channel-synology-chat",
        "synology chat outgoing webhook serve is not implemented yet",
    )
}

pub(super) fn ready_operation(operation: ChannelCatalogOperation) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Ready,
        detail: "ready".to_owned(),
        issues: Vec::new(),
        runtime: None,
    }
}

pub(super) fn disabled_operation(
    operation: ChannelCatalogOperation,
    detail: String,
) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Disabled,
        detail,
        issues: Vec::new(),
        runtime: None,
    }
}

pub(super) fn unsupported_operation(
    operation: ChannelCatalogOperation,
    detail: String,
) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Unsupported,
        detail: detail.clone(),
        issues: vec![detail],
        runtime: None,
    }
}

pub(super) fn misconfigured_operation(
    operation: ChannelCatalogOperation,
    issues: Vec<String>,
) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Misconfigured,
        detail: issues.join("; "),
        issues,
        runtime: None,
    }
}

pub(super) fn attach_runtime(
    platform: ChannelPlatform,
    operation: ChannelCatalogOperation,
    mut status: ChannelOperationStatus,
    account_id: &str,
    account_label: &str,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelOperationStatus {
    if operation.tracks_runtime {
        status.runtime = state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir,
            platform,
            operation.id,
            account_id,
            now_ms,
        )
        .map(|mut runtime| {
            if runtime.account_id.is_none() {
                runtime.account_id = Some(account_id.to_owned());
            }
            if runtime.account_label.is_none() {
                runtime.account_label = Some(account_label.to_owned());
            }
            runtime
        })
        .or(Some(ChannelOperationRuntime {
            running: false,
            stale: false,
            busy: false,
            active_runs: 0,
            last_run_activity_at: None,
            last_heartbeat_at: None,
            pid: None,
            account_id: Some(account_id.to_owned()),
            account_label: Some(account_label.to_owned()),
            instance_count: 0,
            running_instances: 0,
            stale_instances: 0,
        }));
    }
    status
}
