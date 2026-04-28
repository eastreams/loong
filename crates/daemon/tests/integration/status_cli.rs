use super::*;
use loong_contracts::SecretRef;
use loong_daemon::{
    CliResult,
    gateway::{
        service::run_gateway_run_with_hooks_for_test,
        state::{
            default_gateway_runtime_state_dir, load_gateway_owner_status, request_gateway_stop,
        },
    },
    supervisor::{LoadedSupervisorConfig, SupervisorRuntimeHooks},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::{sleep, timeout};

type BoxedShutdownFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send + 'static>>;

const STATUS_GATEWAY_TEST_TIMEOUT: Duration = Duration::from_secs(2);
const STATUS_GATEWAY_WAIT_ATTEMPTS: usize = 400;
const STATUS_GATEWAY_WAIT_INTERVAL: Duration = Duration::from_millis(10);

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT_TEMP_DIR_SEED: AtomicUsize = AtomicUsize::new(1);
    let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let process_id = std::process::id();
    std::env::temp_dir().join(format!("{prefix}-{process_id}-{seed}-{nanos}"))
}

fn write_status_config(
    root: &Path,
    acp_enabled: bool,
    tool_schema_mode: mvp::config::ProviderToolSchemaModeConfig,
) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let sqlite_path = root.join("memory.sqlite3");
    let mut config = mvp::config::LoongConfig::default();
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(root.display().to_string());
    config.gateway.port = 0;
    config.set_active_provider_profile(
        "demo-openai",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "gpt-4.1-mini".to_owned(),
                api_key: Some(SecretRef::Inline("demo-token".to_owned())),
                tool_schema_mode,
                ..Default::default()
            },
        },
    );

    if acp_enabled {
        config.acp.enabled = true;
        config.acp.dispatch.enabled = true;
        config.acp.default_agent = Some("codex".to_owned());
        config.acp.allowed_agents = vec!["codex".to_owned()];
    }

    let config_path = root.join("loong.toml");
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");
    mvp::config::write(Some(config_path_text), &config, true).expect("write config fixture");
    config_path
}

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn pending_shutdown_future() -> BoxedShutdownFuture {
    Box::pin(async move {
        std::future::pending::<()>().await;
        Ok(String::new())
    })
}

fn load_status_config_fixture(config_path: &Path) -> LoadedSupervisorConfig {
    let config_path_text = config_path
        .to_str()
        .expect("status config path should be valid utf-8");
    let (resolved_path, config) =
        mvp::config::load(Some(config_path_text)).expect("load status config fixture");

    LoadedSupervisorConfig {
        resolved_path,
        config,
    }
}

fn seed_approved_pairing_device(config: &mvp::config::LoongConfig) {
    let memory_config =
        loong_daemon::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
            &config.memory,
        );
    let session_store_config =
        loong_daemon::mvp::session::store::SessionStoreConfig::from(&memory_config);
    let registry =
        loong_daemon::mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(
            session_store_config,
        )
        .expect("pairing registry");
    let requested_scopes = BTreeSet::from(["operator.read".to_owned()]);
    let decision = registry
        .evaluate_connect(
            "status-device",
            "status-cli",
            "status-public-key",
            "operator",
            &requested_scopes,
            None,
        )
        .expect("evaluate connect");
    let pairing_request_id = match decision {
        loong_daemon::mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
            request,
            ..
        } => request.pairing_request_id,
        other => panic!("expected pending pairing request, got {other:?}"),
    };

    let resolved = registry
        .resolve_request(pairing_request_id.as_str(), true)
        .expect("resolve pairing request")
        .expect("resolved pairing request");
    assert!(
        resolved.device_token.is_some(),
        "approved pairing should return a device token"
    );
}

async fn wait_for_gateway_control_surface(runtime_dir: &Path) {
    for _ in 0..STATUS_GATEWAY_WAIT_ATTEMPTS {
        if let Some(status) = load_gateway_owner_status(runtime_dir) {
            if status.phase == "failed" {
                let error_message = status
                    .last_error
                    .unwrap_or_else(|| "unknown gateway owner failure".to_owned());
                panic!("gateway owner failed before control surface binding: {error_message}");
            }

            if status.running
                && status.bind_address.is_some()
                && status.port.is_some()
                && status.token_path.is_some()
            {
                return;
            }
        }

        sleep(STATUS_GATEWAY_WAIT_INTERVAL).await;
    }

    panic!("timed out waiting for gateway control surface binding");
}

