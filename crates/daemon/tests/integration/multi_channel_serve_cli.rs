use super::*;

use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use loongclaw_daemon::supervisor::{
    BackgroundChannelRunnerRequest, BackgroundChannelSurface, LoadedSupervisorConfig,
    RuntimeOwnerPhase, SupervisorRuntimeHooks, SupervisorShutdownReason, SurfacePhase,
    run_multi_channel_serve_with_hooks_for_test,
};
use tokio::{sync::Notify, time::sleep};

type BoxedCliFuture = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'static>>;
type BoxedShutdownFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send + 'static>>;
type TestBackgroundChannelRunner =
    Arc<dyn Fn(BackgroundChannelRunnerRequest) -> BoxedCliFuture + Send + Sync + 'static>;
const MULTI_CHANNEL_TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
struct EventLog {
    events: Arc<Mutex<Vec<String>>>,
}

impl EventLog {
    fn push(&self, event: impl Into<String>) {
        self.events
            .lock()
            .expect("event log lock")
            .push(event.into());
    }

    fn snapshot(&self) -> Vec<String> {
        self.events.lock().expect("event log lock").clone()
    }
}

fn boxed_cli_result(f: impl Future<Output = CliResult<()>> + Send + 'static) -> BoxedCliFuture {
    Box::pin(f)
}

fn boxed_shutdown_result(
    f: impl Future<Output = CliResult<String>> + Send + 'static,
) -> BoxedShutdownFuture {
    Box::pin(f)
}

fn pending_shutdown_future() -> BoxedShutdownFuture {
    Box::pin(async move {
        std::future::pending::<()>().await;
        Ok(String::new())
    })
}

fn loaded_config_fixture() -> LoadedSupervisorConfig {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;
    LoadedSupervisorConfig {
        resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
        config,
    }
}

fn loaded_config_fixture_with_path(path: &str) -> LoadedSupervisorConfig {
    let mut fixture = loaded_config_fixture();
    fixture.resolved_path = PathBuf::from(path);
    fixture
}

fn all_service_channels_loaded_config_fixture() -> LoadedSupervisorConfig {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = true;
    config.matrix.enabled = true;
    config.wecom.enabled = true;
    LoadedSupervisorConfig {
        resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
        config,
    }
}

fn telegram_only_loaded_config_fixture() -> LoadedSupervisorConfig {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = true;
    config.feishu.enabled = false;
    LoadedSupervisorConfig {
        resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
        config,
    }
}

fn feishu_only_loaded_config_fixture() -> LoadedSupervisorConfig {
    let mut config = mvp::config::LoongClawConfig::default();
    config.telegram.enabled = false;
    config.feishu.enabled = true;
    LoadedSupervisorConfig {
        resolved_path: PathBuf::from("/tmp/loongclaw.toml"),
        config,
    }
}

fn hooks(
    load_config: impl Fn(Option<&str>) -> CliResult<LoadedSupervisorConfig> + Send + Sync + 'static,
    run_cli_host: impl Fn(mvp::chat::ConcurrentCliHostOptions) -> BoxedCliFuture + Send + Sync + 'static,
    run_telegram: impl Fn(BackgroundChannelRunnerRequest) -> BoxedCliFuture + Send + Sync + 'static,
    run_feishu: impl Fn(BackgroundChannelRunnerRequest) -> BoxedCliFuture + Send + Sync + 'static,
    wait_for_shutdown: impl Fn() -> BoxedShutdownFuture + Send + Sync + 'static,
) -> SupervisorRuntimeHooks {
    let telegram_runner = Arc::new(run_telegram);
    let feishu_runner = Arc::new(run_feishu);
    let matrix_runner = idle_background_channel_runner();
    let wecom_runner = idle_background_channel_runner();
    let background_channel_runners = background_channel_runner_registry(vec![
        (
            mvp::channel::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
            telegram_runner,
        ),
        (
            mvp::channel::FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
            feishu_runner,
        ),
        (
            mvp::channel::MATRIX_RUNTIME_COMMAND_DESCRIPTOR,
            matrix_runner,
        ),
        (mvp::channel::WECOM_RUNTIME_COMMAND_DESCRIPTOR, wecom_runner),
    ]);

    SupervisorRuntimeHooks {
        load_config: Arc::new(load_config),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(run_cli_host),
        background_channel_runners,
        wait_for_shutdown: Arc::new(wait_for_shutdown),
        observe_state: Arc::new(|_| Ok(())),
    }
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

fn unique_runtime_dir(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let runtime_dir = std::env::temp_dir().join(format!(
        "loongclaw-daemon-multi-channel-serve-{label}-{suffix}"
    ));
    std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
    runtime_dir
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

fn channel_accounts(
    values: &[(&str, &str)],
) -> Vec<loongclaw_daemon::MultiChannelServeChannelAccount> {
    values
        .iter()
        .map(
            |(channel_id, account_id)| loongclaw_daemon::MultiChannelServeChannelAccount {
                channel_id: (*channel_id).to_owned(),
                account_id: (*account_id).to_owned(),
            },
        )
        .collect()
}

fn telegram_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
    BackgroundChannelSurface::new(
        mvp::channel::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
        account_id,
    )
}

fn feishu_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
    BackgroundChannelSurface::new(mvp::channel::FEISHU_RUNTIME_COMMAND_DESCRIPTOR, account_id)
}

