use std::path::Path;

use crate::config::{
    ChannelDefaultAccountSelectionSource, FeishuChannelServeMode, LoongConfig,
    ResolvedFeishuChannelConfig, ResolvedLineChannelConfig, ResolvedMatrixChannelConfig,
    ResolvedQqbotChannelConfig, ResolvedTelegramChannelConfig, ResolvedWecomChannelConfig,
    ResolvedWhatsappChannelConfig,
};

use super::status_support::{
    build_invalid_feishu_snapshot, build_invalid_line_snapshot, build_invalid_matrix_snapshot,
    build_invalid_telegram_snapshot, build_invalid_wecom_snapshot, build_invalid_whatsapp_snapshot,
};
use super::*;

pub(super) fn build_telegram_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-telegram");
    let default_selection = config.telegram.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .telegram
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .telegram
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_telegram_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_telegram_snapshot(
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

fn build_telegram_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedTelegramChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.bot_token().is_none() {
        send_issues.push("bot token is missing (telegram.bot_token or env)".to_owned());
    }

    let access_policy = ChannelInboundAccessPolicy::from_i64_lists(
        resolved.allowed_chat_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
    );
    let mut serve_issues = send_issues.clone();
    if !access_policy.has_conversation_restrictions() {
        serve_issues.push("allowed_chat_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SEND_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TELEGRAM_SEND_OPERATION,
            "disabled by telegram account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(TELEGRAM_SEND_OPERATION, send_issues)
    } else {
        ready_operation(TELEGRAM_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SERVE_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TELEGRAM_SERVE_OPERATION,
            "disabled by telegram account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(TELEGRAM_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(TELEGRAM_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Telegram,
        TELEGRAM_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Telegram,
        TELEGRAM_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("polling_timeout_s={}", resolved.polling_timeout_s),
    ];
    if !resolved.allowed_chat_ids.is_empty() {
        let allowed_chat_ids = resolved
            .allowed_chat_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        notes.push(format!("allowed_chat_ids={allowed_chat_ids}"));
    }
    if !resolved.allowed_sender_ids.is_empty() {
        let allowed_sender_ids = resolved
            .allowed_sender_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        notes.push(format!("allowed_sender_ids={allowed_sender_ids}"));
    }
    notes.push(format!("require_mention={}", resolved.require_mention));
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: Some(resolved.base_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_feishu_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-feishu");
    let default_selection = config.feishu.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .feishu
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .feishu
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_feishu_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_feishu_snapshot(
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

fn build_feishu_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedFeishuChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.app_id().is_none() {
        send_issues.push("app_id is missing".to_owned());
    }
    if resolved.app_secret().is_none() {
        send_issues.push("app_secret is missing".to_owned());
    }

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_chat_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        true,
    );
    let mut serve_issues = send_issues.clone();
    if !access_policy.has_conversation_restrictions() {
        serve_issues.push("allowed_chat_ids is empty".to_owned());
    }
    if resolved.mode == FeishuChannelServeMode::Webhook {
        if resolved.verification_token().is_none() {
            serve_issues.push("verification_token is missing".to_owned());
        }
        if resolved.encrypt_key().is_none() {
            serve_issues.push("encrypt_key is missing".to_owned());
        }
    }

    let send_operation = if !compiled {
        unsupported_operation(
            FEISHU_SEND_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            FEISHU_SEND_OPERATION,
            "disabled by feishu account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(FEISHU_SEND_OPERATION, send_issues)
    } else {
        ready_operation(FEISHU_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            FEISHU_SERVE_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            FEISHU_SERVE_OPERATION,
            "disabled by feishu account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(FEISHU_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(FEISHU_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Feishu,
        FEISHU_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Feishu,
        FEISHU_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("mode={}", resolved.mode.as_str()),
        format!("receive_id_type={}", resolved.receive_id_type),
    ];
    if let Some(allowed_chat_ids) = access_policy.string_conversations() {
        notes.push(format!("allowed_chat_ids={}", allowed_chat_ids.join(",")));
    }
    if let Some(allowed_sender_ids) = access_policy.string_senders() {
        notes.push(format!(
            "allowed_sender_ids={}",
            allowed_sender_ids.join(",")
        ));
    }
    if resolved.mode == FeishuChannelServeMode::Webhook {
        notes.push(format!("webhook_bind={}", resolved.webhook_bind));
        notes.push(format!("webhook_path={}", resolved.webhook_path));
    }
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: Some(resolved.resolved_base_url()),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_matrix_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-matrix");
    let default_selection = config.matrix.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .matrix
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .matrix
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_matrix_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_matrix_snapshot(
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

fn build_matrix_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedMatrixChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.access_token().is_none() {
        send_issues.push("access_token is missing".to_owned());
    }
    if resolved.resolved_base_url().is_none() {
        send_issues.push("base_url is missing".to_owned());
    }

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_room_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        false,
    );
    let mut serve_issues = send_issues.clone();
    if !access_policy.has_conversation_restrictions() {
        serve_issues.push("allowed_room_ids is empty".to_owned());
    }
    let has_user_id = resolved
        .user_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if resolved.ignore_self_messages && !has_user_id {
        serve_issues.push("user_id is missing while ignore_self_messages is enabled".to_owned());
    }
    if resolved.require_mention && !has_user_id {
        serve_issues.push("user_id is missing while require_mention is enabled".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            MATRIX_SEND_OPERATION,
            "binary built without feature `channel-matrix`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            MATRIX_SEND_OPERATION,
            "disabled by matrix account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(MATRIX_SEND_OPERATION, send_issues)
    } else {
        ready_operation(MATRIX_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            MATRIX_SERVE_OPERATION,
            "binary built without feature `channel-matrix`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            MATRIX_SERVE_OPERATION,
            "disabled by matrix account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(MATRIX_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(MATRIX_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Matrix,
        MATRIX_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Matrix,
        MATRIX_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("sync_timeout_s={}", resolved.sync_timeout_s),
        format!("ignore_self_messages={}", resolved.ignore_self_messages),
    ];
    if let Some(allowed_room_ids) = access_policy.string_conversations() {
        notes.push(format!("allowed_room_ids={}", allowed_room_ids.join(",")));
    }
    if let Some(allowed_sender_ids) = access_policy.string_senders() {
        notes.push(format!(
            "allowed_sender_ids={}",
            allowed_sender_ids.join(",")
        ));
    }
    notes.push(format!("require_mention={}", resolved.require_mention));
    if let Some(user_id) = resolved.user_id.as_deref() {
        notes.push(format!("user_id={user_id}"));
    }
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: resolved.resolved_base_url(),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_wecom_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-wecom");
    let default_selection = config.wecom.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .wecom
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .wecom
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_wecom_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_wecom_snapshot(
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

fn build_wecom_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWecomChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.bot_id().is_none() {
        send_issues.push("bot_id is missing".to_owned());
    }
    if resolved.secret().is_none() {
        send_issues.push("secret is missing".to_owned());
    }

    let websocket_url = resolved.resolved_websocket_url();
    validate_websocket_url(
        "wecom.websocket_url",
        websocket_url.as_str(),
        &mut send_issues,
    );

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_conversation_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        false,
    );
    let mut serve_issues = send_issues.clone();
    if !access_policy.has_conversation_restrictions() {
        serve_issues.push("allowed_conversation_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WECOM_SEND_OPERATION,
            "binary built without feature `channel-wecom`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WECOM_SEND_OPERATION,
            "disabled by wecom account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WECOM_SEND_OPERATION, send_issues)
    } else {
        ready_operation(WECOM_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WECOM_SERVE_OPERATION,
            "binary built without feature `channel-wecom`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WECOM_SERVE_OPERATION,
            "disabled by wecom account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WECOM_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(WECOM_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Wecom,
        WECOM_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Wecom,
        WECOM_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("websocket_url={websocket_url}"),
        format!("ping_interval_s={}", resolved.ping_interval_s),
        format!("reconnect_interval_s={}", resolved.reconnect_interval_s),
    ];
    if let Some(allowed_conversation_ids) = access_policy.string_conversations() {
        notes.push(format!(
            "allowed_conversation_ids={}",
            allowed_conversation_ids.join(",")
        ));
    }
    if let Some(allowed_sender_ids) = access_policy.string_senders() {
        notes.push(format!(
            "allowed_sender_ids={}",
            allowed_sender_ids.join(",")
        ));
    }
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: Some(websocket_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_line_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-line");
    let http_policy = super::super::http::outbound_http_policy_from_config(config);
    let default_selection = config.line.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .line
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .line
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_line_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_line_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                    runtime_dir,
                    now_ms,
                ),
            }
        })
        .collect()
}

fn build_line_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedLineChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::super::http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.channel_access_token().is_none() {
        send_issues.push("channel_access_token is missing".to_owned());
    }

    let resolved_api_base_url = resolved.resolved_api_base_url();
    let api_base_url = validate_http_base_url(
        "api_base_url",
        resolved_api_base_url.as_str(),
        http_policy,
        &mut send_issues,
    );

    let send_operation = if !compiled {
        unsupported_operation(
            LINE_SEND_OPERATION,
            "binary built without feature `channel-line`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            LINE_SEND_OPERATION,
            "disabled by line account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(LINE_SEND_OPERATION, send_issues.clone())
    } else {
        ready_operation(LINE_SEND_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Line,
        LINE_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut serve_issues = send_issues.clone();
    if resolved.channel_secret().is_none() {
        serve_issues.push("channel_secret is missing".to_owned());
    }

    let serve_operation = if !compiled {
        unsupported_operation(
            LINE_SERVE_OPERATION,
            "binary built without feature `channel-line`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            LINE_SERVE_OPERATION,
            "disabled by line account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(LINE_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(LINE_SERVE_OPERATION)
    };
    let serve_operation = attach_runtime(
        ChannelPlatform::Line,
        LINE_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
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
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: api_base_url.as_ref().and_then(|_| {
            super::super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())
        }),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_whatsapp_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-whatsapp");
    let http_policy = super::super::http::outbound_http_policy_from_config(config);
    let default_selection = config.whatsapp.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .whatsapp
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .whatsapp
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_whatsapp_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_whatsapp_snapshot(
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

fn build_whatsapp_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWhatsappChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::super::http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.access_token().is_none() {
        send_issues.push("access_token is missing".to_owned());
    }
    if resolved.phone_number_id().is_none() {
        send_issues.push("phone_number_id is missing".to_owned());
    }

    let resolved_api_base_url = resolved.resolved_api_base_url();
    let api_base_url = validate_http_base_url(
        "api_base_url",
        resolved_api_base_url.as_str(),
        http_policy,
        &mut send_issues,
    );

    let mut serve_issues = send_issues.clone();
    if resolved.verify_token().is_none() {
        serve_issues.push("verify_token is missing".to_owned());
    }
    if resolved.app_secret().is_none() {
        serve_issues.push("app_secret is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_SEND_OPERATION,
            "binary built without feature `channel-whatsapp`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WHATSAPP_SEND_OPERATION,
            "disabled by whatsapp account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WHATSAPP_SEND_OPERATION, send_issues)
    } else {
        ready_operation(WHATSAPP_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_SERVE_OPERATION,
            "binary built without feature `channel-whatsapp`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WHATSAPP_SERVE_OPERATION,
            "disabled by whatsapp account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WHATSAPP_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(WHATSAPP_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::WhatsApp,
        WHATSAPP_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::WhatsApp,
        WHATSAPP_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if let Some(phone_number_id) = resolved.phone_number_id() {
        notes.push(format!("phone_number_id={phone_number_id}"));
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
        api_base_url: api_base_url.as_ref().and_then(|_| {
            super::super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())
        }),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_qqbot_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-qqbot");
    let default_selection = config.qqbot.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .qqbot
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .qqbot
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_qqbot_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_qqbot_snapshot(
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

fn build_qqbot_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedQqbotChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.app_id().is_none() {
        send_issues.push("app_id is missing".to_owned());
    }
    if resolved.client_secret().is_none() {
        send_issues.push("client_secret is missing".to_owned());
    }

    let mut serve_issues = send_issues.clone();
    let has_allowed_peer_ids = resolved
        .allowed_peer_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowed_peer_ids {
        serve_issues.push("allowed_peer_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            QQBOT_SEND_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            QQBOT_SEND_OPERATION,
            "disabled by qqbot account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(QQBOT_SEND_OPERATION, send_issues)
    } else {
        ready_operation(QQBOT_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            QQBOT_SERVE_OPERATION,
            "disabled by qqbot account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(QQBOT_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(QQBOT_SERVE_OPERATION)
    };

    let send_operation = attach_runtime(
        ChannelPlatform::Qqbot,
        QQBOT_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Qqbot,
        QQBOT_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if !resolved.allowed_peer_ids.is_empty() {
        let allowed_peer_ids = resolved.allowed_peer_ids.join(",");
        notes.push(format!("allowed_peer_ids={allowed_peer_ids}"));
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
        api_base_url: Some("https://api.sgroup.qq.com".to_owned()),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_qqbot_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            QQBOT_SEND_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else {
        misconfigured_operation(QQBOT_SEND_OPERATION, vec![error.clone()])
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else {
        misconfigured_operation(QQBOT_SERVE_OPERATION, vec![error.clone()])
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
        enabled: true,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}
