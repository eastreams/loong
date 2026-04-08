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
fn cli_work_unit_parse_accepts_update_command_shape() {
    let cli = try_parse_cli([
        "loongclaw",
        "work-unit",
        "update",
        "--config",
        "/tmp/loongclaw.toml",
        "--id",
        "wu-demo",
        "--title",
        "Refined title",
        "--description",
        "Refined description",
        "--status",
        "waiting-review",
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
    let Commands::WorkUnit { command } = command else {
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
    let root = unique_temp_dir("loongclaw-work-unit-cli");
    let config_path = write_work_unit_config(&root);
    let config_path_string = config_path.display().to_string();

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Create(
        work_unit_runtime::WorkUnitCreateCommandOptions {
            config: Some(config_path_string.clone()),
            id: Some("wu-blocker".to_owned()),
            kind: work_unit_runtime::WorkUnitKindArg::Ops,
            title: "Prerequisite".to_owned(),
            description: "Finish prerequisite work".to_owned(),
            status: work_unit_runtime::WorkUnitStatusArg::Ready,
            priority: work_unit_runtime::WorkUnitPriorityArg::Low,
            max_attempts: 1,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 1_000,
            next_run_at_ms: Some(1_000),
            actor: Some("operator".to_owned()),
            source_kind: work_unit_runtime::WorkSourceKindArg::Manual,
            project_id: None,
            channel_id: None,
            thread_id: None,
            message_id: None,
            external_ref: None,
            source_url: None,
            parent_work_unit_id: None,
            json: true,
        },
    ))
    .expect("create blocker work unit via CLI");

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

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Assign(
        work_unit_runtime::WorkUnitAssignCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-cli".to_owned(),
            assigned_to: Some("designer".to_owned()),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_050),
            json: true,
        },
    ))
    .expect("assign work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Update(
        work_unit_runtime::WorkUnitUpdateCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-cli".to_owned(),
            title: Some("Durable runtime slice v2".to_owned()),
            description: Some("Refine the orchestration-ready slice".to_owned()),
            status: Some(work_unit_runtime::WorkUnitStatusArg::WaitingReview),
            priority: Some(work_unit_runtime::WorkUnitPriorityArg::Critical),
            next_run_at_ms: Some(1_060),
            blocking_reason: Some("needs review before execution".to_owned()),
            clear_blocking_reason: false,
            actor: Some("planner".to_owned()),
            now_ms: Some(1_055),
            json: true,
        },
    ))
    .expect("update work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Depend(
        work_unit_runtime::WorkUnitDependCommandOptions {
            config: Some(config_path_string.clone()),
            blocking_id: "wu-blocker".to_owned(),
            blocked_id: "wu-cli".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_075),
            json: true,
        },
    ))
    .expect("add dependency via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Note(
        work_unit_runtime::WorkUnitNoteCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-cli".to_owned(),
            actor: Some("operator".to_owned()),
            note: "waiting on prerequisite".to_owned(),
            now_ms: Some(1_090),
            json: true,
        },
    ))
    .expect("append note via CLI");

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

    let repository = load_work_unit_repository(&config_path);
    let updated_snapshot = repository
        .load_work_unit_snapshot("wu-cli")
        .expect("load updated snapshot")
        .expect("updated snapshot");
    assert_eq!(updated_snapshot.work_unit.title, "Durable runtime slice v2");
    assert_eq!(
        updated_snapshot.work_unit.status,
        loongclaw_contracts::WorkUnitStatus::WaitingReview
    );
    assert_eq!(
        updated_snapshot.work_unit.blocking_reason.as_deref(),
        Some("needs review before execution")
    );

    let blocker_snapshot = repository
        .load_work_unit_snapshot("wu-blocker")
        .expect("load blocker snapshot")
        .expect("blocker snapshot");
    assert_eq!(
        blocker_snapshot
            .lease
            .as_ref()
            .map(|lease| lease.owner.as_str()),
        Some("worker-a")
    );

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Complete(
        work_unit_runtime::WorkUnitCompleteCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-blocker".to_owned(),
            owner: "worker-a".to_owned(),
            disposition: work_unit_runtime::WorkUnitDispositionArg::Completed,
            actor: Some("worker-a".to_owned()),
            now_ms: Some(1_050),
            next_run_at_ms: None,
            result_payload_json: None,
            error: None,
            json: true,
        },
    ))
    .expect("complete blocker work unit via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Undepend(
        work_unit_runtime::WorkUnitUndependCommandOptions {
            config: Some(config_path_string.clone()),
            blocking_id: "wu-blocker".to_owned(),
            blocked_id: "wu-cli".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_100),
            json: true,
        },
    ))
    .expect("remove dependency via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Update(
        work_unit_runtime::WorkUnitUpdateCommandOptions {
            config: Some(config_path_string.clone()),
            id: "wu-cli".to_owned(),
            title: None,
            description: None,
            status: Some(work_unit_runtime::WorkUnitStatusArg::Ready),
            priority: None,
            next_run_at_ms: Some(1_100),
            blocking_reason: None,
            clear_blocking_reason: true,
            actor: Some("planner".to_owned()),
            now_ms: Some(1_095),
            json: true,
        },
    ))
    .expect("clear review block via CLI");

    work_unit_runtime::run_work_unit_cli(work_unit_runtime::WorkUnitCommands::Claim(
        work_unit_runtime::WorkUnitClaimCommandOptions {
            config: Some(config_path_string.clone()),
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: Some("scheduler".to_owned()),
            now_ms: Some(1_100),
            json: true,
        },
    ))
    .expect("claim unblocked work unit via CLI");

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
    assert_eq!(snapshot.work_unit.assigned_to.as_deref(), Some("designer"));
    assert!(snapshot.work_unit.blocked_by_work_unit_ids.is_empty());
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