fn run_status_cli_process(
    config_path: &Path,
    home_root: &Path,
    args: &[&str],
    context: &str,
) -> std::process::Output {
    let home_root_text = home_root.to_str().expect("home root should be valid utf-8");
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");

    Command::new(env!("CARGO_BIN_EXE_loong"))
        .arg("status")
        .arg("--config")
        .arg(config_path_text)
        .args(args)
        .env("LOONG_HOME", home_root_text)
        .output()
        .expect(context)
}

fn run_doctor_cli_process(
    config_path: &Path,
    home_root: &Path,
    args: &[&str],
    context: &str,
) -> std::process::Output {
    let home_root_text = home_root.to_str().expect("home root should be valid utf-8");
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");

    Command::new(env!("CARGO_BIN_EXE_loong"))
        .arg("doctor")
        .arg("--config")
        .arg(config_path_text)
        .args(args)
        .env("LOONG_HOME", home_root_text)
        .output()
        .expect(context)
}

#[test]
fn cli_status_help_mentions_operator_runtime_summary() {
    let help = render_cli_help(["status"]);

    assert!(
        help.contains("operator-readable runtime summary"),
        "status help should explain the aggregated operator surface: {help}"
    );
    assert!(
        help.contains("--json"),
        "status help should surface machine-readable output: {help}"
    );
}

#[test]
fn cli_status_parse_accepts_config_and_json_flags() {
    let cli = try_parse_cli(["loong", "status", "--config", "/tmp/loong.toml", "--json"])
        .expect("status CLI should parse");

    let command = cli.command.expect("CLI should parse a subcommand");
    let Commands::Status { config, json } = command else {
        panic!("unexpected CLI parse result: {command:?}");
    };

    assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
    assert!(json);
}