fn matrix_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
    BackgroundChannelSurface::new(mvp::channel::MATRIX_RUNTIME_COMMAND_DESCRIPTOR, account_id)
}

fn wecom_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
    BackgroundChannelSurface::new(mvp::channel::WECOM_RUNTIME_COMMAND_DESCRIPTOR, account_id)
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_starts_all_enabled_runtime_backed_service_channels() {
    let log = EventLog::default();
    let telegram_runner = {
        let log = log.clone();
        Arc::new(move |request: BackgroundChannelRunnerRequest| {
            let log = log.clone();
            boxed_cli_result(async move {
                log.push(format!(
                    "telegram-start account={}",
                    request.account_id.as_deref().unwrap_or("-")
                ));
                while !request.stop.is_requested() {
                    tokio::task::yield_now().await;
                }
                log.push("telegram-stop");
                Ok(())
            })
        })
    };
    let feishu_runner = {
        let log = log.clone();
        Arc::new(move |request: BackgroundChannelRunnerRequest| {
            let log = log.clone();
            boxed_cli_result(async move {
                log.push(format!(
                    "feishu-start account={}",
                    request.account_id.as_deref().unwrap_or("-")
                ));
                while !request.stop.is_requested() {
                    tokio::task::yield_now().await;
                }
                log.push("feishu-stop");
                Ok(())
            })
        })
    };
    let matrix_runner = {
        let log = log.clone();
        Arc::new(move |request: BackgroundChannelRunnerRequest| {
            let log = log.clone();
            boxed_cli_result(async move {
                log.push(format!(
                    "matrix-start account={}",
                    request.account_id.as_deref().unwrap_or("-")
                ));
                while !request.stop.is_requested() {
                    tokio::task::yield_now().await;
                }
                log.push("matrix-stop");
                Ok(())
            })
        })
    };
    let wecom_runner = {
        let log = log.clone();
        Arc::new(move |request: BackgroundChannelRunnerRequest| {
            let log = log.clone();
            boxed_cli_result(async move {
                log.push(format!(
                    "wecom-start account={}",
                    request.account_id.as_deref().unwrap_or("-")
                ));
                while !request.stop.is_requested() {
                    tokio::task::yield_now().await;
                }
                log.push("wecom-stop");
                Ok(())
            })
        })
    };
    let background_channel_runners = background_channel_runner_registry(vec![
        (
            mvp::channel::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
            telegram_runner,
        ),
        (
            mvp::channel::FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
            feishu_runner,
        ),
        (
            mvp::channel::MATRIX_RUNTIME_COMMAND_DESCRIPTOR,
            matrix_runner,
        ),
        (mvp::channel::WECOM_RUNTIME_COMMAND_DESCRIPTOR, wecom_runner),
    ]);
    let state = run_multi_channel_serve_with_hooks_for_test(
        None,
        "cli-supervisor",
        channel_accounts(&[
            ("telegram", "bot_123456"),
            ("feishu", "alerts"),
            ("matrix", "bridge-sync"),
            ("wecom", "robot-prod"),
        ]),
        SupervisorRuntimeHooks {
            load_config: {
                let log = log.clone();
                Arc::new(move |_| {
                    log.push("load-config");
                    Ok(all_service_channels_loaded_config_fixture())
                })
            },
            initialize_runtime_environment: Arc::new(|_| {}),
            run_cli_host: {
                let log = log.clone();
                Arc::new(move |options| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!("cli-start session={}", options.session_id));
                        Ok(())
                    })
                })
            },
            background_channel_runners,
            wait_for_shutdown: Arc::new(pending_shutdown_future),
            observe_state: Arc::new(|_| Ok(())),
        },
    )
    .await
    .expect("run helper");

    assert_eq!(state.phase(), RuntimeOwnerPhase::Stopped);
    assert!(state.final_exit_result().is_ok());
    assert_eq!(
        state
            .surface_state(&telegram_surface(Some("bot_123456")))
            .expect("telegram tracked")
            .phase,
        SurfacePhase::Stopped
    );
    assert_eq!(
        state
            .surface_state(&feishu_surface(Some("alerts")))
            .expect("feishu tracked")
            .phase,
        SurfacePhase::Stopped
    );
    assert_eq!(
        state
            .surface_state(&matrix_surface(Some("bridge-sync")))
            .expect("matrix tracked")
            .phase,
        SurfacePhase::Stopped
    );
    assert_eq!(
        state
            .surface_state(&wecom_surface(Some("robot-prod")))
            .expect("wecom tracked")
            .phase,
        SurfacePhase::Stopped
    );

    let events = log.snapshot();
    assert_eq!(events.first().map(String::as_str), Some("load-config"));
    assert!(
        events
            .iter()
            .any(|event| event == "telegram-start account=bot_123456"),
        "events: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event == "feishu-start account=alerts"),
        "events: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event == "matrix-start account=bridge-sync"),
        "events: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event == "wecom-start account=robot-prod"),
        "events: {events:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_skips_feishu_when_only_telegram_is_enabled() {
    let log = EventLog::default();
    let state = run_multi_channel_serve_with_hooks_for_test(
        None,
        "cli-supervisor",
        channel_accounts(&[("telegram", "bot_123456"), ("feishu", "stale-feishu")]),
        hooks(
            |_| Ok(telegram_only_loaded_config_fixture()),
            {
                let log = log.clone();
                move |options| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!("cli-start session={}", options.session_id));
                        Ok(())
                    })
                }
            },
            {
                let log = log.clone();
                move |request| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!(
                            "telegram-start account={}",
                            request.account_id.as_deref().unwrap_or("-")
                        ));
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        log.push("telegram-stop");
                        Ok(())
                    })
                }
            },
            {
                let log = log.clone();
                move |_| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push("feishu-start");
                        Err("feishu should not start when disabled".to_owned())
                    })
                }
            },
            pending_shutdown_future,
        ),
    )
    .await
    .expect("run helper");

    assert!(state.final_exit_result().is_ok());
    assert_eq!(state.spec().surfaces.len(), 1);
    assert_eq!(
        state.spec().surfaces[0],
        telegram_surface(Some("bot_123456"))
    );

    let events = log.snapshot();
    assert!(
        events
            .iter()
            .any(|event| event == "telegram-start account=bot_123456"),
        "events: {events:?}"
    );
    assert!(
        !events.iter().any(|event| event == "feishu-start"),
        "events: {events:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_skips_telegram_when_only_feishu_is_enabled() {
    let log = EventLog::default();
    let state = run_multi_channel_serve_with_hooks_for_test(
        None,
        "cli-supervisor",
        channel_accounts(&[("telegram", "stale-telegram"), ("feishu", "alerts")]),
        hooks(
            |_| Ok(feishu_only_loaded_config_fixture()),
            {
                let log = log.clone();
                move |options| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!("cli-start session={}", options.session_id));
                        Ok(())
                    })
                }
            },
            {
                let log = log.clone();
                move |_| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push("telegram-start");
                        Err("telegram should not start when disabled".to_owned())
                    })
                }
            },
            {
                let log = log.clone();
                move |request| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!(
                            "feishu-start account={}",
                            request.account_id.as_deref().unwrap_or("-")
                        ));
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        log.push("feishu-stop");
                        Ok(())
                    })
                }
            },
            pending_shutdown_future,
        ),
    )
    .await
    .expect("run helper");

    assert!(state.final_exit_result().is_ok());
    assert_eq!(state.spec().surfaces.len(), 1);
    assert_eq!(state.spec().surfaces[0], feishu_surface(Some("alerts")));

    let events = log.snapshot();
    assert!(
        events
            .iter()
            .any(|event| event == "feishu-start account=alerts"),
        "events: {events:?}"
    );
    assert!(
        !events.iter().any(|event| event == "telegram-start"),
        "events: {events:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_background_failure_exits_foreground_cli_host_with_summarized_shutdown_reason()
 {
    let log = EventLog::default();
    let fail_telegram = Arc::new(Notify::new());
    let run_log = log.clone();
    let run: tokio::task::JoinHandle<CliResult<loongclaw_daemon::supervisor::SupervisorState>> = {
        let fail_telegram_for_run = fail_telegram.clone();
        tokio::spawn(async move {
            run_multi_channel_serve_with_hooks_for_test(
                None,
                "cli-supervisor",
                channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
                hooks(
                    |_| Ok(loaded_config_fixture()),
                    {
                        let log = run_log.clone();
                        move |options| {
                            let log = log.clone();
                            boxed_cli_result(async move {
                                log.push(format!("cli-start session={}", options.session_id));
                                options.shutdown.wait().await;
                                log.push("cli-stop");
                                Ok(())
                            })
                        }
                    },
                    {
                        let log = run_log.clone();
                        move |request| {
                            let log = log.clone();
                            let fail_telegram_for_run = fail_telegram_for_run.clone();
                            boxed_cli_result(async move {
                                log.push("telegram-start");
                                fail_telegram_for_run.notified().await;
                                let _ = request;
                                Err("telegram task exited unexpectedly".to_owned())
                            })
                        }
                    },
                    {
                        let log = run_log.clone();
                        move |request| {
                            let log = log.clone();
                            boxed_cli_result(async move {
                                log.push("feishu-start");
                                while !request.stop.is_requested() {
                                    tokio::task::yield_now().await;
                                }
                                log.push("feishu-stop");
                                Ok(())
                            })
                        }
                    },
                    pending_shutdown_future,
                ),
            )
            .await
        })
    };
    wait_until("telegram background start", || {
        log.snapshot().iter().any(|event| event == "telegram-start")
    })
    .await;
    fail_telegram.notify_waiters();
    let state = run.await.expect("supervisor join").expect("run helper");

    let error = state
        .final_exit_result()
        .expect_err("surface failure should fail the supervisor");
    assert!(
        error.contains(
            "telegram(account=bot_123456) exited unexpectedly: telegram task exited unexpectedly"
        ),
        "error: {error}"
    );
    assert_eq!(
        state.shutdown_reason(),
        Some(&SupervisorShutdownReason::SurfaceFailed {
            surface: telegram_surface(Some("bot_123456")),
            error: "telegram task exited unexpectedly".to_owned(),
        })
    );
    assert!(
        log.snapshot().iter().any(|event| event == "cli-stop"),
        "cli host should stop after the background failure"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_background_failure_before_cli_wait_still_stops_foreground_cli_host() {
    let log = EventLog::default();
    let state = tokio::time::timeout(
        MULTI_CHANNEL_TEST_TIMEOUT,
        run_multi_channel_serve_with_hooks_for_test(
            None,
            "cli-supervisor",
            channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
            hooks(
                |_| Ok(loaded_config_fixture()),
                {
                    let log = log.clone();
                    move |options| {
                        let log = log.clone();
                        boxed_cli_result(async move {
                            log.push("cli-start");
                            sleep(Duration::from_millis(50)).await;
                            log.push("cli-await-shutdown");
                            options.shutdown.wait().await;
                            log.push("cli-stop");
                            Ok(())
                        })
                    }
                },
                {
                    let log = log.clone();
                    move |request| {
                        let log = log.clone();
                        boxed_cli_result(async move {
                            log.push("telegram-fail");
                            let _ = request;
                            Err("telegram task exited unexpectedly".to_owned())
                        })
                    }
                },
                move |request| {
                    boxed_cli_result(async move {
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                },
                pending_shutdown_future,
            ),
        ),
    )
    .await
    .expect("supervisor should not hang after an early background failure")
    .expect("run helper");

    let error = state
        .final_exit_result()
        .expect_err("surface failure should fail the supervisor");
    assert!(
        error.contains(
            "telegram(account=bot_123456) exited unexpectedly: telegram task exited unexpectedly"
        ),
        "error: {error}"
    );

    let events = log.snapshot();
    let telegram_fail = events
        .iter()
        .position(|event| event == "telegram-fail")
        .expect("telegram failure logged");
    let cli_wait = events
        .iter()
        .position(|event| event == "cli-await-shutdown")
        .expect("cli wait logged");
    assert!(
        telegram_fail < cli_wait,
        "expected shutdown to be requested before CLI started waiting: {events:?}"
    );
    assert!(
        events.iter().any(|event| event == "cli-stop"),
        "cli host should still stop after shutdown was requested early: {events:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_background_join_error_still_shuts_down_cli_and_other_surfaces() {
    let log = EventLog::default();
    let run_log = log.clone();
    let run: tokio::task::JoinHandle<CliResult<loongclaw_daemon::supervisor::SupervisorState>> =
        tokio::spawn(async move {
            run_multi_channel_serve_with_hooks_for_test(
                None,
                "cli-supervisor",
                channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
                hooks(
                    |_| Ok(loaded_config_fixture()),
                    {
                        let log = run_log.clone();
                        move |options| {
                            let log = log.clone();
                            boxed_cli_result(async move {
                                log.push("cli-start");
                                options.shutdown.wait().await;
                                log.push("cli-stop");
                                Ok(())
                            })
                        }
                    },
                    {
                        let log = run_log.clone();
                        move |_| {
                            let log = log.clone();
                            boxed_cli_result(async move {
                                log.push("telegram-panic");
                                panic!("telegram runner panicked");
                            })
                        }
                    },
                    {
                        let log = run_log.clone();
                        move |request| {
                            let log = log.clone();
                            boxed_cli_result(async move {
                                log.push("feishu-start");
                                while !request.stop.is_requested() {
                                    tokio::task::yield_now().await;
                                }
                                log.push("feishu-stop");
                                Ok(())
                            })
                        }
                    },
                    pending_shutdown_future,
                ),
            )
            .await
        });

    let state = tokio::time::timeout(MULTI_CHANNEL_TEST_TIMEOUT, run)
        .await
        .expect("supervisor should not hang after a background join error")
        .expect("supervisor join")
        .expect("run helper");

    let error = state
        .final_exit_result()
        .expect_err("background join error should fail the supervisor");
    assert!(
        error.contains("telegram(account=bot_123456) exited unexpectedly"),
        "error: {error}"
    );
    assert!(error.contains("failed to join"), "error: {error}");

    let events = log.snapshot();
    assert!(events.iter().any(|event| event == "telegram-panic"));
    assert!(
        events.iter().any(|event| event == "cli-stop"),
        "events: {events:?}"
    );
    assert!(
        events.iter().any(|event| event == "feishu-stop"),
        "events: {events:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_keeps_cli_session_distinct_from_channel_sessions() {
    let log = EventLog::default();
    let state = run_multi_channel_serve_with_hooks_for_test(
        None,
        "cli-supervisor",
        channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
        hooks(
            |_| Ok(loaded_config_fixture()),
            {
                let log = log.clone();
                move |options| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!("cli-session={}", options.session_id));
                        Ok(())
                    })
                }
            },
            {
                let log = log.clone();
                move |request| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!(
                            "telegram-account={}",
                            request.account_id.as_deref().unwrap_or("-")
                        ));
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                }
            },
            {
                let log = log.clone();
                move |request| {
                    let log = log.clone();
                    boxed_cli_result(async move {
                        log.push(format!(
                            "feishu-account={}",
                            request.account_id.as_deref().unwrap_or("-")
                        ));
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                }
            },
            pending_shutdown_future,
        ),
    )
    .await
    .expect("run helper");

    assert!(state.final_exit_result().is_ok());
    let events = log.snapshot();
    assert!(
        events
            .iter()
            .any(|event| event == "cli-session=cli-supervisor")
    );
    assert!(
        events
            .iter()
            .any(|event| event == "telegram-account=bot_123456")
    );
    assert!(events.iter().any(|event| event == "feishu-account=alerts"));
    assert!(
        !events
            .iter()
            .any(|event| event == "telegram-account=cli-supervisor"
                || event == "feishu-account=cli-supervisor"),
        "background channel runners must not reuse the CLI session id: {events:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_loads_config_once_before_spawning_children() {
    let load_count = Arc::new(AtomicUsize::new(0));
    let log = EventLog::default();
    let state = run_multi_channel_serve_with_hooks_for_test(
        Some("/tmp/loongclaw.toml"),
        "cli-supervisor",
        channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
        hooks(
            {
                let load_count = load_count.clone();
                let log = log.clone();
                move |config_path| {
                    log.push(format!("load-config path={}", config_path.unwrap_or("-")));
                    load_count.fetch_add(1, Ordering::SeqCst);
                    Ok(loaded_config_fixture())
                }
            },
            {
                let load_count = load_count.clone();
                let log = log.clone();
                move |_options| {
                    let log = log.clone();
                    let load_count = load_count.clone();
                    boxed_cli_result(async move {
                        assert_eq!(load_count.load(Ordering::SeqCst), 1);
                        log.push("cli-start");
                        Ok(())
                    })
                }
            },
            {
                let load_count = load_count.clone();
                let log = log.clone();
                move |request| {
                    let log = log.clone();
                    let load_count = load_count.clone();
                    boxed_cli_result(async move {
                        assert_eq!(load_count.load(Ordering::SeqCst), 1);
                        log.push(format!("telegram-path={}", request.resolved_path.display()));
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                }
            },
            {
                let load_count = load_count.clone();
                let log = log.clone();
                move |request| {
                    let log = log.clone();
                    let load_count = load_count.clone();
                    boxed_cli_result(async move {
                        assert_eq!(load_count.load(Ordering::SeqCst), 1);
                        log.push(format!("feishu-path={}", request.resolved_path.display()));
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                }
            },
            pending_shutdown_future,
        ),
    )
    .await
    .expect("run helper");

    assert!(state.final_exit_result().is_ok());
    assert_eq!(load_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        log.snapshot().first().map(String::as_str),
        Some("load-config path=/tmp/loongclaw.toml")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_initializes_runtime_environment_before_spawning_children() {
    let expected_path = "/tmp/loongclaw-supervisor-runtime-env.toml";
    let expected_path_string = expected_path.to_owned();
    let initialized_config_path = Arc::new(Mutex::new(None::<String>));
    let started = Arc::new(AtomicUsize::new(0));
    let background_channel_runners = background_channel_runner_registry(vec![
        (
            mvp::channel::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
            Arc::new({
                let initialized_config_path = initialized_config_path.clone();
                let started = started.clone();
                let expected_path_string = expected_path_string.clone();
                move |request: BackgroundChannelRunnerRequest| {
                    let initialized_config_path = initialized_config_path.clone();
                    let started = started.clone();
                    let expected_path_string = expected_path_string.clone();
                    boxed_cli_result(async move {
                        let observed_path = initialized_config_path
                            .lock()
                            .expect("initialized config path lock")
                            .clone();
                        assert_eq!(
                            observed_path.as_deref(),
                            Some(expected_path_string.as_str())
                        );
                        assert!(!request.initialize_runtime_environment);
                        started.fetch_add(1, Ordering::SeqCst);
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                }
            }),
        ),
        (
            mvp::channel::FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
            Arc::new({
                let initialized_config_path = initialized_config_path.clone();
                let started = started.clone();
                let expected_path_string = expected_path_string.clone();
                move |request: BackgroundChannelRunnerRequest| {
                    let initialized_config_path = initialized_config_path.clone();
                    let started = started.clone();
                    let expected_path_string = expected_path_string.clone();
                    boxed_cli_result(async move {
                        let observed_path = initialized_config_path
                            .lock()
                            .expect("initialized config path lock")
                            .clone();
                        assert_eq!(
                            observed_path.as_deref(),
                            Some(expected_path_string.as_str())
                        );
                        assert!(!request.initialize_runtime_environment);
                        started.fetch_add(1, Ordering::SeqCst);
                        while !request.stop.is_requested() {
                            tokio::task::yield_now().await;
                        }
                        Ok(())
                    })
                }
            }),
        ),
        (
            mvp::channel::MATRIX_RUNTIME_COMMAND_DESCRIPTOR,
            idle_background_channel_runner(),
        ),
        (
            mvp::channel::WECOM_RUNTIME_COMMAND_DESCRIPTOR,
            idle_background_channel_runner(),
        ),
    ]);

    let state = run_multi_channel_serve_with_hooks_for_test(
        None,
        "cli-supervisor",
        channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
        SupervisorRuntimeHooks {
            load_config: Arc::new(move |_| Ok(loaded_config_fixture_with_path(expected_path))),
            initialize_runtime_environment: Arc::new({
                let initialized_config_path = initialized_config_path.clone();
                move |loaded_config| {
                    // The runtime env module has its own coverage; this test only needs to prove
                    // the supervisor runs the hook before spawning children.
                    let mut guard = initialized_config_path
                        .lock()
                        .expect("initialized config path lock");
                    *guard = Some(loaded_config.resolved_path.display().to_string());
                }
            }),
            run_cli_host: Arc::new({
                let initialized_config_path = initialized_config_path.clone();
                let started = started.clone();
                let expected_path_string = expected_path_string.clone();
                move |options| {
                    let initialized_config_path = initialized_config_path.clone();
                    let started = started.clone();
                    let expected_path_string = expected_path_string.clone();
                    boxed_cli_result(async move {
                        while started.load(Ordering::SeqCst) < 2 {
                            tokio::task::yield_now().await;
                        }

                        let observed_path = initialized_config_path
                            .lock()
                            .expect("initialized config path lock")
                            .clone();
                        assert_eq!(
                            observed_path.as_deref(),
                            Some(expected_path_string.as_str())
                        );
                        assert!(!options.initialize_runtime_environment);
                        Ok(())
                    })
                }
            }),
            background_channel_runners,
            wait_for_shutdown: Arc::new(pending_shutdown_future),
            observe_state: Arc::new(|_| Ok(())),
        },
    )
    .await
    .expect("run helper");

    assert!(state.final_exit_result().is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_ctrl_c_waits_for_background_joins_and_reports_shutdown_reason() {
    let ctrl_c = Arc::new(Notify::new());
    let release_telegram = Arc::new(Notify::new());
    let log = EventLog::default();
    let run: tokio::task::JoinHandle<CliResult<loongclaw_daemon::supervisor::SupervisorState>> =
        tokio::spawn({
            let ctrl_c = ctrl_c.clone();
            let release_telegram = release_telegram.clone();
            let log = log.clone();
            async move {
                run_multi_channel_serve_with_hooks_for_test(
                    None,
                    "cli-supervisor",
                    channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
                    hooks(
                        |_| Ok(loaded_config_fixture()),
                        {
                            let log = log.clone();
                            move |options| {
                                let log = log.clone();
                                boxed_cli_result(async move {
                                    log.push("cli-start");
                                    options.shutdown.wait().await;
                                    log.push("cli-stop");
                                    Ok(())
                                })
                            }
                        },
                        {
                            let log = log.clone();
                            let release_telegram = release_telegram.clone();
                            move |request| {
                                let log = log.clone();
                                let release_telegram = release_telegram.clone();
                                boxed_cli_result(async move {
                                    log.push("telegram-start");
                                    while !request.stop.is_requested() {
                                        tokio::task::yield_now().await;
                                    }
                                    log.push("telegram-stop-requested");
                                    release_telegram.notified().await;
                                    log.push("telegram-joined");
                                    Ok(())
                                })
                            }
                        },
                        {
                            let log = log.clone();
                            move |request| {
                                let log = log.clone();
                                boxed_cli_result(async move {
                                    log.push("feishu-start");
                                    while !request.stop.is_requested() {
                                        tokio::task::yield_now().await;
                                    }
                                    log.push("feishu-joined");
                                    Ok(())
                                })
                            }
                        },
                        move || {
                            let ctrl_c = ctrl_c.clone();
                            let log = log.clone();
                            boxed_shutdown_result(async move {
                                ctrl_c.notified().await;
                                log.push("ctrl-c");
                                Ok("ctrl-c received".to_owned())
                            })
                        },
                    ),
                )
                .await
            }
        });

    wait_until("background and cli startup", || {
        let events = log.snapshot();
        events.iter().any(|event| event == "cli-start")
            && events.iter().any(|event| event == "telegram-start")
            && events.iter().any(|event| event == "feishu-start")
    })
    .await;

    ctrl_c.notify_waiters();
    wait_until("cooperative stop request", || {
        log.snapshot()
            .iter()
            .any(|event| event == "telegram-stop-requested")
    })
    .await;
    assert!(
        !run.is_finished(),
        "supervisor should wait for background joins after Ctrl-C"
    );

    release_telegram.notify_waiters();
    let state = run.await.expect("supervisor join").expect("run helper");

    assert_eq!(state.phase(), RuntimeOwnerPhase::Stopped);
    assert!(state.final_exit_result().is_ok());
    assert!(
        state
            .final_exit_summary()
            .contains("shutdown requested: ctrl-c received"),
        "summary: {}",
        state.final_exit_summary()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn multi_channel_serve_cooperative_stop_clears_channel_runtime_running_state() {
    let temp_home = unique_runtime_dir("cooperative-stop-home");
    let runtime_dir = temp_home.join(".loongclaw").join("channel-runtime");
    let _env =
        MigrationEnvironmentGuard::set(&[("HOME", Some(temp_home.to_string_lossy().as_ref()))]);
    let runtime_entered = Arc::new(Notify::new());
    let ctrl_c = Arc::new(Notify::new());

    let run: tokio::task::JoinHandle<CliResult<loongclaw_daemon::supervisor::SupervisorState>> = {
        let runtime_entered = runtime_entered.clone();
        let ctrl_c = ctrl_c.clone();
        tokio::spawn(async move {
            run_multi_channel_serve_with_hooks_for_test(
                None,
                "cli-supervisor",
                channel_accounts(&[("telegram", "bot_123456"), ("feishu", "alerts")]),
                hooks(
                    |_| Ok(loaded_config_fixture()),
                    move |options| {
                        boxed_cli_result(async move {
                            options.shutdown.wait().await;
                            Ok(())
                        })
                    },
                    move |request| {
                        let runtime_entered = runtime_entered.clone();
                        boxed_cli_result(async move {
                            mvp::channel::run_channel_serve_runtime_probe_for_test(
                                mvp::channel::ChannelPlatform::Telegram,
                                request.account_id.as_deref().unwrap_or("bot_123456"),
                                "bot:123456",
                                request.stop,
                                runtime_entered,
                            )
                            .await
                        })
                    },
                    move |request| {
                        boxed_cli_result(async move {
                            while !request.stop.is_requested() {
                                tokio::task::yield_now().await;
                            }
                            Ok(())
                        })
                    },
                    move || {
                        let ctrl_c = ctrl_c.clone();
                        boxed_shutdown_result(async move {
                            ctrl_c.notified().await;
                            Ok("ctrl-c received".to_owned())
                        })
                    },
                ),
            )
            .await
        })
    };

    runtime_entered.notified().await;
    ctrl_c.notify_waiters();
    let state = run.await.expect("supervisor join").expect("run helper");

    assert!(state.final_exit_result().is_ok());

    let runtime = mvp::channel::load_channel_operation_runtime_for_account_from_dir_for_test(
        runtime_dir.as_path(),
        mvp::channel::ChannelPlatform::Telegram,
        "serve",
        "bot_123456",
        0,
    )
    .expect("runtime snapshot should exist after cooperative shutdown");
    assert!(
        !runtime.running,
        "cooperative stop should clear the runtime running flag"
    );
}
