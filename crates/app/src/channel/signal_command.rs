use std::path::PathBuf;

use crate::CliResult;
use crate::config::LoongConfig;
use crate::config::{self, ResolvedSignalChannelConfig};

use super::commands::ChannelCommandContext;

pub(super) fn load_signal_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSignalChannelConfig>> {
    let (resolved_path, config) = config::load(config_path)?;
    build_signal_command_context(resolved_path, config, account_id)
}

fn build_signal_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSignalChannelConfig>> {
    let resolved = config.signal.resolve_account(account_id)?;
    let route = config
        .signal
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());

    if !resolved.enabled {
        return Err(format!(
            "signal account `{}` is disabled by configuration",
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
