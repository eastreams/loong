use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::sync::Notify;
use tokio::time::sleep;

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

#[cfg(test)]
const CHANNEL_RUNTIME_DUPLICATE_RECLAIM_POLL_MS: u64 = 25;
#[cfg(not(test))]
const CHANNEL_RUNTIME_DUPLICATE_RECLAIM_POLL_MS: u64 = 500;
#[cfg(test)]
const CHANNEL_RUNTIME_DUPLICATE_RECLAIM_COOLDOWN_MS: u64 = 50;
#[cfg(not(test))]
const CHANNEL_RUNTIME_DUPLICATE_RECLAIM_COOLDOWN_MS: u64 = 5_000;

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
/// invoking `run`, auto-reclaims duplicate owners conservatively, and always
/// attempts shutdown bookkeeping afterward.
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

async fn with_channel_serve_runtime_stop_tasks<F, Fut>(
    runtime_dir: &std::path::Path,
    spec: ChannelServeRuntimeSpec<'_>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
    run: F,
) -> CliResult<()>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>, ChannelServeStopHandle) -> Fut,
    Fut: Future<Output = CliResult<()>>,
{
    let duplicate_reclaim_platform = spec.platform;
    let duplicate_reclaim_operation_id = spec.operation_id;
    let duplicate_reclaim_account_id = spec.account_id.to_owned();
    let runtime_for_stop_request = runtime.clone();
    let stop_for_stop_request = stop.clone();
    let stop_request_task = tokio::spawn(async move {
        let wait_result = runtime_for_stop_request.wait_for_stop_request().await;
        if wait_result.is_ok() {
            stop_for_stop_request.request_stop();
        }
    });
    let runtime_for_duplicate_reclaim = runtime.clone();
    let stop_for_duplicate_reclaim = stop.clone();
    let duplicate_reclaim_runtime_dir = runtime_dir.to_path_buf();
    let duplicate_reclaim_task = tokio::spawn(async move {
        loop {
            if stop_for_duplicate_reclaim.is_requested() {
                break;
            }
            sleep(Duration::from_millis(
                CHANNEL_RUNTIME_DUPLICATE_RECLAIM_POLL_MS,
            ))
            .await;
            if stop_for_duplicate_reclaim.is_requested() {
                break;
            }
            let reclaim_result = maybe_reclaim_duplicate_runtime_owners_for_preferred_owner(
                duplicate_reclaim_runtime_dir.as_path(),
                duplicate_reclaim_platform,
                duplicate_reclaim_operation_id,
                duplicate_reclaim_account_id.as_str(),
                runtime_for_duplicate_reclaim.pid(),
            );
            match reclaim_result {
                Ok(Some(result)) => {
                    let record_result = runtime_for_duplicate_reclaim
                        .record_duplicate_reclaim(result.targeted_owner_pids.as_slice())
                        .await;
                    if let Err(error) = record_result {
                        tracing::warn!(
                            target: "loong.channel.runtime",
                            platform = duplicate_reclaim_platform.as_str(),
                            operation_id = duplicate_reclaim_operation_id,
                            account_id = duplicate_reclaim_account_id.as_str(),
                            "duplicate runtime auto-reclaim persisted cleanup request poorly: {error}"
                        );
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "loong.channel.runtime",
                        platform = duplicate_reclaim_platform.as_str(),
                        operation_id = duplicate_reclaim_operation_id,
                        account_id = duplicate_reclaim_account_id.as_str(),
                        "duplicate runtime auto-reclaim failed: {error}"
                    );
                }
            }
        }
    });
    let result = run(runtime, stop).await;
    stop_request_task.abort();
    duplicate_reclaim_task.abort();
    result
}

fn maybe_reclaim_duplicate_runtime_owners_for_preferred_owner(
    runtime_dir: &std::path::Path,
    platform: ChannelPlatform,
    operation_id: &'static str,
    account_id: &str,
    current_pid: Option<u32>,
) -> CliResult<Option<state::ChannelOperationDuplicateCleanupResult>> {
    let Some(current_pid) = current_pid else {
        return Ok(None);
    };
    let now = channel_runtime_now_ms();
    let Some(runtime) = state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir,
        platform,
        operation_id,
        account_id,
        now,
    ) else {
        return Ok(None);
    };
    if runtime.running_instances <= 1 || runtime.pid != Some(current_pid) {
        return Ok(None);
    }
    if runtime
        .last_duplicate_reclaim_at
        .is_some_and(|last_reclaim_at| {
            now.saturating_sub(last_reclaim_at) < CHANNEL_RUNTIME_DUPLICATE_RECLAIM_COOLDOWN_MS
        })
    {
        return Ok(None);
    }

    let result = state::request_channel_operation_duplicate_cleanup_in_dir(
        runtime_dir,
        platform,
        operation_id,
        Some(account_id),
    )?;
    if matches!(
        result.outcome,
        state::ChannelOperationDuplicateCleanupOutcome::Requested
    ) {
        tracing::info!(
            target: "loong.channel.runtime",
            platform = platform.as_str(),
            operation_id = operation_id,
            account_id = account_id,
            preferred_owner_pid = result.preferred_owner_pid.unwrap_or_default(),
            cleanup_owner_pids = %render_runtime_owner_pid_list(result.targeted_owner_pids.as_slice()),
            "duplicate runtime auto-reclaim requested cooperative shutdown for non-preferred owners"
        );
        return Ok(Some(result));
    }
    Ok(None)
}

