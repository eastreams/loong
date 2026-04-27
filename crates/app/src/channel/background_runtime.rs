use std::{future::Future, path::PathBuf, pin::Pin};

use crate::{CliResult, config::LoongConfig};

#[cfg(feature = "channel-feishu")]
use super::FEISHU_RUNTIME_COMMAND_DESCRIPTOR;
#[cfg(feature = "channel-matrix")]
use super::MATRIX_RUNTIME_COMMAND_DESCRIPTOR;
#[cfg(feature = "channel-telegram")]
use super::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR;
#[cfg(feature = "channel-wecom")]
use super::WECOM_RUNTIME_COMMAND_DESCRIPTOR;
#[cfg(feature = "channel-whatsapp")]
use super::WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR;
#[cfg(feature = "channel-qqbot")]
use super::registry::QQBOT_RUNTIME_COMMAND_DESCRIPTOR;
use super::{ChannelRuntimeCommandDescriptor, ChannelServeStopHandle};

type BackgroundChannelRunFuture = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'static>>;
type BackgroundChannelRunFn = fn(BackgroundChannelRunRequest) -> BackgroundChannelRunFuture;

pub(crate) struct BackgroundChannelRunRequest {
    pub(crate) resolved_path: PathBuf,
    pub(crate) config: LoongConfig,
    pub(crate) account_id: Option<String>,
    pub(crate) stop: ChannelServeStopHandle,
    pub(crate) initialize_runtime_environment: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct BackgroundChannelRuntimeRunner {
    pub(crate) runtime: ChannelRuntimeCommandDescriptor,
    run: BackgroundChannelRunFn,
}

impl BackgroundChannelRuntimeRunner {
    const fn new(runtime: ChannelRuntimeCommandDescriptor, run: BackgroundChannelRunFn) -> Self {
        Self { runtime, run }
    }

    fn run(self, request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
        (self.run)(request)
    }
}

#[cfg(feature = "channel-telegram")]
fn run_telegram_channel(request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
    Box::pin(async move {
        super::dispatch::run_telegram_channel_with_stop(
            request.resolved_path,
            request.config,
            false,
            request.account_id.as_deref(),
            request.stop,
            request.initialize_runtime_environment,
        )
        .await
    })
}

#[cfg(feature = "channel-feishu")]
fn run_feishu_channel(request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
    Box::pin(async move {
        super::dispatch::run_feishu_channel_with_stop(
            request.resolved_path,
            request.config,
            request.account_id.as_deref(),
            None,
            None,
            request.stop,
            request.initialize_runtime_environment,
        )
        .await
    })
}

#[cfg(feature = "channel-matrix")]
fn run_matrix_channel(request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
    Box::pin(async move {
        super::dispatch::run_matrix_channel_with_stop(
            request.resolved_path,
            request.config,
            false,
            request.account_id.as_deref(),
            request.stop,
            request.initialize_runtime_environment,
        )
        .await
    })
}

#[cfg(feature = "channel-wecom")]
fn run_wecom_channel(request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
    Box::pin(async move {
        super::dispatch::run_wecom_channel_with_stop(
            request.resolved_path,
            request.config,
            request.account_id.as_deref(),
            request.stop,
            request.initialize_runtime_environment,
        )
        .await
    })
}

#[cfg(feature = "channel-whatsapp")]
fn run_whatsapp_channel(request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
    Box::pin(async move {
        super::dispatch::run_whatsapp_channel_with_stop(
            request.resolved_path,
            request.config,
            request.account_id.as_deref(),
            request.stop,
            request.initialize_runtime_environment,
        )
        .await
    })
}

#[cfg(feature = "channel-qqbot")]
fn run_qqbot_channel(request: BackgroundChannelRunRequest) -> BackgroundChannelRunFuture {
    Box::pin(async move {
        super::qqbot::run_qqbot_channel_with_stop(
            request.resolved_path,
            request.config,
            request.account_id.as_deref(),
            request.stop,
            request.initialize_runtime_environment,
        )
        .await
    })
}

pub(crate) fn background_channel_runtime_runners() -> &'static [BackgroundChannelRuntimeRunner] {
    BACKGROUND_CHANNEL_RUNTIME_RUNNERS
}

