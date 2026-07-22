use std::sync::Arc;

use crate::CliResult;
use crate::runtime::bootstrap_runtime_with_config;

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
use super::super::registry::ChannelCommandFamilyDescriptor;
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
use super::super::runtime::serve::{
    ChannelServeRuntimeSpec, ChannelServeStopHandle, with_channel_serve_runtime_with_stop,
};
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
use super::super::runtime::state::ChannelOperationRuntimeTracker;
use super::super::types::ChannelCommandFuture;
use super::context::{ChannelCommandContext, ChannelResolvedRuntimeAccount};

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
#[derive(Debug, Clone, Copy)]
pub(in crate::channel) struct ChannelServeCommandSpec {
    pub(in crate::channel) family: ChannelCommandFamilyDescriptor,
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
/// Shared bootstrap for long-running channel `serve` commands.
///
/// Account resolution happens before this helper runs. It owns the common
/// runtime setup that most channel entrypoints share: channel-specific
/// validation, optional runtime-environment exports, kernel bootstrap, runtime
/// tracker registration, and stop-handle plumbing before handing control to the
/// channel-specific serve loop.
pub(in crate::channel) async fn run_channel_serve_command_with_stop<R, V, F>(
    context: ChannelCommandContext<R>,
    spec: ChannelServeCommandSpec,
    validate: V,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
    run: F,
) -> CliResult<()>
where
    R: ChannelResolvedRuntimeAccount,
    V: FnOnce(&R) -> CliResult<()>,
    F: for<'a> FnOnce(
        &'a ChannelCommandContext<R>,
        Arc<loong_runtime::runtime::Runtime<crate::RuntimeContextFactory>>,
        &'static str,
        Arc<ChannelOperationRuntimeTracker>,
        ChannelServeStopHandle,
    ) -> ChannelCommandFuture<'a>,
{
    validate(&context.resolved)?;
    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(
            &context.config,
            Some(context.resolved_path.as_path()),
        );
    }
    let execution_runtime = bootstrap_runtime_with_config(&context.config)?;
    let agent_id = spec.family.runtime.serve_bootstrap_agent_id;
    let runtime_account_id = context.resolved.runtime_account_id().to_owned();
    let runtime_account_label = context.resolved.runtime_account_label().to_owned();

    with_channel_serve_runtime_with_stop(
        ChannelServeRuntimeSpec {
            platform: spec.family.runtime.platform,
            operation_id: spec.family.serve().id,
            account_id: runtime_account_id.as_str(),
            account_label: runtime_account_label.as_str(),
        },
        stop,
        move |runtime, stop| async move {
            let channel_id = spec.family.channel_id();
            context.emit_route_notice(channel_id);
            run(&context, execution_runtime, agent_id, runtime, stop).await
        },
    )
    .await
}
