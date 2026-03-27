use super::*;

use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use loongclaw_daemon::{
    gateway::{
        service::{
            run_gateway_run_with_hooks_for_test,
            run_multi_channel_serve_gateway_compat_with_hooks_for_test,
        },
        state::{
            GatewayOwnerMode, GatewayStopRequestOutcome, load_gateway_owner_status,
            request_gateway_stop,
        },
    },
    supervisor::{BackgroundChannelRunnerRequest, LoadedSupervisorConfig, SupervisorRuntimeHooks},
};
use tokio::time::{sleep, timeout};

type BoxedCliFuture = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'static>>;
type BoxedShutdownFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send + 'static>>;
type TestBackgroundChannelRunner =
    Arc<dyn Fn(BackgroundChannelRunnerRequest) -> BoxedCliFuture + Send + Sync + 'static>;

const GATEWAY_OWNER_TEST_TIMEOUT: Duration = Duration::from_secs(2);

fn boxed_cli_result(f: impl Future<Output = CliResult<()>> + Send + 'static) -> BoxedCliFuture {
    Box::pin(f)
}

fn pending_shutdown_future() -> BoxedShutdownFuture {
    Box::pin(async move {
        std::future::pending::<()>().await;
        Ok(String::new())
    })
}

fn unique_runtime_dir(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let runtime_dir = std::env::temp_dir().join(format!(
        "loongclaw-daemon-gateway-owner-state-{label}-{suffix}"
    ));
    std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
    runtime_dir
}

fn headless_loaded_config_fixture() -> LoadedSupervisorConfig {
    LoadedSupervisorConfig {
        resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
        config: mvp::config::LoongClawConfig::default(),
    }
}

fn telegram_loaded_config_fixture() -> LoadedSupervisorConfig {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    LoadedSupervisorConfig {
        resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
        config,
    }
}

fn idle_background_channel_runner() -> TestBackgroundChannelRunner {
    Arc::new(|request| {
        boxed_cli_result(async move {
            while !request.stop.is_requested() {
                tokio::task::yield_now().await;
            }
            Ok(())
        })
    })
}

fn background_channel_runner_registry(
    entries: Vec<(
        mvp::channel::ChannelRuntimeCommandDescriptor,
        TestBackgroundChannelRunner,
    )>,
) -> BTreeMap<&'static str, TestBackgroundChannelRunner> {
    let mut runners = BTreeMap::new();
    for (runtime, runner) in entries {
        runners.insert(runtime.channel_id, runner);
    }
    runners
}

