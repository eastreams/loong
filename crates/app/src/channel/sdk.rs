use crate::{
    CliResult,
    config::{ConfigValidationIssue, LoongClawConfig},
};

use super::registry::{
    ChannelRegistryDescriptor, ChannelRuntimeCommandDescriptor,
    FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR, FEISHU_CHANNEL_REGISTRY_DESCRIPTOR,
    MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR, MATRIX_CHANNEL_REGISTRY_DESCRIPTOR,
    TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, TELEGRAM_CHANNEL_REGISTRY_DESCRIPTOR,
    WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, WECOM_CHANNEL_REGISTRY_DESCRIPTOR,
};

#[cfg(feature = "channel-feishu")]
use super::registry::FEISHU_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-matrix")]
use super::registry::MATRIX_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-telegram")]
use super::registry::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-wecom")]
use super::registry::WECOM_RUNTIME_COMMAND_DESCRIPTOR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRuntimeKind {
    Interactive,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub surface_label: &'static str,
    pub runtime_kind: ChannelRuntimeKind,
    pub serve_subcommand: Option<&'static str>,
}

type ChannelEnabledFn = fn(&LoongClawConfig) -> bool;
type ChannelValidationFn = fn(&LoongClawConfig) -> Vec<ConfigValidationIssue>;
type BackgroundSurfaceEnabledFn = fn(&LoongClawConfig, Option<&str>) -> CliResult<bool>;

#[derive(Clone, Copy)]
pub(crate) struct ChannelIntegrationDescriptor {
    pub descriptor: &'static ChannelDescriptor,
    pub registry_descriptor: Option<&'static ChannelRegistryDescriptor>,
    pub background_runtime: Option<ChannelRuntimeCommandDescriptor>,
    pub is_enabled: ChannelEnabledFn,
    pub collect_validation_issues: ChannelValidationFn,
    pub background_surface_is_enabled: Option<BackgroundSurfaceEnabledFn>,
}

const CLI_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "cli",
    label: "cli",
    surface_label: "cli channel",
    runtime_kind: ChannelRuntimeKind::Interactive,
    serve_subcommand: None,
};

const TELEGRAM_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "telegram",
    label: "telegram",
    surface_label: "telegram channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const FEISHU_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "feishu",
    label: "feishu",
    surface_label: "feishu channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const MATRIX_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "matrix",
    label: "matrix",
    surface_label: "matrix channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const WECOM_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "wecom",
    label: "wecom",
    surface_label: "wecom channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const CLI_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &CLI_CHANNEL_DESCRIPTOR,
    registry_descriptor: None,
    background_runtime: None,
    is_enabled: cli_channel_is_enabled,
    collect_validation_issues: collect_cli_channel_validation_issues,
    background_surface_is_enabled: None,
};

#[cfg(feature = "channel-telegram")]
const TELEGRAM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-telegram"))]
const TELEGRAM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const TELEGRAM_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &TELEGRAM_CHANNEL_DESCRIPTOR,
    registry_descriptor: Some(&TELEGRAM_CHANNEL_REGISTRY_DESCRIPTOR),
    background_runtime: TELEGRAM_BACKGROUND_RUNTIME,
    is_enabled: telegram_channel_is_enabled,
    collect_validation_issues: collect_telegram_channel_validation_issues,
    background_surface_is_enabled: Some(telegram_background_surface_is_enabled),
};

#[cfg(feature = "channel-feishu")]
const FEISHU_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(FEISHU_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-feishu"))]
const FEISHU_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const FEISHU_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &FEISHU_CHANNEL_DESCRIPTOR,
    registry_descriptor: Some(&FEISHU_CHANNEL_REGISTRY_DESCRIPTOR),
    background_runtime: FEISHU_BACKGROUND_RUNTIME,
    is_enabled: feishu_channel_is_enabled,
    collect_validation_issues: collect_feishu_channel_validation_issues,
    background_surface_is_enabled: Some(feishu_background_surface_is_enabled),
};

#[cfg(feature = "channel-matrix")]
const MATRIX_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(MATRIX_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-matrix"))]
const MATRIX_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const MATRIX_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &MATRIX_CHANNEL_DESCRIPTOR,
    registry_descriptor: Some(&MATRIX_CHANNEL_REGISTRY_DESCRIPTOR),
    background_runtime: MATRIX_BACKGROUND_RUNTIME,
    is_enabled: matrix_channel_is_enabled,
    collect_validation_issues: collect_matrix_channel_validation_issues,
    background_surface_is_enabled: Some(matrix_background_surface_is_enabled),
};

#[cfg(feature = "channel-wecom")]
const WECOM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(WECOM_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-wecom"))]
const WECOM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const WECOM_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &WECOM_CHANNEL_DESCRIPTOR,
    registry_descriptor: Some(&WECOM_CHANNEL_REGISTRY_DESCRIPTOR),
    background_runtime: WECOM_BACKGROUND_RUNTIME,
    is_enabled: wecom_channel_is_enabled,
    collect_validation_issues: collect_wecom_channel_validation_issues,
    background_surface_is_enabled: Some(wecom_background_surface_is_enabled),
};

const CHANNEL_INTEGRATIONS: &[ChannelIntegrationDescriptor] = &[
    CLI_CHANNEL_INTEGRATION,
    TELEGRAM_CHANNEL_INTEGRATION,
    FEISHU_CHANNEL_INTEGRATION,
    MATRIX_CHANNEL_INTEGRATION,
    WECOM_CHANNEL_INTEGRATION,
];

pub(crate) fn channel_descriptor(id: &str) -> Option<&'static ChannelDescriptor> {
    let integration = find_channel_integration(id)?;
    Some(integration.descriptor)
}

