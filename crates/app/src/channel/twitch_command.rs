use std::path::PathBuf;

use super::{
    ChannelCommandContext, ChannelOutboundTargetKind, ChannelSendCommandSpec, LoongClawConfig,
    http, run_channel_send_command, twitch,
};
use crate::CliResult;
use crate::config::{self, ResolvedTwitchChannelConfig};

fn load_twitch_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTwitchChannelConfig>> {
    let (resolved_path, config) = config::load(config_path)?;
    build_twitch_command_context(resolved_path, config, account_id)
}

fn build_twitch_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTwitchChannelConfig>> {
    let resolved = config.twitch.resolve_account(account_id)?;
    let route = config
        .twitch
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());

    if !resolved.enabled {
        return Err(format!(
            "twitch account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }

    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_twitch_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-twitch") {
        return Err("twitch channel is disabled (enable feature `channel-twitch`)".to_owned());
    }

    #[cfg(not(feature = "channel-twitch"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("twitch channel is disabled (enable feature `channel-twitch`)".to_owned());
    }

    #[cfg(feature = "channel-twitch")]
    {
        let context = load_twitch_command_context(config_path, account_id)?;
        let outbound_http_policy = http::outbound_http_policy_from_config(&context.config);
        let target = target.to_owned();
        let text = text.to_owned();

        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "twitch",
            },
            |context| {
                Box::pin(async move {
                    twitch::run_twitch_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        outbound_http_policy,
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "twitch message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}
