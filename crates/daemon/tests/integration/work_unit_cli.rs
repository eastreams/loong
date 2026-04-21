use super::*;
use loong_daemon::work_unit_cli as daemon_work_unit_cli;
use loong_daemon::work_unit_cli as work_unit_runtime;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

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

fn write_work_unit_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let sqlite_path = root.join("memory.sqlite3");
    let mut config = mvp::config::LoongConfig::default();
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let config_path = root.join("loong.toml");
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

fn load_work_unit_repository(config_path: &Path) -> mvp::work::repository::WorkUnitRepository {
    let (_, config) = mvp::config::load(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("load work-unit config");
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    mvp::work::repository::WorkUnitRepository::new(&memory_config).expect("work unit repository")
}

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn run_work_unit_cli_process(args: Vec<String>, context: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_loong"))
        .args(args)
        .output()
        .expect(context);
    if output.status.success() {
        return;
    }

    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    panic!(
        "{context}: status={:?}\nstdout={stdout}\nstderr={stderr}",
        output.status.code()
    );
}

#[test]
fn cli_work_unit_help_mentions_durable_runtime_commands() {
    let help = render_cli_help(["work-unit"]);

    assert!(
        help.contains("Create one durable work unit record"),
        "work-unit help should describe the create flow: {help}"
    );
    assert!(
        help.contains("claim"),
        "work-unit help should expose lease claiming: {help}"
    );
    assert!(
        help.contains("recover"),
        "work-unit help should expose lease recovery: {help}"
    );
    assert!(
        help.contains("assign"),
        "work-unit help should expose orchestration assignment: {help}"
    );
    assert!(
        help.contains("update"),
        "work-unit help should expose general work-unit mutation: {help}"
    );
}

#[test]
fn cli_work_unit_parse_accepts_full_complete_command_shape() {
    let cli = try_parse_cli([
        "loong",
        "work-unit",
        "complete",
        "--config",
        "/tmp/loong.toml",
        "--id",
        "wu-demo",
        "--owner",
        "worker-a",
        "--disposition",
        "retry_pending",
        "--actor",
        "scheduler",
        "--now-ms",
        "1500",
        "--next-run-at-ms",
        "2500",
        "--result-payload-json",
        "{\"summary\":\"retry\"}",
        "--error",
        "transient",
        "--json",
    ])
    .expect("work-unit complete CLI should parse");

    let command = cli.command.expect("CLI should parse a subcommand");
    let Commands::Runtime {
        command: loong_daemon::runtime_cli::RuntimeCommands::WorkUnit { command },
    } = command
    else {
        panic!("unexpected CLI parse result: {command:?}");
    };
    let work_unit_runtime::WorkUnitCommands::Complete(options) = command else {
        panic!("unexpected work-unit subcommand parse result: {command:?}");
    };

    assert_eq!(options.id, "wu-demo");
    assert_eq!(options.owner, "worker-a");
    assert_eq!(
        options.disposition,
        work_unit_runtime::WorkUnitDispositionArg::RetryPending
    );
    assert_eq!(options.actor.as_deref(), Some("scheduler"));
    assert_eq!(options.now_ms, Some(1500));
    assert_eq!(options.next_run_at_ms, Some(2500));
    assert!(options.json);
}

#[test]
fn cli_work_unit_parse_accepts_update_command_shape() {
    let cli = try_parse_cli([
        "loong",
        "work-unit",
        "update",
        "--config",
        "/tmp/loong.toml",
        "--id",
        "wu-demo",
        "--title",
        "Refined title",
        "--description",
        "Refined description",
        "--status",
        "waiting_review",
        "--priority",
        "critical",
        "--next-run-at-ms",
        "3333",
        "--blocking-reason",
        "awaiting review",
        "--actor",
        "planner",
        "--now-ms",
        "2222",
        "--json",
    ])
    .expect("work-unit update CLI should parse");

    let command = cli.command.expect("CLI should parse a subcommand");
    let Commands::Runtime {
        command: loong_daemon::runtime_cli::RuntimeCommands::WorkUnit { command },
    } = command
    else {
        panic!("unexpected CLI parse result: {command:?}");
    };
    let work_unit_runtime::WorkUnitCommands::Update(options) = command else {
        panic!("unexpected work-unit update parse result: {command:?}");
    };

    assert_eq!(options.id, "wu-demo");
    assert_eq!(options.title.as_deref(), Some("Refined title"));
    assert_eq!(options.description.as_deref(), Some("Refined description"));
    assert_eq!(
        options.status,
        Some(work_unit_runtime::WorkUnitStatusArg::WaitingReview)
    );
    assert_eq!(
        options.priority,
        Some(work_unit_runtime::WorkUnitPriorityArg::Critical)
    );
    assert_eq!(options.next_run_at_ms, Some(3333));
    assert_eq!(options.blocking_reason.as_deref(), Some("awaiting review"));
    assert_eq!(options.actor.as_deref(), Some("planner"));
    assert_eq!(options.now_ms, Some(2222));
    assert!(options.json);
}

#[test]
fn work_unit_cli_create_claim_complete_and_archive_round_trip() {
    let _env_lock = super::lock_daemon_test_environment();
    let root = unique_temp_dir("loong-work-unit-cli");
    let config_path = write_work_unit_config(&root);
    let config_path_string = config_path.display().to_string();
    let scenario_id = root
        .file_name()
        .expect("temp dir should have file name")
        .to_string_lossy()
        .into_owned();
    let work_unit_id = format!("wu-cli-{scenario_id}");

    run_work_unit_cli_process(
        vec![
            "work-unit".to_owned(),
            "create".to_owned(),
            "--config".to_owned(),
            config_path_string.clone(),
            "--id".to_owned(),
            work_unit_id.clone(),
            "--kind".to_owned(),
            "feature".to_owned(),
            "--title".to_owned(),
            "Durable runtime slice".to_owned(),
            "--description".to_owned(),
            "Create the first work-unit runtime slice".to_owned(),
            "--status".to_owned(),
            "ready".to_owned(),
            "--priority".to_owned(),
            "high".to_owned(),
            "--max-attempts".to_owned(),
            "3".to_owned(),
            "--initial-backoff-ms".to_owned(),
            "1000".to_owned(),
            "--max-backoff-ms".to_owned(),
            "8000".to_owned(),
            "--next-run-at-ms".to_owned(),
            "1000".to_owned(),
            "--actor".to_owned(),
            "operator".to_owned(),
            "--source-kind".to_owned(),
            "discord".to_owned(),
            "--project-id".to_owned(),
            "loong-ai/server".to_owned(),
            "--channel-id".to_owned(),
            "feature".to_owned(),
            "--thread-id".to_owned(),
            "thread-1".to_owned(),
            "--message-id".to_owned(),
            "message-1".to_owned(),
            "--external-ref".to_owned(),
            "feature-thread".to_owned(),
            "--json".to_owned(),
        ],
        "create work unit via CLI subprocess",
    );

    run_work_unit_cli_process(
        vec![
            "work-unit".to_owned(),
            "assign".to_owned(),
            "--config".to_owned(),
            config_path_string.clone(),
            "--id".to_owned(),
            work_unit_id.clone(),
            "--assigned-to".to_owned(),
            "designer".to_owned(),
            "--actor".to_owned(),
            "operator".to_owned(),
            "--now-ms".to_owned(),
            "1050".to_owned(),
            "--json".to_owned(),
        ],
        "assign work unit via CLI subprocess",
    );

    run_work_unit_cli_process(
        vec![
            "work-unit".to_owned(),
            "update".to_owned(),
            "--config".to_owned(),
            config_path_string.clone(),
            "--id".to_owned(),
            work_unit_id.clone(),
            "--title".to_owned(),
            "Durable runtime slice v2".to_owned(),
            "--description".to_owned(),
            "Refine the orchestration-ready slice".to_owned(),
            "--status".to_owned(),
            "waiting_review".to_owned(),
            "--priority".to_owned(),
            "critical".to_owned(),
            "--next-run-at-ms".to_owned(),
            "1060".to_owned(),
            "--blocking-reason".to_owned(),
            "needs review before execution".to_owned(),
            "--actor".to_owned(),
            "planner".to_owned(),
            "--now-ms".to_owned(),
            "1055".to_owned(),
            "--json".to_owned(),
        ],
        "update work unit via CLI subprocess",
    );

    run_work_unit_cli_process(
        vec![
            "work-unit".to_owned(),
            "note".to_owned(),
            "--config".to_owned(),
            config_path_string.clone(),
            "--id".to_owned(),
            work_unit_id.clone(),
            "--actor".to_owned(),
            "operator".to_owned(),
            "--note".to_owned(),
            "waiting on prerequisite".to_owned(),
            "--now-ms".to_owned(),
            "1090".to_owned(),
            "--json".to_owned(),
        ],
        "append note via CLI subprocess",
    );

    run_work_unit_cli_process(
        vec![
            "work-unit".to_owned(),
            "claim".to_owned(),
            "--config".to_owned(),
            config_path_string.clone(),
            "--owner".to_owned(),
            "worker-a".to_owned(),
            "--ttl-ms".to_owned(),
            "5000".to_owned(),
            "--actor".to_owned(),
            "scheduler".to_owned(),
            "--now-ms".to_owned(),
            "1000".to_owned(),
            "--json".to_owned(),
        ],
        "claim work unit via CLI subprocess",
    );

    let repository = load_work_unit_repository(&config_path);
    let updated_snapshot = repository
        .load_work_unit_snapshot(work_unit_id.as_str())
        .expect("load updated snapshot")
        .expect("updated snapshot");
    assert_eq!(updated_snapshot.work_unit.title, "Durable runtime slice v2");
    assert_eq!(
        updated_snapshot.work_unit.status,
        loong_contracts::WorkUnitStatus::WaitingReview
    );
    assert_eq!(
        updated_snapshot.work_unit.blocking_reason.as_deref(),
        Some("needs review before execution")
    );
    let note_events = repository
        .list_work_unit_events(work_unit_id.as_str(), 20)
        .expect("load note events");
    assert!(
        note_events
            .iter()
            .any(|event| event.event_kind == "work_unit_note_added"),
        "expected note event in work-unit ledger"
    );

    run_work_unit_cli_process(
        vec![
            "work-unit".to_owned(),
            "update".to_owned(),
            "--config".to_owned(),
            config_path_string,
            "--id".to_owned(),
            work_unit_id.clone(),
            "--status".to_owned(),
            "ready".to_owned(),
            "--next-run-at-ms".to_owned(),
            "1100".to_owned(),
            "--clear-blocking-reason".to_owned(),
            "--actor".to_owned(),
            "planner".to_owned(),
            "--now-ms".to_owned(),
            "1095".to_owned(),
            "--json".to_owned(),
        ],
        "clear review block via CLI subprocess",
    );

    let leased_snapshot = repository
        .acquire_next_ready_lease(mvp::work::repository::AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: Some("scheduler".to_owned()),
            now_ms: Some(1_100),
        })
        .expect("lease ready work unit")
        .expect("leased work unit snapshot");
    assert_eq!(leased_snapshot.work_unit.work_unit_id, work_unit_id);
    assert_eq!(
        leased_snapshot
            .lease
            .as_ref()
            .map(|lease| lease.owner.as_str()),
        Some("worker-a")
    );

    let running_snapshot = repository
        .mark_leased_running(mvp::work::repository::StartWorkUnitLeaseRequest {
            work_unit_id: work_unit_id.clone(),
            owner: "worker-a".to_owned(),
            actor: Some("worker-a".to_owned()),
            now_ms: Some(1_100),
        })
        .expect("start leased work unit")
        .expect("running work unit snapshot");
    assert_eq!(
        running_snapshot.work_unit.status,
        loong_contracts::WorkUnitStatus::Running
    );

    let completed_snapshot = repository
        .complete_work_unit(mvp::work::repository::CompleteWorkUnitRequest {
            work_unit_id: work_unit_id.clone(),
            owner: "worker-a".to_owned(),
            disposition: mvp::work::repository::WorkUnitCompletionDisposition::Completed,
            actor: Some("worker-a".to_owned()),
            now_ms: Some(1_200),
            next_run_at_ms: None,
            result_payload_json: Some(json!({"summary": "done"})),
            error: None,
        })
        .expect("complete running work unit")
        .expect("completed work unit snapshot");
    assert_eq!(
        completed_snapshot.work_unit.status,
        loong_contracts::WorkUnitStatus::Completed
    );

    let archived_snapshot = repository
        .archive_work_unit(mvp::work::repository::ArchiveWorkUnitRequest {
            work_unit_id: work_unit_id.clone(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_300),
        })
        .expect("archive completed work unit")
        .expect("archived work unit snapshot");
    assert_eq!(
        archived_snapshot.work_unit.status,
        loong_contracts::WorkUnitStatus::Archived
    );

    let snapshot = repository
        .load_work_unit_snapshot(work_unit_id.as_str())
        .expect("load work unit snapshot")
        .expect("work unit snapshot");
    let events = repository
        .list_work_unit_events(work_unit_id.as_str(), 20)
        .expect("load work unit events");

    assert_eq!(
        snapshot.work_unit.status,
        loong_contracts::WorkUnitStatus::Archived
    );
    assert_eq!(
        snapshot.work_unit.result_payload_json,
        Some(json!({"summary": "done"}))
    );
    assert_eq!(snapshot.work_unit.assigned_to.as_deref(), Some("designer"));
    assert!(snapshot.lease.is_none());
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == "work_unit_created"),
        "expected create event in work-unit ledger"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == "work_unit_updated"),
        "expected update event in work-unit ledger"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == "work_unit_archived"),
        "expected archive event in work-unit ledger"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == "work_unit_assigned"),
        "expected assignment event in work-unit ledger"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == "work_unit_note_added"),
        "expected note event in work-unit ledger"
    );
}

