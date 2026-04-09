use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::Notify;

use super::super::types::ChannelPlatform;
use super::state;
use super::state::ChannelOperationRuntimeTracker;
use crate::CliResult;

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
#[derive(Debug, Clone, Copy)]
pub struct ChannelServeRuntimeSpec<'a> {
    pub platform: ChannelPlatform,
    pub operation_id: &'static str,
    pub account_id: &'a str,
    pub account_label: &'a str,
}

#[derive(Debug, Clone)]
pub struct ChannelServeStopHandle {
    requested: Arc<AtomicBool>,
    stop: Arc<Notify>,
}

impl ChannelServeStopHandle {
    pub fn new() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(Notify::new()),
        }
    }

    pub fn request_stop(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.stop.notify_waiters();
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    #[cfg(any(
        feature = "channel-plugin-bridge",
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-line",
        feature = "channel-matrix",
        feature = "channel-wecom",
        feature = "channel-whatsapp",
        feature = "channel-webhook"
    ))]
    pub async fn wait(&self) {
        if self.is_requested() {
            return;
        }
        let notified = self.stop.notified();
        tokio::pin!(notified);
        if self.is_requested() {
            return;
        }
        notified.await;
    }
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub fn channel_runtime_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub fn ensure_channel_operation_runtime_slot_available_in_dir(
    runtime_dir: &std::path::Path,
    spec: ChannelServeRuntimeSpec<'_>,
) -> CliResult<()> {
    let now = channel_runtime_now_ms();
    state::prune_inactive_channel_operation_runtime_files_for_account_from_dir(
        runtime_dir,
        spec.platform,
        spec.operation_id,
        Some(spec.account_id),
        now,
    )?;
    let Some(runtime) = state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir,
        spec.platform,
        spec.operation_id,
        spec.account_id,
        now,
    ) else {
        return Ok(());
    };
    if runtime.running_instances == 0 {
        return Ok(());
    }

    let pid = runtime
        .pid
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    Err(format!(
        "{} account `{}` already has an active {} runtime (pid={}, running_instances={}); stop the existing instance or wait for it to become stale before restarting",
        spec.platform.as_str(),
        spec.account_id,
        spec.operation_id,
        pid,
        runtime.running_instances
    ))
}

/// Guard a channel serve loop with runtime-tracker lifecycle management.
///
/// The helper prunes stale runtime state, rejects duplicate active serve loops
/// for the same platform/operation/account triple, starts a tracker before
/// invoking `run`, and always attempts shutdown bookkeeping afterward.
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub async fn with_channel_serve_runtime<T, F, Fut>(
    spec: ChannelServeRuntimeSpec<'_>,
    run: F,
) -> CliResult<T>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>) -> Fut,
    Fut: Future<Output = CliResult<T>>,
{
    ensure_channel_operation_runtime_slot_available_in_dir(
        state::default_channel_runtime_state_dir().as_path(),
        spec,
    )?;
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
        )
        .await?,
    );
    let result = run(runtime.clone()).await;
    let shutdown_result = runtime.shutdown().await;
    merge_runtime_result(result, shutdown_result)
}

/// Variant of `with_channel_serve_runtime` that forwards a cooperative stop
/// handle into the serve loop.
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub async fn with_channel_serve_runtime_with_stop<F, Fut>(
    spec: ChannelServeRuntimeSpec<'_>,
    stop: ChannelServeStopHandle,
    run: F,
) -> CliResult<()>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>, ChannelServeStopHandle) -> Fut,
    Fut: Future<Output = CliResult<()>>,
{
    with_channel_serve_runtime(spec, move |runtime| run(runtime, stop)).await
}

#[cfg(test)]
pub(in crate::channel) async fn with_channel_serve_runtime_with_stop_in_dir<F, Fut>(
    runtime_dir: &std::path::Path,
    process_id: u32,
    spec: ChannelServeRuntimeSpec<'_>,
    stop: ChannelServeStopHandle,
    run: F,
) -> CliResult<()>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>, ChannelServeStopHandle) -> Fut,
    Fut: Future<Output = CliResult<()>>,
{
    with_channel_serve_runtime_in_dir(runtime_dir, process_id, spec, move |runtime| {
        run(runtime, stop)
    })
    .await
}

#[cfg(test)]
pub(in crate::channel) async fn with_channel_serve_runtime_in_dir<T, F, Fut>(
    runtime_dir: &std::path::Path,
    process_id: u32,
    spec: ChannelServeRuntimeSpec<'_>,
    run: F,
) -> CliResult<T>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>) -> Fut,
    Fut: Future<Output = CliResult<T>>,
{
    ensure_channel_operation_runtime_slot_available_in_dir(runtime_dir, spec)?;
    let runtime = Arc::new(
        state::start_channel_operation_runtime_tracker_for_test(
            runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            process_id,
        )
        .await?,
    );
    let result = run(runtime.clone()).await;
    let shutdown_result = runtime.shutdown().await;
    merge_runtime_result(result, shutdown_result)
}

fn merge_runtime_result<T>(result: CliResult<T>, shutdown_result: CliResult<()>) -> CliResult<T> {
    match (result, shutdown_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(shutdown_error)) => Err(shutdown_error),
        (Err(run_error), Ok(())) => Err(run_error),
        (Err(run_error), Err(shutdown_error)) => Err(format!(
            "{run_error}; additionally failed to shut down channel runtime cleanly: {shutdown_error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::merge_runtime_result;

    #[test]
    fn merge_runtime_result_returns_value_when_both_steps_succeed() {
        let result = merge_runtime_result::<u8>(Ok(7), Ok(()));

        assert_eq!(result, Ok(7));
    }

    #[test]
    fn merge_runtime_result_returns_shutdown_error_after_successful_run() {
        let result = merge_runtime_result::<u8>(Ok(7), Err("shutdown failed".to_owned()));

        assert_eq!(result, Err("shutdown failed".to_owned()));
    }

    #[test]
    fn merge_runtime_result_returns_run_error_when_shutdown_succeeds() {
        let result = merge_runtime_result::<u8>(Err("run failed".to_owned()), Ok(()));

        assert_eq!(result, Err("run failed".to_owned()));
    }

    #[test]
    fn merge_runtime_result_preserves_both_errors() {
        let result = merge_runtime_result::<u8>(
            Err("run failed".to_owned()),
            Err("shutdown failed".to_owned()),
        );
        let error = result.expect_err("both failures should produce an error");

        assert!(error.contains("run failed"));
        assert!(error.contains("shutdown failed"));
    }
}
