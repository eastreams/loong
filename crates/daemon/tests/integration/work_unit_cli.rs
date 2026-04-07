use super::*;
use loongclaw_daemon::work_unit_cli as daemon_work_unit_cli;
use loongclaw_daemon::work_unit_cli as work_unit_runtime;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn write_work_unit_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let sqlite_path = root.join("memory.sqlite3");
    let mut config = mvp::config::LoongClawConfig::default();
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let config_path = root.join("loongclaw.toml");
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
}

#[test]
fn cli_work_unit_parse_accepts_full_complete_command_shape() {
    let cli = try_parse_cli([
        "loongclaw",
        "work-unit",
        "complete",
        "--config",
        "/tmp/loongclaw.toml",
        "--id",
        "wu-demo",
        "--owner",
        "worker-a",
        "--disposition",
        "retry-pending",
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
    let Commands::WorkUnit { command } = command else {
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
fn work_unit_cli_create_claim_complete_and_archive_round_trip() {
    let root = unique_temp_dir("loongclaw-work-unit-cli");
    let config_path = write_work_unit_config(&root);
    let config_path_string = config_path.display().to_string();

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Create(
        work_unit_runtime::WorkUnitCreateCommandOptions {
            config: Some(config_path_string.clone()),
            id: Some("wu-cli".to_owned()),
            kind: work_unit_runtime::WorkUnitKindArg::Feature,
            title: "Durable runtime slice".to_owned(),
            description: "Create the first work-unit runtime slice".to_owned(),
            status: work_unit_runtime::WorkUnitStatusArg::Ready,
            priority: work_unit_runtime::WorkUnitPriorityArg::High,
            max_attempts: 3,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 8_000,
            next_run_at_ms: Some(1_000),
            actor: Some("operator".to_owned()),
            source_kind: work_unit_runtime::WorkSourceKindArg::Discord,
            project_id: Some("loongclaw-ai/server".to_owned()),
            channel_id: Some("feature".to_owned()),
            thread_id: Some("thread-1".to_owned()),
            message_id: Some("message-1".to_owned()),
            external_ref: Some("feature-thread".to_owned()),
            source_url: None,
            parent_work_unit_id: None,
            json: true,
        },
    ))
    .expect("create work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Claim(
        work_unit_runtime::WorkUnitClaimCommandOptions {
            config: Some(config_path_string.clone()),
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: Some("scheduler".to_owned()),
            now_ms: Some(1_000),
            json: true,
        },
    ))
    .expect("claim work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Start(
        work_unit_runtime::WorkUnitStartCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-cli".to_owned(),
            owner: "worker-a".to_owned(),
            actor: Some("worker-a".to_owned()),
            now_ms: Some(1_100),
            json: true,
        },
    ))
    .expect("start work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Complete(
        work_unit_runtime::WorkUnitCompleteCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-cli".to_owned(),
            owner: "worker-a".to_owned(),
            disposition: work_unit_runtime::WorkUnitDispositionArg::Completed,
            actor: Some("worker-a".to_owned()),
            now_ms: Some(1_200),
            next_run_at_ms: None,
            result_payload_json: Some("{\"summary\":\"done\"}".to_owned()),
            error: None,
            json: true,
        },
    ))
    .expect("complete work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Archive(
        work_unit_runtime::WorkUnitArchiveCommandOptions {
            config: Some(config_path_string),
            id: "wu-cli".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_300),
            json: true,
        },
    ))
    .expect("archive work unit via CLI");

    let repository = load_work_unit_repository(&config_path);
    let snapshot = repository
        .load_work_unit_snapshot("wu-cli")
        .expect("load work unit snapshot")
        .expect("work unit snapshot");
    let events = repository
        .list_work_unit_events("wu-cli", 20)
        .expect("load work unit events");

    assert_eq!(
        snapshot.work_unit.status,
        loongclaw_contracts::WorkUnitStatus::Archived
    );
    assert_eq!(
        snapshot.work_unit.result_payload_json,
        Some(json!({"summary": "done"}))
    );
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
            .any(|event| event.event_kind == "work_unit_archived"),
        "expected archive event in work-unit ledger"
    );
}