pub async fn run_background_channel_with_stop(
    channel_id: &str,
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let Some(runner) = background_channel_runtime_runners()
        .iter()
        .find(|runner| runner.runtime.channel_id == channel_id)
        .copied()
    else {
        return Err(format!("unsupported background channel `{channel_id}`"));
    };

    runner
        .run(BackgroundChannelRunRequest {
            resolved_path,
            config,
            account_id: account_id.map(str::to_owned),
            stop,
            initialize_runtime_environment,
        })
        .await
}

#[cfg(feature = "channel-telegram")]
const TELEGRAM_BACKGROUND_CHANNEL_RUNTIME_RUNNER: BackgroundChannelRuntimeRunner =
    BackgroundChannelRuntimeRunner::new(TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR, run_telegram_channel);

#[cfg(feature = "channel-feishu")]
const FEISHU_BACKGROUND_CHANNEL_RUNTIME_RUNNER: BackgroundChannelRuntimeRunner =
    BackgroundChannelRuntimeRunner::new(FEISHU_RUNTIME_COMMAND_DESCRIPTOR, run_feishu_channel);

#[cfg(feature = "channel-matrix")]
const MATRIX_BACKGROUND_CHANNEL_RUNTIME_RUNNER: BackgroundChannelRuntimeRunner =
    BackgroundChannelRuntimeRunner::new(MATRIX_RUNTIME_COMMAND_DESCRIPTOR, run_matrix_channel);

#[cfg(feature = "channel-wecom")]
const WECOM_BACKGROUND_CHANNEL_RUNTIME_RUNNER: BackgroundChannelRuntimeRunner =
    BackgroundChannelRuntimeRunner::new(WECOM_RUNTIME_COMMAND_DESCRIPTOR, run_wecom_channel);

#[cfg(feature = "channel-qqbot")]
const QQBOT_BACKGROUND_CHANNEL_RUNTIME_RUNNER: BackgroundChannelRuntimeRunner =
    BackgroundChannelRuntimeRunner::new(QQBOT_RUNTIME_COMMAND_DESCRIPTOR, run_qqbot_channel);

#[cfg(feature = "channel-whatsapp")]
const WHATSAPP_BACKGROUND_CHANNEL_RUNTIME_RUNNER: BackgroundChannelRuntimeRunner =
    BackgroundChannelRuntimeRunner::new(WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR, run_whatsapp_channel);

const BACKGROUND_CHANNEL_RUNTIME_RUNNERS: &[BackgroundChannelRuntimeRunner] = &[
    #[cfg(feature = "channel-telegram")]
    TELEGRAM_BACKGROUND_CHANNEL_RUNTIME_RUNNER,
    #[cfg(feature = "channel-feishu")]
    FEISHU_BACKGROUND_CHANNEL_RUNTIME_RUNNER,
    #[cfg(feature = "channel-matrix")]
    MATRIX_BACKGROUND_CHANNEL_RUNTIME_RUNNER,
    #[cfg(feature = "channel-wecom")]
    WECOM_BACKGROUND_CHANNEL_RUNTIME_RUNNER,
    #[cfg(feature = "channel-qqbot")]
    QQBOT_BACKGROUND_CHANNEL_RUNTIME_RUNNER,
    #[cfg(feature = "channel-whatsapp")]
    WHATSAPP_BACKGROUND_CHANNEL_RUNTIME_RUNNER,
];

#[cfg(test)]
mod tests {
    use super::super::{
        background_channel_runtime_descriptors, gateway_supervised_channel_descriptors,
    };
    use super::*;

    #[test]
    fn background_channel_runtime_runners_follow_background_runtime_descriptors() {
        let runner_ids = background_channel_runtime_runners()
            .iter()
            .map(|runner| runner.runtime.channel_id)
            .collect::<Vec<_>>();
        let descriptor_ids = background_channel_runtime_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.channel_id)
            .collect::<Vec<_>>();

        assert_eq!(runner_ids, descriptor_ids);
    }

    #[test]
    fn background_channel_runtime_runners_match_gateway_supervised_channel_ids() {
        let runner_ids = background_channel_runtime_runners()
            .iter()
            .map(|runner| runner.runtime.channel_id)
            .collect::<Vec<_>>();
        let gateway_supervised_ids = gateway_supervised_channel_descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();

        assert_eq!(runner_ids, gateway_supervised_ids);
    }
}