async fn wait_until(description: &str, predicate: impl Fn() -> bool) {
    for _ in 0..200 {
        if predicate() {
            return;
        }
        sleep(Duration::from_millis(5)).await;
    }

    panic!("timed out waiting for {description}");
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_headless_run_claims_slot_and_stops_via_stop_request() {
    let runtime_dir = unique_runtime_dir("headless-stop");
    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(|_| Ok(headless_loaded_config_fixture())),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };

    let runtime_dir_for_run = runtime_dir.clone();
    let run = tokio::spawn(async move {
        run_gateway_run_with_hooks_for_test(
            None,
            None,
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_until("gateway headless status", || {
        load_gateway_owner_status(runtime_dir.as_path())
            .map(|status| status.running && status.phase == "running")
            .unwrap_or(false)
    })
    .await;

    let running_status =
        load_gateway_owner_status(runtime_dir.as_path()).expect("gateway status should be present");
    assert_eq!(running_status.phase, "running");
    assert_eq!(running_status.mode, GatewayOwnerMode::GatewayHeadless);
    assert_eq!(running_status.configured_surface_count, 0);
    assert_eq!(running_status.running_surface_count, 0);

    let stop_result = request_gateway_stop(runtime_dir.as_path()).expect("request gateway stop");
    assert_eq!(stop_result, GatewayStopRequestOutcome::Requested);

    let supervisor = timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop")
        .expect("join gateway run")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());

    let stopped_status = load_gateway_owner_status(runtime_dir.as_path())
        .expect("stopped gateway status should be present");
    assert_eq!(stopped_status.phase, "stopped");
    assert!(!stopped_status.running);
    assert_eq!(
        stopped_status.shutdown_reason.as_deref(),
        Some("shutdown requested: gateway stop requested")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_rejects_second_active_owner_slot() {
    let runtime_dir = unique_runtime_dir("exclusive-slot");
    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(|_| Ok(headless_loaded_config_fixture())),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };

    let runtime_dir_for_run = runtime_dir.clone();
    let run = tokio::spawn(async move {
        run_gateway_run_with_hooks_for_test(
            None,
            None,
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_until("first gateway owner", || {
        load_gateway_owner_status(runtime_dir.as_path())
            .map(|status| status.running)
            .unwrap_or(false)
    })
    .await;

    let second_hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(|_| Ok(headless_loaded_config_fixture())),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };
    let second_result = run_gateway_run_with_hooks_for_test(
        None,
        None,
        Vec::new(),
        runtime_dir.as_path(),
        second_hooks,
    )
    .await
    .expect_err("second gateway owner should be rejected");
    assert!(
        second_result.contains("gateway owner already active"),
        "unexpected duplicate-owner error: {second_result}"
    );

    request_gateway_stop(runtime_dir.as_path()).expect("request gateway stop");
    timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("first gateway run should stop")
        .expect("join gateway run")
        .expect("first gateway run should return supervisor state");
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_second_owner_attempt_preserves_pending_stop_request() {
    let runtime_dir = unique_runtime_dir("duplicate-start-pending-stop");
    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(|_| Ok(headless_loaded_config_fixture())),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };

    let runtime_dir_for_run = runtime_dir.clone();
    let run = tokio::spawn(async move {
        run_gateway_run_with_hooks_for_test(
            None,
            None,
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_until("first gateway owner", || {
        load_gateway_owner_status(runtime_dir.as_path())
            .map(|status| status.running)
            .unwrap_or(false)
    })
    .await;

    let stop_result = request_gateway_stop(runtime_dir.as_path()).expect("request stop");
    assert_eq!(stop_result, GatewayStopRequestOutcome::Requested);

    let second_hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(|_| Ok(headless_loaded_config_fixture())),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };
    let second_result = run_gateway_run_with_hooks_for_test(
        None,
        None,
        Vec::new(),
        runtime_dir.as_path(),
        second_hooks,
    )
    .await
    .expect_err("second gateway owner should be rejected");
    assert!(
        second_result.contains("gateway owner already active"),
        "unexpected duplicate-owner error: {second_result}"
    );

    let supervisor = timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("first gateway run should still stop")
        .expect("join gateway run")
        .expect("first gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_multi_channel_compat_records_wrapper_mode_and_session() {
    let runtime_dir = unique_runtime_dir("compat-wrapper");
    let telegram_runner = idle_background_channel_runner();
    let background_channel_runners = background_channel_runner_registry(vec![(
        mvp::channel::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
        telegram_runner,
    )]);
    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(|_| Ok(telegram_loaded_config_fixture())),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|options| {
            boxed_cli_result(async move {
                options.shutdown.wait().await;
                Ok(())
            })
        }),
        background_channel_runners,
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };

    let runtime_dir_for_run = runtime_dir.clone();
    let run = tokio::spawn(async move {
        run_multi_channel_serve_gateway_compat_with_hooks_for_test(
            None,
            "cli-supervisor",
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_until("multi-channel compatibility status", || {
        load_gateway_owner_status(runtime_dir.as_path())
            .map(|status| status.running && status.running_surface_count == 1)
            .unwrap_or(false)
    })
    .await;

    let running_status =
        load_gateway_owner_status(runtime_dir.as_path()).expect("compat status should be present");
    assert_eq!(running_status.mode, GatewayOwnerMode::MultiChannelServe);
    assert_eq!(
        running_status.attached_cli_session.as_deref(),
        Some("cli-supervisor")
    );
    assert_eq!(running_status.configured_surface_count, 1);
    assert_eq!(running_status.running_surface_count, 1);

    request_gateway_stop(runtime_dir.as_path()).expect("request compatibility stop");
    let supervisor = timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("compat run should stop")
        .expect("join compat run")
        .expect("compat run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());
}