pub(crate) fn service_channel_descriptors() -> Vec<&'static ChannelDescriptor> {
    CHANNEL_INTEGRATIONS
        .iter()
        .map(|integration| integration.descriptor)
        .filter(|descriptor| descriptor.runtime_kind == ChannelRuntimeKind::Service)
        .collect()
}

pub(crate) fn enabled_channel_ids(
    config: &LoongClawConfig,
    runtime_kind: Option<ChannelRuntimeKind>,
) -> Vec<String> {
    CHANNEL_INTEGRATIONS
        .iter()
        .filter(|integration| {
            let enabled = (integration.is_enabled)(config);
            let matches_runtime_kind =
                runtime_kind.is_none_or(|kind| integration.descriptor.runtime_kind == kind);
            enabled && matches_runtime_kind
        })
        .map(|integration| integration.descriptor.id.to_owned())
        .collect()
}

pub(crate) fn collect_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    CHANNEL_INTEGRATIONS
        .iter()
        .flat_map(|integration| (integration.collect_validation_issues)(config))
        .collect()
}

pub(crate) fn runtime_backed_channel_registry_descriptors()
-> Vec<&'static ChannelRegistryDescriptor> {
    CHANNEL_INTEGRATIONS
        .iter()
        .filter_map(|integration| integration.registry_descriptor)
        .collect()
}

pub fn background_channel_runtime_descriptors() -> Vec<ChannelRuntimeCommandDescriptor> {
    CHANNEL_INTEGRATIONS
        .iter()
        .filter_map(|integration| integration.background_runtime)
        .collect()
}

pub fn is_background_channel_surface_enabled(
    channel_id: &str,
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    let integration = find_channel_integration(channel_id)
        .ok_or_else(|| format!("unsupported background channel `{channel_id}`"))?;
    let surface_is_enabled = integration
        .background_surface_is_enabled
        .ok_or_else(|| format!("unsupported background channel `{channel_id}`"))?;
    surface_is_enabled(config, account_id)
}

fn find_channel_integration(id: &str) -> Option<&'static ChannelIntegrationDescriptor> {
    let normalized_id = super::registry::normalize_channel_catalog_id(id)?;

    CHANNEL_INTEGRATIONS
        .iter()
        .find(|integration| integration.descriptor.id == normalized_id)
}

fn cli_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.cli.enabled
}

fn telegram_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.telegram.enabled
}

fn feishu_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.feishu.enabled
}

fn matrix_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.matrix.enabled
}

fn wecom_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.wecom.enabled
}

fn collect_cli_channel_validation_issues(_config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    Vec::new()
}

fn collect_telegram_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.telegram.validate()
}

fn collect_feishu_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.feishu.validate()
}

fn collect_matrix_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.matrix.validate()
}

fn collect_wecom_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.wecom.validate()
}

fn telegram_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.telegram.enabled {
        return Ok(false);
    }
    let resolved = config.telegram.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

#[cfg(feature = "feishu-integration")]
fn feishu_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.feishu.enabled {
        return Ok(false);
    }

    let resolved = crate::feishu::resolve_requested_feishu_account(
        &config.feishu,
        account_id,
        "rerun with `--channel-account <CHANNEL=ACCOUNT>` using one of those configured accounts",
    )?;
    Ok(resolved.enabled)
}

#[cfg(not(feature = "feishu-integration"))]
fn feishu_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.feishu.enabled {
        return Ok(false);
    }

    let resolved = config.feishu.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

fn matrix_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.matrix.enabled {
        return Ok(false);
    }
    let resolved = config.matrix.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

fn wecom_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.wecom.enabled {
        return Ok(false);
    }
    let resolved = config.wecom.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use loongclaw_contracts::SecretRef;

    use super::*;

    #[test]
    fn service_channel_descriptors_follow_integration_order() {
        let descriptors = service_channel_descriptors();
        let ids = descriptors
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["telegram", "feishu", "matrix", "wecom"]);
    }

    #[test]
    fn background_channel_runtime_descriptors_follow_integration_order() {
        let descriptors = background_channel_runtime_descriptors();
        let ids = descriptors
            .into_iter()
            .map(|descriptor| descriptor.channel_id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["telegram", "feishu", "matrix", "wecom"]);
    }

    #[test]
    fn unsupported_background_channels_are_rejected() {
        let config = LoongClawConfig::default();
        let error = is_background_channel_surface_enabled("cli", &config, None)
            .expect_err("cli should not be a background channel");

        assert_eq!(error, "unsupported background channel `cli`");
    }

    #[test]
    fn background_channel_surface_enablement_normalizes_aliases() {
        let config = LoongClawConfig::default();
        let enabled = is_background_channel_surface_enabled(" LARK ", &config, None)
            .expect("feishu alias should normalize through the channel registry");

        assert!(!enabled);
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_background_surface_enablement_accepts_runtime_account_aliases() {
        let shared_account_id = "feishu_shared".to_owned();
        let work_account = crate::config::FeishuAccountConfig {
            account_id: Some(shared_account_id.clone()),
            app_id: Some(SecretRef::Inline("cli_work".to_owned())),
            app_secret: Some(SecretRef::Inline("app-secret-work".to_owned())),
            ..crate::config::FeishuAccountConfig::default()
        };
        let accounts = BTreeMap::from([("work".to_owned(), work_account)]);
        let feishu = crate::config::FeishuChannelConfig {
            enabled: true,
            accounts,
            ..crate::config::FeishuChannelConfig::default()
        };
        let config = LoongClawConfig {
            feishu,
            ..LoongClawConfig::default()
        };

        let enabled = is_background_channel_surface_enabled(
            "feishu",
            &config,
            Some(shared_account_id.as_str()),
        )
        .expect("resolve unique feishu runtime-account alias");

        assert!(enabled);
    }
}
