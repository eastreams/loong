use std::collections::BTreeSet;

use loong_spec::CliResult;

use crate::{gateway, mvp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiChannelServeChannelAccount {
    pub channel_id: String,
    pub account_id: String,
}

impl std::str::FromStr for MultiChannelServeChannelAccount {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        parse_multi_channel_serve_channel_account(raw)
    }
}

fn parse_multi_channel_serve_channel_account(
    raw: &str,
) -> Result<MultiChannelServeChannelAccount, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("multi-channel channel-account entries cannot be empty".to_owned());
    }

    let (raw_channel_id, raw_account_id) = trimmed.split_once('=').ok_or_else(|| {
        format!("multi-channel channel-account `{trimmed}` must use CHANNEL=ACCOUNT syntax")
    })?;

    let channel_token = raw_channel_id.trim();
    if channel_token.is_empty() {
        return Err(format!(
            "multi-channel channel-account `{trimmed}` is missing a channel id"
        ));
    }

    let normalized_channel_id = mvp::channel::normalize_channel_catalog_id(channel_token)
        .ok_or_else(|| {
            let supported_channels = supported_multi_channel_serve_channel_ids().join(", ");
            format!(
                "unrecognized multi-channel service channel `{channel_token}` (available runtime-backed channels: {supported_channels})"
            )
        })?;
    let supported_channel_ids = supported_multi_channel_serve_channel_ids();
    let runtime_channel_id = normalized_channel_id;
    let runtime_is_supported = supported_channel_ids.contains(&runtime_channel_id);
    if !runtime_is_supported {
        let supported_channels = supported_channel_ids.join(", ");
        return Err(format!(
            "multi-channel service channel `{channel_token}` resolves to `{runtime_channel_id}` but is not supported in this build (expected one of: {supported_channels})"
        ));
    }

    let account_token = raw_account_id.trim();
    if account_token.is_empty() {
        return Err(format!(
            "multi-channel channel-account `{trimmed}` is missing an account id"
        ));
    }

    Ok(MultiChannelServeChannelAccount {
        channel_id: runtime_channel_id.to_owned(),
        account_id: account_token.to_owned(),
    })
}

fn supported_multi_channel_serve_channel_ids() -> Vec<&'static str> {
    let supported_channels = mvp::channel::background_channel_runtime_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.channel_id)
        .collect::<BTreeSet<_>>();
    supported_channels.into_iter().collect()
}

pub async fn run_multi_channel_serve_cli(
    config_path: Option<&str>,
    session: &str,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
) -> CliResult<()> {
    gateway::service::run_multi_channel_serve_gateway_compat_cli(
        config_path,
        session,
        channel_accounts,
    )
    .await
}

#[cfg(test)]
mod tests;
