use super::*;

use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use loongclaw_daemon::{
    gateway::{
        client::{GatewayLocalClient, GatewayStopResponseOutcome},
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
use serde_json::Value;
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

async fn wait_for_gateway_control_surface(
    runtime_dir: &std::path::Path,
) -> loongclaw_daemon::gateway::state::GatewayOwnerStatus {
    wait_until("gateway control surface binding", || {
        let status = load_gateway_owner_status(runtime_dir);
        let Some(status) = status else {
            return false;
        };

        status.running
            && status.bind_address.is_some()
            && status.port.is_some()
            && status.token_path.is_some()
    })
    .await;

    load_gateway_owner_status(runtime_dir)
        .expect("gateway control surface status should be present")
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

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_localhost_control_surface_requires_auth_and_stops_runtime() {
    let runtime_dir = unique_runtime_dir("localhost-control");
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

    let running_status = wait_for_gateway_control_surface(runtime_dir.as_path()).await;
    assert_eq!(running_status.bind_address.as_deref(), Some("127.0.0.1"));
    let port = running_status
        .port
        .expect("control surface port should be persisted");
    let token_path = running_status
        .token_path
        .clone()
        .expect("control surface token path should be persisted");
    let token_path = PathBuf::from(token_path);
    assert!(token_path.exists());

    let token = fs::read_to_string(token_path.as_path()).expect("read gateway control token file");
    let token = token.trim().to_owned();
    assert!(!token.is_empty());

    let base_url = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let unauthorized_status_response = client
        .get(format!("{base_url}/api/gateway/status"))
        .send()
        .await
        .expect("send unauthorized gateway status request");
    assert_eq!(
        unauthorized_status_response.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let unauthorized_status_json: Value = unauthorized_status_response
        .json()
        .await
        .expect("decode unauthorized gateway status response");
    assert_eq!(unauthorized_status_json["error"]["code"], "unauthorized");

    let authorized_status_response = client
        .get(format!("{base_url}/api/gateway/status"))
        .bearer_auth(token.as_str())
        .send()
        .await
        .expect("send authorized gateway status request");
    assert_eq!(authorized_status_response.status(), reqwest::StatusCode::OK);
    let authorized_status_json: Value = authorized_status_response
        .json()
        .await
        .expect("decode authorized gateway status response");
    assert_eq!(authorized_status_json["phase"], "running");
    assert_eq!(authorized_status_json["bind_address"], "127.0.0.1");
    assert_eq!(
        authorized_status_json["port"].as_u64(),
        Some(u64::from(port))
    );

    let channels_response = client
        .get(format!("{base_url}/api/gateway/channels"))
        .bearer_auth(token.as_str())
        .send()
        .await
        .expect("send gateway channels request");
    assert_eq!(channels_response.status(), reqwest::StatusCode::OK);
    let channels_json: Value = channels_response
        .json()
        .await
        .expect("decode gateway channels response");
    assert_eq!(
        channels_json["schema"]["primary_channel_view"],
        "channel_surfaces"
    );
    assert_eq!(channels_json["schema"]["catalog_view"], "channel_catalog");

    let runtime_snapshot_response = client
        .get(format!("{base_url}/api/gateway/runtime-snapshot"))
        .bearer_auth(token.as_str())
        .send()
        .await
        .expect("send gateway runtime snapshot request");
    assert_eq!(runtime_snapshot_response.status(), reqwest::StatusCode::OK);
    let runtime_snapshot_json: Value = runtime_snapshot_response
        .json()
        .await
        .expect("decode gateway runtime snapshot response");
    assert_eq!(
        runtime_snapshot_json["schema"]["surface"],
        "runtime_snapshot"
    );
    assert_eq!(
        runtime_snapshot_json["channels"]["inventory"]["schema"]["catalog_view"],
        "channel_catalog"
    );
    assert!(
        runtime_snapshot_json["tools"]["visible_tool_count"]
            .as_u64()
            .is_some()
    );

    let stop_response = client
        .post(format!("{base_url}/api/gateway/stop"))
        .bearer_auth(token.as_str())
        .send()
        .await
        .expect("send gateway stop request");
    assert_eq!(stop_response.status(), reqwest::StatusCode::ACCEPTED);
    let stop_json: Value = stop_response
        .json()
        .await
        .expect("decode gateway stop response");
    assert_eq!(stop_json["outcome"], "requested");

    let supervisor = timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop after control stop")
        .expect("join gateway run after control stop")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());

    let stopped_status = load_gateway_owner_status(runtime_dir.as_path())
        .expect("stopped gateway status should be present");
    assert_eq!(stopped_status.phase, "stopped");
    assert!(!stopped_status.running);
    assert_eq!(stopped_status.bind_address, None);
    assert_eq!(stopped_status.port, None);
    assert_eq!(stopped_status.token_path, None);
    assert!(!token_path.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_turn_endpoint_rejects_when_acp_disabled_by_policy() {
    let runtime_dir = unique_runtime_dir("turn-policy-disabled");
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

    let running_status = wait_for_gateway_control_surface(runtime_dir.as_path()).await;
    let port = running_status
        .port
        .expect("gateway control surface port should be persisted");
    let token_path = PathBuf::from(
        running_status
            .token_path
            .clone()
            .expect("control surface token path should be persisted"),
    );
    let token = fs::read_to_string(token_path.as_path()).expect("read gateway control token file");
    let token = token.trim().to_owned();
    assert!(!token.is_empty());

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://127.0.0.1:{port}/v1/turn"))
        .bearer_auth(token.as_str())
        .json(&serde_json::json!({
            "session_id": "gateway-policy-disabled",
            "input": "hello",
        }))
        .send()
        .await
        .expect("send gateway turn request");

    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let response_json: Value = response.json().await.expect("decode gateway turn response");
    assert_eq!(
        response_json["error"],
        "ACP is disabled by policy (`acp.enabled=false`)"
    );

    request_gateway_stop(runtime_dir.as_path()).expect("request gateway stop");
    let supervisor = timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop")
        .expect("join gateway run")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owner_state_local_client_discovers_owner_reads_summary_and_stops_runtime() {
    let runtime_dir = unique_runtime_dir("local-client");
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

    let running_status = wait_for_gateway_control_surface(runtime_dir.as_path()).await;
    let expected_port = running_status
        .port
        .expect("gateway control surface port should be persisted");
    let client =
        GatewayLocalClient::discover(runtime_dir.as_path()).expect("discover gateway local client");

    assert_eq!(client.discovery().socket_address().port(), expected_port);
    assert_eq!(
        client.discovery().base_url(),
        format!("http://127.0.0.1:{expected_port}")
    );

    let status = client.status().await.expect("read gateway status");
    assert_eq!(status.phase, "running");
    assert_eq!(status.bind_address.as_deref(), Some("127.0.0.1"));

    let channels = client.channels().await.expect("read gateway channels");
    assert_eq!(
        channels["schema"]["primary_channel_view"],
        "channel_surfaces"
    );
    assert_eq!(channels["schema"]["catalog_view"], "channel_catalog");

    let runtime_snapshot = client
        .runtime_snapshot()
        .await
        .expect("read gateway runtime snapshot");
    assert_eq!(runtime_snapshot["schema"]["surface"], "runtime_snapshot");

    let operator_summary = client
        .operator_summary()
        .await
        .expect("read gateway operator summary");
    assert_eq!(operator_summary.owner.phase, "running");
    assert_eq!(
        operator_summary.control_surface.base_url.as_deref(),
        Some(client.discovery().base_url())
    );
    assert!(operator_summary.control_surface.loopback_only);
    assert_eq!(
        operator_summary.channels.enabled_service_channel_count,
        runtime_snapshot["channels"]["enabled_service_channel_ids"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default()
    );
    assert_eq!(
        operator_summary.runtime.visible_tool_count,
        runtime_snapshot["tools"]["visible_tool_count"]
            .as_u64()
            .map(|value| value as usize)
            .unwrap_or_default()
    );

    let stop = client.stop().await.expect("request gateway stop");
    assert_eq!(stop.outcome, GatewayStopResponseOutcome::Requested);

    let supervisor = timeout(GATEWAY_OWNER_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop after local client stop")
        .expect("join gateway run after local client stop")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());

    let stopped_status = load_gateway_owner_status(runtime_dir.as_path())
        .expect("stopped gateway status should be present");
    assert_eq!(stopped_status.phase, "stopped");
    assert!(!stopped_status.running);
}