#[test]
fn status_cli_json_rolls_up_gateway_acp_and_work_unit_sections() {
    let root = unique_temp_dir("loong-status-cli-json");
    let home_root = root.join("home");
    fs::create_dir_all(&home_root).expect("create home root");
    let config_path = write_status_config(
        &root,
        true,
        mvp::config::ProviderToolSchemaModeConfig::EnabledWithDowngrade,
    );
    let output =
        run_status_cli_process(&config_path, &home_root, &["--json"], "run status CLI json");

    if !output.status.success() {
        let stdout = render_output(&output.stdout);
        let stderr = render_output(&output.stderr);
        panic!(
            "status CLI json should succeed: status={:?}\nstdout={stdout}\nstderr={stderr}",
            output.status.code()
        );
    }

    let stdout = render_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout).expect("decode status json");

    assert_eq!(payload["schema"]["version"], 3);
    assert_eq!(payload["schema"]["surface"], "status");
    assert_eq!(payload["schema"]["purpose"], "operator_runtime_summary");
    assert_eq!(payload["gateway"]["owner"]["phase"], "stopped");
    assert_eq!(
        payload["gateway"]["runtime"]["tool_calling"]["availability"],
        "ready"
    );
    assert_eq!(
        payload["gateway"]["runtime"]["tool_calling"]["structured_tool_schema_enabled"],
        true
    );
    assert_eq!(payload["gateway"]["nodes"]["paired_device_count"], 0);
    assert_eq!(payload["gateway"]["nodes"]["managed_bridge_count"], 0);
    assert_eq!(payload["gateway"]["nodes"]["total_count"], 0);
    assert_eq!(payload["acp"]["enabled"], true);
    let acp_availability = payload["acp"]["availability"]
        .as_str()
        .expect("acp availability string");
    assert!(
        matches!(acp_availability, "available" | "unavailable"),
        "unexpected acp availability: {payload:#?}"
    );
    if acp_availability == "available" {
        assert!(payload["acp"]["observability"].is_object());
    } else {
        assert!(payload["acp"]["error"].is_string());
    }

    let work_units_availability = payload["work_units"]["availability"]
        .as_str()
        .expect("work-unit availability string");
    assert!(
        matches!(work_units_availability, "available" | "unavailable"),
        "unexpected work-unit availability: {payload:#?}"
    );
    if work_units_availability == "available" {
        assert_eq!(payload["work_units"]["health"]["total_count"], 0);
    } else {
        assert!(payload["work_units"]["error"].is_string());
    }
    assert!(
        payload["deep_dive_actions"]
            .as_array()
            .map(|actions| actions.len() >= 4)
            .unwrap_or(false),
        "status JSON should include typed drill-down actions: {payload:#?}"
    );
    assert!(
        payload["recipes"]
            .as_array()
            .map(|recipes| recipes.len() >= 4)
            .unwrap_or(false),
        "status JSON should retain the command-only drill-down alias: {payload:#?}"
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test(flavor = "current_thread")]
async fn status_cli_prefers_live_gateway_operator_summary_for_matching_config() {
    let root = unique_temp_dir("loong-status-cli-live-gateway");
    let home_root = root.join("home");
    fs::create_dir_all(&home_root).expect("create home root");
    let _env = MigrationEnvironmentGuard::set(&[(
        "LOONG_HOME",
        Some(home_root.to_str().expect("home root should be valid utf-8")),
    )]);
    let config_path = write_status_config(
        &root,
        false,
        mvp::config::ProviderToolSchemaModeConfig::EnabledWithDowngrade,
    );
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8")
        .to_owned();
    let loaded_config = load_status_config_fixture(config_path.as_path());
    seed_approved_pairing_device(&loaded_config.config);

    let runtime_dir = default_gateway_runtime_state_dir();
    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new({
            let config_path = config_path.clone();
            move |_| Ok(load_status_config_fixture(config_path.as_path()))
        }),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("status CLI gateway test should not start the concurrent CLI host")
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
            None,
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_for_gateway_control_surface(runtime_dir.as_path()).await;

    let status =
        loong_daemon::status_cli::collect_status_cli_read_model(Some(config_path_text.as_str()))
            .await
            .expect("collect status CLI read model");

    assert_eq!(status.gateway.owner.phase, "running");
    assert_eq!(status.gateway.owner.config_path, config_path_text);
    assert_eq!(status.gateway.pairing.approved_device_count, 1);
    assert_eq!(status.gateway.nodes.paired_device_count, 1);
    assert_eq!(status.gateway.nodes.total_count, 1);

    request_gateway_stop(runtime_dir.as_path()).expect("request gateway stop");
    let supervisor = timeout(STATUS_GATEWAY_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop")
        .expect("join gateway run")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn doctor_cli_json_includes_schema_for_machine_readable_automation() {
    let root = unique_temp_dir("loong-doctor-cli-json");
    let home_root = root.join("home");
    fs::create_dir_all(&home_root).expect("create home root");
    let config_path = write_status_config(
        &root,
        false,
        mvp::config::ProviderToolSchemaModeConfig::EnabledWithDowngrade,
    );
    let output = run_doctor_cli_process(
        &config_path,
        &home_root,
        &["--json", "--skip-model-probe"],
        "run doctor CLI json",
    );

    if !output.status.success() {
        let stdout = render_output(&output.stdout);
        let stderr = render_output(&output.stderr);
        panic!(
            "doctor CLI json should succeed: status={:?}\nstdout={stdout}\nstderr={stderr}",
            output.status.code()
        );
    }

    let stdout = render_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout).expect("decode doctor json");

    assert_eq!(payload["schema"]["version"], 1);
    assert_eq!(payload["schema"]["surface"], "doctor");
    assert_eq!(payload["schema"]["purpose"], "runtime_health_diagnostics");
    assert!(
        payload["checks"]
            .as_array()
            .is_some_and(|checks| !checks.is_empty()),
        "doctor JSON should include machine-readable checks: {payload:#?}"
    );
    assert!(
        payload["next_steps"].is_array(),
        "doctor JSON should keep next steps machine-readable: {payload:#?}"
    );
    assert!(
        payload["next_step_actions"].is_array(),
        "doctor JSON should include typed next-step actions for direct operator handoff surfaces: {payload:#?}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn status_cli_text_surfaces_section_summaries_and_drill_down_actions() {
    let root = unique_temp_dir("loong-status-cli-text");
    let home_root = root.join("home");
    fs::create_dir_all(&home_root).expect("create home root");
    let config_path = write_status_config(
        &root,
        false,
        mvp::config::ProviderToolSchemaModeConfig::Disabled,
    );
    let output = run_status_cli_process(&config_path, &home_root, &[], "run status CLI text");

    if !output.status.success() {
        let stdout = render_output(&output.stdout);
        let stderr = render_output(&output.stderr);
        panic!(
            "status CLI text should succeed: status={:?}\nstdout={stdout}\nstderr={stderr}",
            output.status.code()
        );
    }

    let stdout = render_output(&output.stdout);

    assert!(stdout.contains("operator runtime summary"));
    assert!(stdout.contains("start here"));
    assert!(stdout.contains("runtime posture"));
    assert!(stdout.contains("[WARN] tool calling"));
    assert!(stdout.contains("enabled=false · availability=disabled"));
    assert!(stdout.contains("inspect deeper"));

    fs::remove_dir_all(&root).ok();
}