fn render_runtime_owner_pid_list(owner_pids: &[u32]) -> String {
    if owner_pids.is_empty() {
        return "-".to_owned();
    }

    owner_pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
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
    with_channel_serve_runtime(spec, move |runtime| async move {
        with_channel_serve_runtime_stop_tasks(
            state::default_channel_runtime_state_dir().as_path(),
            spec,
            runtime,
            stop,
            run,
        )
        .await
    })
    .await
}

#[cfg(test)]
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
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
    with_channel_serve_runtime_in_dir(runtime_dir, process_id, spec, move |runtime| async move {
        with_channel_serve_runtime_stop_tasks(runtime_dir, spec, runtime, stop, run).await
    })
    .await
}

#[cfg(test)]
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
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
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        CHANNEL_RUNTIME_DUPLICATE_RECLAIM_COOLDOWN_MS, ChannelServeRuntimeSpec,
        channel_runtime_now_ms, maybe_reclaim_duplicate_runtime_owners_for_preferred_owner,
        merge_runtime_result,
    };
    use crate::channel::CHANNEL_OPERATION_SERVE_ID;
    use crate::channel::ChannelPlatform;
    use crate::channel::runtime::state;

    fn temp_runtime_dir(suffix: &str) -> PathBuf {
        let unique = format!(
            "loong-channel-serve-{suffix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

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

    #[tokio::test]
    async fn duplicate_auto_reclaim_requests_cleanup_only_for_preferred_owner() {
        let runtime_dir = temp_runtime_dir("duplicate-auto-reclaim");
        let spec = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Weixin,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "default",
            account_label: "default",
        };
        let first = state::start_channel_operation_runtime_tracker_for_test(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            5151,
        )
        .await
        .expect("start first runtime");
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second = state::start_channel_operation_runtime_tracker_for_test(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            6262,
        )
        .await
        .expect("start second runtime");

        let reclaim_result = maybe_reclaim_duplicate_runtime_owners_for_preferred_owner(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            second.pid(),
        )
        .expect("preferred runtime can trigger duplicate cleanup");
        assert!(reclaim_result.is_some());
        tokio::time::timeout(Duration::from_millis(200), first.wait_for_stop_request())
            .await
            .expect("first stop request should become visible")
            .expect("wait for first stop request");
        assert!(
            tokio::time::timeout(Duration::from_millis(100), second.wait_for_stop_request())
                .await
                .is_err(),
            "preferred runtime owner should not receive duplicate cleanup stop request"
        );
        let runtime = state::load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            channel_runtime_now_ms(),
        )
        .expect("load preferred runtime state");
        assert_eq!(runtime.pid, Some(6262));
        assert!(runtime.last_duplicate_reclaim_cleanup_owner_pids.is_empty());
        assert!(runtime.last_duplicate_reclaim_at.is_none());

        first.shutdown().await.expect("shutdown first runtime");
        second.shutdown().await.expect("shutdown second runtime");
    }

    #[tokio::test]
    async fn duplicate_auto_reclaim_respects_cooldown_before_retrying() {
        let runtime_dir = temp_runtime_dir("duplicate-auto-reclaim-cooldown");
        let spec = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Weixin,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "default",
            account_label: "default",
        };
        let first = state::start_channel_operation_runtime_tracker_for_test(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            5151,
        )
        .await
        .expect("start first runtime");
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second = state::start_channel_operation_runtime_tracker_for_test(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            6262,
        )
        .await
        .expect("start second runtime");

        second
            .record_duplicate_reclaim(&[5151])
            .await
            .expect("seed duplicate reclaim cooldown");

        let within_cooldown = maybe_reclaim_duplicate_runtime_owners_for_preferred_owner(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            second.pid(),
        )
        .expect("preferred runtime can inspect duplicate cleanup state");
        assert!(
            within_cooldown.is_none(),
            "duplicate auto-reclaim should skip retries while the cooldown is active"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), first.wait_for_stop_request())
                .await
                .is_err(),
            "cooldown should prevent a new duplicate cleanup stop request"
        );

        tokio::time::sleep(Duration::from_millis(
            CHANNEL_RUNTIME_DUPLICATE_RECLAIM_COOLDOWN_MS + 20,
        ))
        .await;

        let after_cooldown = maybe_reclaim_duplicate_runtime_owners_for_preferred_owner(
            &runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            second.pid(),
        )
        .expect("preferred runtime can retry duplicate cleanup after cooldown");
        assert!(after_cooldown.is_some());
        tokio::time::timeout(Duration::from_millis(200), first.wait_for_stop_request())
            .await
            .expect("first stop request should become visible after cooldown")
            .expect("wait for first stop request after cooldown");

        first.shutdown().await.expect("shutdown first runtime");
        second.shutdown().await.expect("shutdown second runtime");
    }
}
