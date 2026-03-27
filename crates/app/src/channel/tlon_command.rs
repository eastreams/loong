use crate::CliResult;

#[cfg(feature = "channel-tlon")]
use std::path::{Path, PathBuf};

#[cfg(feature = "channel-tlon")]
use super::tlon;
#[cfg(feature = "channel-tlon")]
use crate::config::{ChannelResolvedAccountRoute, LoongClawConfig, ResolvedTlonChannelConfig};

#[cfg(feature = "channel-tlon")]
struct TlonCommandContext {
    resolved_path: PathBuf,
    config: LoongClawConfig,
    resolved: ResolvedTlonChannelConfig,
    route: ChannelResolvedAccountRoute,
}

#[cfg(feature = "channel-tlon")]
fn load_tlon_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<TlonCommandContext> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_tlon_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-tlon")]
fn build_tlon_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<TlonCommandContext> {
    let resolved = config.tlon.resolve_account(account_id)?;
    let route = config
        .tlon
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "tlon account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }

    Ok(TlonCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(feature = "channel-tlon")]
fn initialize_tlon_runtime(config: &LoongClawConfig, resolved_path: &Path) {
    crate::runtime_env::initialize_runtime_environment(config, Some(resolved_path));
}

#[cfg(feature = "channel-tlon")]
fn render_tlon_route_notice(route: &ChannelResolvedAccountRoute) -> Option<String> {
    if !route.uses_implicit_fallback_default() {
        return None;
    }

    Some(format!(
        "tlon omitted --account and routed to configured account `{}` via fallback default selection; set tlon.default_account or pass --account to avoid routing surprises",
        route.configured_account_id
    ))
}

#[cfg(feature = "channel-tlon")]
fn emit_tlon_route_notice(route: &ChannelResolvedAccountRoute) {
    let Some(notice) = render_tlon_route_notice(route) else {
        return;
    };

    #[allow(clippy::print_stderr)]
    {
        eprintln!("warning: {notice}");
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_tlon_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: super::ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-tlon") {
        return Err("tlon channel is disabled (enable feature `channel-tlon`)".to_owned());
    }

    #[cfg(not(feature = "channel-tlon"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("tlon channel is disabled (enable feature `channel-tlon`)".to_owned());
    }

    #[cfg(feature = "channel-tlon")]
    {
        let context = load_tlon_command_context(config_path, account_id)?;
        let target_text = target.to_owned();
        let message_text = text.to_owned();
        initialize_tlon_runtime(&context.config, context.resolved_path.as_path());
        emit_tlon_route_notice(&context.route);
        tlon::run_tlon_send(
            &context.resolved,
            target_kind,
            target_text.as_str(),
            message_text.as_str(),
        )
        .await?;

        #[allow(clippy::print_stdout)]
        {
            println!(
                "tlon message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                context.resolved_path.display(),
                context.resolved.configured_account_id,
                context.resolved.account.label,
                context.route.selected_by_default(),
                context.route.default_account_source.as_str(),
                target_kind
            );
        }

        Ok(())
    }
}
