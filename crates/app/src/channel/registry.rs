use std::{collections::BTreeSet, path::Path};

use serde::Serialize;

use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedFeishuChannelConfig,
    ResolvedTelegramChannelConfig,
};

use super::{ChannelOperationRuntime, ChannelPlatform, runtime_state};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChannelCatalogOperation {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub tracks_runtime: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub aliases: Vec<&'static str>,
    pub transport: &'static str,
    pub operations: Vec<ChannelCatalogOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOperationHealth {
    Ready,
    Disabled,
    Unsupported,
    Misconfigured,
}

impl ChannelOperationHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Disabled => "disabled",
            Self::Unsupported => "unsupported",
            Self::Misconfigured => "misconfigured",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelOperationStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub health: ChannelOperationHealth,
    pub detail: String,
    pub issues: Vec<String>,
    pub runtime: Option<ChannelOperationRuntime>,
}

impl ChannelOperationStatus {
    pub fn is_ready(&self) -> bool {
        self.health == ChannelOperationHealth::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelStatusSnapshot {
    pub id: &'static str,
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub is_default_account: bool,
    pub default_account_source: ChannelDefaultAccountSelectionSource,
    pub label: &'static str,
    pub aliases: Vec<&'static str>,
    pub transport: &'static str,
    pub compiled: bool,
    pub enabled: bool,
    pub api_base_url: Option<String>,
    pub notes: Vec<String>,
    pub operations: Vec<ChannelOperationStatus>,
}

impl ChannelStatusSnapshot {
    pub fn operation(&self, id: &str) -> Option<&ChannelOperationStatus> {
        self.operations.iter().find(|operation| operation.id == id)
    }
}

#[derive(Debug, Clone, Copy)]
struct ChannelRegistryDescriptor {
    platform: ChannelPlatform,
    label: &'static str,
    aliases: &'static [&'static str],
    transport: &'static str,
    operations: &'static [ChannelCatalogOperation],
}

const TELEGRAM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: "serve",
    label: "reply loop",
    command: "telegram-serve",
    tracks_runtime: true,
};

const TELEGRAM_OPERATIONS: &[ChannelCatalogOperation] = &[TELEGRAM_SERVE_OPERATION];

const FEISHU_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: "send",
    label: "direct send",
    command: "feishu-send",
    tracks_runtime: false,
};

const FEISHU_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: "serve",
    label: "webhook reply server",
    command: "feishu-serve",
    tracks_runtime: true,
};

const FEISHU_OPERATIONS: &[ChannelCatalogOperation] =
    &[FEISHU_SEND_OPERATION, FEISHU_SERVE_OPERATION];

const CHANNEL_REGISTRY: &[ChannelRegistryDescriptor] = &[
    ChannelRegistryDescriptor {
        platform: ChannelPlatform::Telegram,
        label: "Telegram",
        aliases: &[],
        transport: "telegram_bot_api_polling",
        operations: TELEGRAM_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        platform: ChannelPlatform::Feishu,
        label: "Feishu/Lark",
        aliases: &["lark"],
        transport: "feishu_openapi_webhook",
        operations: FEISHU_OPERATIONS,
    },
];

pub fn list_channel_catalog() -> Vec<ChannelCatalogEntry> {
    CHANNEL_REGISTRY
        .iter()
        .map(|descriptor| ChannelCatalogEntry {
            id: descriptor.platform.as_str(),
            label: descriptor.label,
            aliases: descriptor.aliases.to_vec(),
            transport: descriptor.transport,
            operations: descriptor.operations.to_vec(),
        })
        .collect()
}

pub fn catalog_only_channel_entries(
    snapshots: &[ChannelStatusSnapshot],
) -> Vec<ChannelCatalogEntry> {
    let catalog = list_channel_catalog();
    catalog_only_channel_entries_from(&catalog, snapshots)
}

