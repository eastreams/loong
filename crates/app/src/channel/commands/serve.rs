use std::sync::Arc;

use crate::CliResult;
use crate::KernelContext;
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config};

use super::super::runtime::serve::{
    ChannelServeCommandSpec, ChannelServeRuntimeSpec, ChannelServeStopHandle,
    with_channel_serve_runtime_with_stop,
};
use super::super::runtime::state::ChannelOperationRuntimeTracker;
use super::super::types::ChannelCommandFuture;
use super::context::{ChannelCommandContext, ChannelResolvedRuntimeAccount};

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
        KernelContext,
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
    let kernel_ctx = bootstrap_kernel_context_with_config(
        spec.family.runtime.serve_bootstrap_agent_id,
        DEFAULT_TOKEN_TTL_S,
        &context.config,
    )?;
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
            run(&context, kernel_ctx, runtime, stop).await
        },
    )
    .await
}