#[test]
fn work_unit_cli_update_text_output_uses_snake_case_status_labels() {
    let _env_lock = super::lock_daemon_test_environment();
    let root = unique_temp_dir("loong-work-unit-cli-text");
    let config_path = write_work_unit_config(&root);
    let repository = load_work_unit_repository(&config_path);
    let retry_policy = loong_contracts::WorkUnitRetryPolicy {
        max_attempts: 2,
        initial_backoff_ms: 1_000,
        max_backoff_ms: 8_000,
    };
    let source_ref = loong_contracts::WorkUnitSourceRef {
        source_kind: loong_contracts::WorkSourceKind::Manual,
        project_id: None,
        channel_id: None,
        thread_id: None,
        message_id: None,
        external_ref: None,
        source_url: None,
    };
    let new_work_unit = mvp::work::repository::NewWorkUnitRecord {
        work_unit_id: Some("wu-text".to_owned()),
        kind: loong_contracts::WorkUnitKind::Feature,
        title: "text renderer".to_owned(),
        description: "verify non-json output".to_owned(),
        source_ref,
        status: loong_contracts::WorkUnitStatus::Ready,
        priority: loong_contracts::WorkUnitPriority::Normal,
        retry_policy,
        parent_work_unit_id: None,
        next_run_at_ms: Some(1_000),
    };
    repository
        .create_work_unit(new_work_unit, Some("operator"))
        .expect("create work unit fixture");

    let config_path_string = config_path.display().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_loong"))
        .args([
            "work-unit",
            "update",
            "--config",
            config_path_string.as_str(),
            "--id",
            "wu-text",
            "--status",
            "waiting_review",
            "--actor",
            "planner",
            "--now-ms",
            "1234",
        ])
        .output()
        .expect("run work-unit update text command");

    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);

    assert!(
        output.status.success(),
        "work-unit update text output should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains("status=waiting_review"),
        "text output should preserve snake_case status labels, stdout={stdout:?}"
    );
}