fn catalog_only_channel_entries_from(
    catalog: &[ChannelCatalogEntry],
    snapshots: &[ChannelStatusSnapshot],
) -> Vec<ChannelCatalogEntry> {
    let snapshot_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.id)
        .collect::<BTreeSet<_>>();
    catalog
        .iter()
        .filter(|entry| !snapshot_ids.contains(entry.id))
        .cloned()
        .collect()
}

pub fn normalize_channel_platform(raw: &str) -> Option<ChannelPlatform> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    CHANNEL_REGISTRY.iter().find_map(|descriptor| {
        if descriptor.platform.as_str() == normalized {
            return Some(descriptor.platform);
        }
        descriptor
            .aliases
            .iter()
            .copied()
            .find(|alias| *alias == normalized)
            .map(|_| descriptor.platform)
    })
}

pub fn channel_status_snapshots(config: &LoongClawConfig) -> Vec<ChannelStatusSnapshot> {
    channel_status_snapshots_with_now(
        config,
        runtime_state::default_channel_runtime_state_dir().as_path(),
        now_ms(),
    )
}

fn channel_status_snapshots_with_now(
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let mut snapshots = Vec::new();
    for descriptor in CHANNEL_REGISTRY {
        match descriptor.platform {
            ChannelPlatform::Telegram => {
                snapshots.extend(build_telegram_snapshots(
                    descriptor,
                    config,
                    runtime_dir,
                    now_ms,
                ));
            }
            ChannelPlatform::Feishu => {
                snapshots.extend(build_feishu_snapshots(
                    descriptor,
                    config,
                    runtime_dir,
                    now_ms,
                ));
            }
        }
    }
    snapshots
}

fn build_telegram_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
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
    let mut issues = Vec::new();
    if resolved.bot_token().is_none() {
        issues.push("bot token is missing (telegram.bot_token or env)".to_owned());
    }
    if resolved.allowed_chat_ids.is_empty() {
        issues.push("allowed_chat_ids is empty".to_owned());
    }

    let operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SERVE_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TELEGRAM_SERVE_OPERATION,
            "disabled by telegram account configuration".to_owned(),
        )
    } else if !issues.is_empty() {
        misconfigured_operation(TELEGRAM_SERVE_OPERATION, issues)
    } else {
        ready_operation(TELEGRAM_SERVE_OPERATION)
    };
    let operation = attach_runtime(
        ChannelPlatform::Telegram,
        TELEGRAM_SERVE_OPERATION,
        operation,
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
        id: descriptor.platform.as_str(),
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
        operations: vec![operation],
    }
}

fn build_feishu_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
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

    let mut serve_issues = send_issues.clone();
    if !resolved
        .allowed_chat_ids
        .iter()
        .any(|value| !value.trim().is_empty())
    {
        serve_issues.push("allowed_chat_ids is empty".to_owned());
    }
    if resolved.verification_token().is_none() {
        serve_issues.push("verification_token is missing".to_owned());
    }
    if resolved.encrypt_key().is_none() {
        serve_issues.push("encrypt_key is missing".to_owned());
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
        format!("receive_id_type={}", resolved.receive_id_type),
        format!("webhook_bind={}", resolved.webhook_bind),
        format!("webhook_path={}", resolved.webhook_path),
    ];
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
        id: descriptor.platform.as_str(),
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
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_telegram_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SERVE_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else {
        misconfigured_operation(TELEGRAM_SERVE_OPERATION, vec![error.clone()])
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
        id: descriptor.platform.as_str(),
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
        operations: vec![operation],
    }
}

fn build_invalid_feishu_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            FEISHU_SEND_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else {
        misconfigured_operation(FEISHU_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            FEISHU_SERVE_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else {
        misconfigured_operation(FEISHU_SERVE_OPERATION, vec![error.clone()])
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
        id: descriptor.platform.as_str(),
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

fn ready_operation(operation: ChannelCatalogOperation) -> ChannelOperationStatus {
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

fn disabled_operation(
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

fn unsupported_operation(
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

fn misconfigured_operation(
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

fn attach_runtime(
    platform: ChannelPlatform,
    operation: ChannelCatalogOperation,
    mut status: ChannelOperationStatus,
    account_id: &str,
    account_label: &str,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelOperationStatus {
    if operation.tracks_runtime {
        status.runtime = runtime_state::load_channel_operation_runtime_for_account_from_dir(
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_channel_platform_maps_lark_alias_to_feishu() {
        assert_eq!(
            normalize_channel_platform("lark"),
            Some(ChannelPlatform::Feishu)
        );
        assert_eq!(
            normalize_channel_platform(" TELEGRAM "),
            Some(ChannelPlatform::Telegram)
        );
        assert_eq!(normalize_channel_platform("discord"), None);
    }

    #[test]
    fn channel_catalog_keeps_lark_alias_under_feishu_surface() {
        let catalog = list_channel_catalog();
        let feishu = catalog
            .iter()
            .find(|entry| entry.id == "feishu")
            .expect("feishu catalog entry");

        assert_eq!(feishu.aliases, vec!["lark"]);
        assert_eq!(feishu.operations.len(), 2);
        assert_eq!(feishu.operations[0].command, "feishu-send");
        assert_eq!(feishu.operations[1].command, "feishu-serve");
    }

    #[test]
    fn catalog_only_channel_entries_skip_platforms_that_already_have_status_snapshots() {
        let catalog = vec![
            ChannelCatalogEntry {
                id: "telegram",
                label: "Telegram",
                aliases: vec![],
                transport: "telegram_bot_api_polling",
                operations: vec![ChannelCatalogOperation {
                    id: "serve",
                    label: "reply loop",
                    command: "telegram-serve",
                    tracks_runtime: true,
                }],
            },
            ChannelCatalogEntry {
                id: "discord",
                label: "Discord",
                aliases: vec![],
                transport: "discord_gateway",
                operations: vec![ChannelCatalogOperation {
                    id: "send",
                    label: "direct send",
                    command: "discord-send",
                    tracks_runtime: false,
                }],
            },
        ];
        let snapshots = vec![ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            is_default_account: true,
            default_account_source: ChannelDefaultAccountSelectionSource::Fallback,
            label: "Telegram",
            aliases: vec![],
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: false,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: vec![],
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "telegram-serve",
                health: ChannelOperationHealth::Disabled,
                detail: "disabled".to_owned(),
                issues: vec![],
                runtime: None,
            }],
        }];

        let catalog_only = catalog_only_channel_entries_from(&catalog, &snapshots);

        assert_eq!(catalog_only.len(), 1);
        assert_eq!(catalog_only[0].id, "discord");
        assert_eq!(catalog_only[0].operations[0].command, "discord-send");
    }

    #[test]
    fn telegram_status_reports_ready_when_token_and_allowlist_are_configured() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some("123456:token".to_owned());
        config.telegram.allowed_chat_ids = vec![123];

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");
        let serve = telegram
            .operation("serve")
            .expect("telegram serve operation");

        assert_eq!(serve.health, ChannelOperationHealth::Ready);
        assert!(serve.is_ready());
        assert_eq!(
            telegram.api_base_url.as_deref(),
            Some("https://api.telegram.org")
        );
        assert!(!serve.runtime.as_ref().expect("telegram runtime").running);
    }

    #[test]
    fn feishu_status_splits_direct_send_and_webhook_readiness() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some("app-id".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let send = feishu.operation("send").expect("feishu send operation");
        let serve = feishu.operation("serve").expect("feishu serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("allowed_chat_ids")),
            "serve issues should mention allowlist"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("verification_token")),
            "serve issues should mention verification token"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("encrypt_key")),
            "serve issues should mention encrypt key"
        );
        assert!(send.runtime.is_none());
        assert_eq!(
            serve.runtime.as_ref().expect("serve runtime").active_runs,
            0
        );
    }

    #[test]
    fn channel_status_snapshots_merge_runtime_activity_for_serve_operations() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some("app-id".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token".to_owned());
        config.feishu.encrypt_key = Some("encrypt".to_owned());

        let runtime_dir = temp_runtime_dir("registry-runtime");
        let now = now_ms();
        runtime_state::write_runtime_state_for_test(
            runtime_dir.as_path(),
            ChannelPlatform::Feishu,
            "serve",
            true,
            true,
            2,
            Some(now.saturating_sub(1_000)),
            Some(now.saturating_sub(500)),
            Some(4242),
        )
        .expect("write runtime state");

        let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let serve = feishu.operation("serve").expect("feishu serve operation");
        let runtime = serve.runtime.as_ref().expect("runtime info");

        assert!(runtime.running);
        assert!(!runtime.stale);
        assert!(runtime.busy);
        assert_eq!(runtime.active_runs, 2);
        assert_eq!(runtime.pid, Some(4242));
    }

    #[test]
    fn channel_status_snapshots_report_resolved_account_identity_in_notes() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some("123456:token".to_owned());
        config.telegram.allowed_chat_ids = vec![123];

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");

        assert!(
            telegram
                .notes
                .iter()
                .any(|note| note.contains("account_id=bot_123456")),
            "telegram notes should expose the resolved account id"
        );
    }

    #[test]
    fn channel_status_snapshots_report_telegram_acp_bootstrap_mcp_servers_in_notes() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some("123456:token".to_owned());
        config.telegram.allowed_chat_ids = vec![123];
        config.telegram.acp.bootstrap_mcp_servers = vec!["filesystem".to_owned()];
        config.telegram.acp.working_directory = Some(" /workspace/telegram ".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");

        assert!(
            telegram
                .notes
                .iter()
                .any(|note| note == "acp_bootstrap_mcp_servers=filesystem"),
            "telegram notes should expose configured ACP bootstrap MCP servers"
        );
        assert!(
            telegram
                .notes
                .iter()
                .any(|note| note == "acp_working_directory=/workspace/telegram"),
            "telegram notes should expose configured ACP working directory"
        );
    }

    #[test]
    fn channel_status_snapshots_report_feishu_acp_bootstrap_mcp_servers_in_notes() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token".to_owned());
        config.feishu.encrypt_key = Some("encrypt".to_owned());
        config.feishu.acp.bootstrap_mcp_servers = vec!["search".to_owned()];
        config.feishu.acp.working_directory = Some("/workspace/feishu".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");

        assert!(
            feishu
                .notes
                .iter()
                .any(|note| note == "acp_bootstrap_mcp_servers=search"),
            "feishu notes should expose configured ACP bootstrap MCP servers"
        );
        assert!(
            feishu
                .notes
                .iter()
                .any(|note| note == "acp_working_directory=/workspace/feishu"),
            "feishu notes should expose configured ACP working directory"
        );
    }

    #[test]
    fn channel_status_snapshots_attach_account_identity_to_runtime_view() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token".to_owned());
        config.feishu.encrypt_key = Some("encrypt".to_owned());

        let runtime_dir = temp_runtime_dir("registry-account-runtime");
        let now = now_ms();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Feishu,
            "serve",
            "feishu_cli_a1b2c3",
            4242,
            true,
            true,
            2,
            Some(now.saturating_sub(1_000)),
            Some(now.saturating_sub(500)),
            Some(4242),
        )
        .expect("write runtime state");

        let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let serve = feishu.operation("serve").expect("feishu serve operation");
        let runtime = serve.runtime.as_ref().expect("runtime info");

        assert_eq!(runtime.account_id.as_deref(), Some("feishu_cli_a1b2c3"));
        assert_eq!(runtime.account_label.as_deref(), Some("feishu:cli_a1b2c3"));
    }

    #[test]
    fn channel_status_snapshots_preserve_runtime_instance_counts() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some("123456:token".to_owned());
        config.telegram.allowed_chat_ids = vec![123];

        let runtime_dir = temp_runtime_dir("registry-duplicate-runtime");
        let now = now_ms();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            1001,
            true,
            true,
            1,
            Some(now.saturating_sub(300)),
            Some(now.saturating_sub(100)),
            Some(1001),
        )
        .expect("write first runtime state");
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            1002,
            true,
            false,
            0,
            Some(now.saturating_sub(200)),
            Some(now.saturating_sub(50)),
            Some(1002),
        )
        .expect("write second runtime state");

        let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");
        let serve = telegram
            .operation("serve")
            .expect("telegram serve operation");
        let runtime = serve.runtime.as_ref().expect("runtime info");

        assert_eq!(runtime.instance_count, 2);
        assert_eq!(runtime.running_instances, 2);
        assert_eq!(runtime.stale_instances, 0);
    }

    #[test]
    fn multi_account_registry_emits_one_snapshot_per_configured_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "default_account": "Work Bot",
                "allowed_chat_ids": [1001],
                "accounts": {
                    "Work Bot": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:token-work",
                        "allowed_chat_ids": [2002]
                    },
                    "Personal": {
                        "bot_token": "654321:token-personal",
                        "allowed_chat_ids": [3003]
                    }
                }
            }
        }))
        .expect("deserialize multi-account config");

        let telegram = channel_status_snapshots(&config)
            .into_iter()
            .filter(|snapshot| snapshot.id == "telegram")
            .collect::<Vec<_>>();

        assert_eq!(telegram.len(), 2);
        assert_eq!(telegram[0].configured_account_id, "personal");
        assert_eq!(telegram[1].configured_account_id, "work-bot");
        assert!(
            telegram[1]
                .notes
                .iter()
                .any(|note| note == "configured_account_id=work-bot")
        );
        assert!(
            telegram[1]
                .notes
                .iter()
                .any(|note| note == "account_id=ops-bot")
        );
    }

    #[test]
    fn multi_account_registry_marks_default_configured_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "default_account": "Work Bot",
                "allowed_chat_ids": [1001],
                "accounts": {
                    "Work Bot": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:token-work",
                        "allowed_chat_ids": [2002]
                    },
                    "Personal": {
                        "bot_token": "654321:token-personal",
                        "allowed_chat_ids": [3003]
                    }
                }
            }
        }))
        .expect("deserialize multi-account config");

        let telegram = channel_status_snapshots(&config)
            .into_iter()
            .filter(|snapshot| snapshot.id == "telegram")
            .collect::<Vec<_>>();
        let encoded = serde_json::to_value(&telegram).expect("serialize telegram snapshots");

        assert!(
            telegram[1]
                .notes
                .iter()
                .any(|note| note == "default_account=true")
        );
        assert_eq!(
            encoded[0]
                .get("is_default_account")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            encoded[1]
                .get("is_default_account")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            encoded[1]
                .get("default_account_source")
                .and_then(serde_json::Value::as_str),
            Some("explicit_default")
        );
    }

    #[test]
    fn multi_account_registry_records_fallback_default_account_source() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "bot_token": "123456:token-work",
                        "allowed_chat_ids": [2002]
                    },
                    "Alerts": {
                        "bot_token": "654321:token-alerts",
                        "allowed_chat_ids": [3003]
                    }
                }
            }
        }))
        .expect("deserialize multi-account config");

        let telegram = channel_status_snapshots(&config)
            .into_iter()
            .filter(|snapshot| snapshot.id == "telegram")
            .collect::<Vec<_>>();

        assert!(telegram[0].is_default_account);
        assert_eq!(
            telegram[0].default_account_source,
            ChannelDefaultAccountSelectionSource::Fallback
        );
        assert!(
            telegram[0]
                .notes
                .iter()
                .any(|note| note == "default_account_source=fallback")
        );
    }

    fn temp_runtime_dir(suffix: &str) -> std::path::PathBuf {
        let unique = format!(
            "loongclaw-channel-registry-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}
