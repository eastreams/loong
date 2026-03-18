#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::Value;
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

fn write_runtime_experiment_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongClawConfig::default();
    config.tools.file_root = Some(root.display().to_string());
    config.tools.browser.enabled = true;
    config.tools.web.enabled = true;
    config.acp.enabled = true;
    config.acp.dispatch.enabled = true;
    config.acp.default_agent = Some("planner".to_owned());
    config.acp.allowed_agents = vec!["planner".to_owned(), "codex".to_owned()];
    config.providers.insert(
        "openai-main".to_owned(),
        mvp::config::ProviderProfileConfig {
            default_for_kind: false,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Openai,
                model: "gpt-4.1-mini".to_owned(),
                ..Default::default()
            },
        },
    );
    config.set_active_provider_profile(
        "deepseek-lab",
        mvp::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: mvp::config::ProviderConfig {
                kind: mvp::config::ProviderKind::Deepseek,
                model: "deepseek-chat".to_owned(),
                api_key: Some("demo-token".to_owned()),
                ..Default::default()
            },
        },
    );

    let config_path = root.join("loongclaw.toml");
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

fn write_snapshot_artifact(
    root: &Path,
    config_path: &Path,
    relative: &str,
    metadata: loongclaw_daemon::RuntimeSnapshotArtifactMetadata,
) -> (PathBuf, Value) {
    let snapshot = collect_runtime_snapshot_cli_state(Some(
        config_path.to_str().expect("config path should be utf-8"),
    ))
    .expect("collect runtime snapshot");
    let payload =
        loongclaw_daemon::build_runtime_snapshot_artifact_json_payload(&snapshot, &metadata)
            .expect("build runtime snapshot artifact");
    let artifact_path = root.join(relative);
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent).expect("create artifact directory");
    }
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&payload).expect("encode snapshot artifact"),
    )
    .expect("write snapshot artifact");
    (artifact_path, payload)
}

fn snapshot_id_from_payload(payload: &Value) -> String {
    payload
        .get("lineage")
        .and_then(|lineage| lineage.get("snapshot_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("snapshot payload should include lineage.snapshot_id")
}

fn canonical_display_path(path: &Path) -> String {
    fs::canonicalize(path)
        .expect("canonicalize fixture path")
        .display()
        .to_string()
}

fn rewrite_json_file(path: &Path, mutate: impl FnOnce(&mut Value)) {
    let raw = fs::read_to_string(path).expect("read json fixture");
    let mut payload = serde_json::from_str::<Value>(&raw).expect("decode json fixture");
    mutate(&mut payload);
    fs::write(
        path,
        serde_json::to_string_pretty(&payload).expect("encode json fixture"),
    )
    .expect("write json fixture");
}

fn rewrite_runtime_experiment_compare_config(config_path: &Path) {
    let (_, mut config) = mvp::config::load(Some(
        config_path
            .to_str()
            .expect("config path should be valid utf-8"),
    ))
    .expect("load config fixture");
    let openai = config
        .providers
        .get("openai-main")
        .cloned()
        .expect("openai-main provider should exist");
    config.set_active_provider_profile("openai-main", openai);
    config.tools.browser.enabled = false;
    config.tools.web.enabled = false;
    config.acp.dispatch.enabled = false;
    config.acp.default_agent = Some("codex".to_owned());
    config.acp.allowed_agents = vec!["codex".to_owned()];
    mvp::config::write(
        Some(
            config_path
                .to_str()
                .expect("config path should be valid utf-8"),
        ),
        &config,
        true,
    )
    .expect("rewrite config fixture");
}

fn start_runtime_experiment(
    root: &Path,
    snapshot_path: &Path,
    experiment_id: Option<&str>,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let run_path = root.join("artifacts/runtime-experiment.json");
    let run = loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_start_command(
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStartCommandOptions {
            snapshot: snapshot_path.display().to_string(),
            output: run_path.display().to_string(),
            mutation_summary: "enable browser preview skill".to_owned(),
            experiment_id: experiment_id.map(str::to_owned),
            label: Some("browser-preview-a".to_owned()),
            tag: vec!["browser".to_owned(), "preview".to_owned()],
            json: false,
        },
    )
    .expect("runtime experiment start should succeed");
    (run_path, run)
}

fn finish_runtime_experiment(
    root: &Path,
    config_path: &Path,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(root, &baseline_snapshot_path, None);
    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );

    let finished =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "task success improved".to_owned(),
                metric: vec!["task_success=1".to_owned(), "token_delta=0".to_owned()],
                warning: vec!["manual verification only".to_owned()],
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("runtime experiment finish should succeed");
    (run_path, finished)
}

fn finish_runtime_experiment_with_compare_delta(
    root: &Path,
    config_path: &Path,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(root, &baseline_snapshot_path, None);

    rewrite_runtime_experiment_compare_config(config_path);

    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );

    let finished =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "provider and tool policy updated".to_owned(),
                metric: vec!["task_success=1".to_owned(), "cost_delta=-0.2".to_owned()],
                warning: vec!["manual verification only".to_owned()],
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("runtime experiment finish should succeed");

    (
        run_path,
        baseline_snapshot_path,
        result_snapshot_path,
        finished,
    )
}

fn finish_runtime_experiment_with_missing_compare_sections(
    root: &Path,
    config_path: &Path,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    rewrite_json_file(&baseline_snapshot_path, |payload| {
        payload["context_engine"]
            .as_object_mut()
            .expect("context_engine should be an object")
            .remove("compaction");
        payload["memory_system"]
            .as_object_mut()
            .expect("memory_system should be an object")
            .remove("policy");
        payload["acp"] = Value::Null;
    });

    let (run_path, _) = start_runtime_experiment(root, &baseline_snapshot_path, None);
    rewrite_runtime_experiment_compare_config(config_path);

    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );
    let finished =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "filled missing runtime sections".to_owned(),
                metric: vec!["task_success=1".to_owned()],
                warning: Vec::new(),
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("runtime experiment finish should succeed");

    (
        run_path,
        baseline_snapshot_path,
        result_snapshot_path,
        finished,
    )
}

#[test]
fn runtime_experiment_start_creates_planned_run_and_inherits_baseline_lineage() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-start");
    let config_path = write_runtime_experiment_config(&root);
    let (snapshot_path, snapshot_payload) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let run_path = root.join("artifacts/runtime-experiment.json");

    let run = loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_start_command(
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStartCommandOptions {
            snapshot: snapshot_path.display().to_string(),
            output: run_path.display().to_string(),
            mutation_summary: "enable browser preview skill".to_owned(),
            experiment_id: None,
            label: Some("browser-preview-a".to_owned()),
            tag: vec!["browser".to_owned(), "preview".to_owned()],
            json: false,
        },
    )
    .expect("runtime experiment start should succeed");

    assert!(
        !run.run_id.is_empty(),
        "run_id should be populated for persisted experiment records"
    );
    assert_eq!(run.experiment_id, "exp-42");
    assert_eq!(
        run.baseline_snapshot.snapshot_id,
        snapshot_id_from_payload(&snapshot_payload)
    );
    assert_eq!(run.baseline_snapshot.label.as_deref(), Some("baseline"));
    assert_eq!(
        run.baseline_snapshot.parent_snapshot_id.as_deref(),
        Some("snapshot-parent")
    );
    assert_eq!(
        run.status,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStatus::Planned
    );
    assert_eq!(
        run.decision,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Undecided
    );
    assert_eq!(run.mutation.summary, "enable browser preview skill");
    assert_eq!(
        run.mutation.tags,
        vec!["browser".to_owned(), "preview".to_owned()]
    );
    assert_eq!(run.result_snapshot, None);
    assert_eq!(run.evaluation, None);
    assert!(
        run_path.exists(),
        "start should persist the experiment-run artifact"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_start_requires_explicit_experiment_id_when_baseline_is_missing_one() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-start-missing-id");
    let config_path = write_runtime_experiment_config(&root);
    let (snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: None,
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let run_path = root.join("artifacts/runtime-experiment.json");

    let error = loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_start_command(
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStartCommandOptions {
            snapshot: snapshot_path.display().to_string(),
            output: run_path.display().to_string(),
            mutation_summary: "enable browser preview skill".to_owned(),
            experiment_id: None,
            label: None,
            tag: Vec::new(),
            json: false,
        },
    )
    .expect_err("missing experiment id should be rejected");

    assert!(error.contains("experiment_id"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_finish_persists_result_metrics_and_warnings() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-finish");
    let config_path = write_runtime_experiment_config(&root);
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &baseline_snapshot_path, None);
    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );

    let finished =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "task success improved".to_owned(),
                metric: vec!["task_success=1".to_owned(), "token_delta=0".to_owned()],
                warning: vec!["manual verification only".to_owned()],
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("runtime experiment finish should succeed");

    assert_eq!(
        finished.status,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStatus::Completed
    );
    assert_eq!(
        finished.decision,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted
    );
    assert!(
        finished.finished_at.is_some(),
        "finish should record a completion timestamp"
    );
    assert_eq!(
        finished
            .result_snapshot
            .as_ref()
            .expect("finish should attach result snapshot")
            .label
            .as_deref(),
        Some("candidate")
    );
    assert_eq!(
        finished
            .evaluation
            .as_ref()
            .expect("finish should attach evaluation")
            .summary,
        "task success improved"
    );
    assert_eq!(
        finished
            .evaluation
            .as_ref()
            .expect("finish should attach evaluation")
            .metrics,
        std::collections::BTreeMap::from([
            ("task_success".to_owned(), 1.0),
            ("token_delta".to_owned(), 0.0),
        ])
    );
    assert_eq!(
        finished
            .evaluation
            .as_ref()
            .expect("finish should attach evaluation")
            .warnings,
        vec!["manual verification only".to_owned()]
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_finish_rejects_conflicting_result_snapshot_experiment_id() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-finish-conflict");
    let config_path = write_runtime_experiment_config(&root);
    let (baseline_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &baseline_snapshot_path, None);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: Some("exp-other".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "task success improved".to_owned(),
                metric: vec!["task_success=1".to_owned()],
                warning: Vec::new(),
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Rejected,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect_err("conflicting result snapshot experiment id must fail");

    assert!(error.contains("experiment_id"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_finish_warns_when_result_snapshot_has_no_experiment_id() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-finish-missing-id");
    let config_path = write_runtime_experiment_config(&root);
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &baseline_snapshot_path, None);
    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: None,
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );

    let finished =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "task success improved".to_owned(),
                metric: vec!["task_success=1".to_owned()],
                warning: Vec::new(),
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Rejected,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("missing result snapshot experiment id should downgrade to a warning");

    assert!(
        finished
            .evaluation
            .as_ref()
            .expect("finish should attach evaluation")
            .warnings
            .iter()
            .any(|warning: &String| warning.contains("missing experiment_id")),
        "finish should record a warning for result snapshots missing experiment_id"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_finish_rejects_mutating_a_finalized_run() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-finish-finalized");
    let config_path = write_runtime_experiment_config(&root);
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &baseline_snapshot_path, None);
    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:30:00Z".to_owned(),
            label: Some("candidate".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );

    let first_finish =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "task success improved".to_owned(),
                metric: vec!["task_success=1".to_owned()],
                warning: Vec::new(),
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("first finish should succeed");
    assert_eq!(
        first_finish.status,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStatus::Completed
    );

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: "task success improved again".to_owned(),
                metric: vec!["task_success=2".to_owned()],
                warning: Vec::new(),
                decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect_err("finalized run should reject further mutation");

    assert!(error.contains("completed"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_show_round_trips_the_persisted_artifact() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-show");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, finished) = finish_runtime_experiment(&root, &config_path);

    let shown = loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_show_command(
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentShowCommandOptions {
            run: run_path.display().to_string(),
            json: true,
        },
    )
    .expect("show should load the persisted artifact");

    assert_eq!(shown, finished);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_show_text_surfaces_decision_fields_first() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-show-text");
    let config_path = write_runtime_experiment_config(&root);
    let (_, finished) = finish_runtime_experiment(&root, &config_path);

    let rendered =
        loongclaw_daemon::runtime_experiment_cli::render_runtime_experiment_text(&finished);
    let lines = rendered.lines().take(8).collect::<Vec<_>>();

    assert_eq!(lines[0], format!("run_id={}", finished.run_id));
    assert_eq!(
        lines[1],
        format!("experiment_id={}", finished.experiment_id)
    );
    assert_eq!(
        lines[2],
        format!(
            "baseline_snapshot_id={}",
            finished.baseline_snapshot.snapshot_id
        )
    );
    assert_eq!(
        lines[3],
        format!(
            "result_snapshot_id={}",
            finished
                .result_snapshot
                .as_ref()
                .expect("finish should attach result snapshot")
                .snapshot_id
        )
    );
    assert_eq!(lines[4], "status=completed");
    assert_eq!(lines[5], "decision=promoted");
    assert_eq!(lines[6], "metrics=task_success:1,token_delta:0");
    assert_eq!(lines[7], "warnings=manual verification only");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_record_only_surfaces_decision_summary() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-record-only");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, finished) = finish_runtime_experiment(&root, &config_path);

    let report =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: None,
                result_snapshot: None,
                recorded_snapshots: false,
                json: true,
            },
        )
        .expect("record-only compare should succeed");

    assert_eq!(
        report.compare_mode,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareMode::RecordOnly
    );
    assert_eq!(report.run_id, finished.run_id);
    assert_eq!(report.status, finished.status);
    assert_eq!(report.decision, finished.decision);
    assert_eq!(
        report
            .evaluation
            .as_ref()
            .expect("compare should carry evaluation")
            .summary,
        "task success improved"
    );
    assert!(report.snapshot_delta.is_none());

    let rendered =
        loongclaw_daemon::runtime_experiment_cli::render_runtime_experiment_compare_text(&report);
    assert!(rendered.contains("compare_mode=record_only"));
    assert!(rendered.contains("evaluation_summary=task success improved"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_with_snapshot_delta_reports_changed_runtime_surfaces() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-snapshot-delta");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, baseline_snapshot_path, result_snapshot_path, _) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);

    let report =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: Some(baseline_snapshot_path.display().to_string()),
                result_snapshot: Some(result_snapshot_path.display().to_string()),
                recorded_snapshots: false,
                json: true,
            },
        )
        .expect("snapshot-delta compare should succeed");

    assert_eq!(
        report.compare_mode,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareMode::SnapshotDelta
    );
    let snapshot_delta = report
        .snapshot_delta
        .as_ref()
        .expect("compare should include snapshot delta");
    assert_eq!(
        snapshot_delta.provider_active_profile.before.as_deref(),
        Some("deepseek-lab")
    );
    assert_eq!(
        snapshot_delta.provider_active_profile.after.as_deref(),
        Some("openai-main")
    );
    assert_eq!(
        snapshot_delta.provider_active_model.before.as_deref(),
        Some("deepseek-chat")
    );
    assert_eq!(
        snapshot_delta.provider_active_model.after.as_deref(),
        Some("gpt-4.1-mini")
    );
    assert!(
        !snapshot_delta.visible_tool_names.removed.is_empty(),
        "tool diff should report removed tools after disabling browser and web"
    );
    assert!(
        snapshot_delta.changed_surface_count >= 4,
        "compare should report multiple changed runtime surfaces"
    );

    let rendered =
        loongclaw_daemon::runtime_experiment_cli::render_runtime_experiment_compare_text(&report);
    assert!(rendered.contains("compare_mode=snapshot_delta"));
    assert!(rendered.contains("provider_active_profile=deepseek-lab -> openai-main"));
    assert!(rendered.contains("provider_active_model=deepseek-chat -> gpt-4.1-mini"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_with_recorded_snapshots_reports_changed_runtime_surfaces() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-recorded-snapshots");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, _baseline_snapshot_path, _result_snapshot_path, _) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);

    let report =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: None,
                result_snapshot: None,
                recorded_snapshots: true,
                json: true,
            },
        )
        .expect("recorded snapshot compare should succeed");

    assert_eq!(
        report.compare_mode,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareMode::SnapshotDelta
    );
    let snapshot_delta = report
        .snapshot_delta
        .as_ref()
        .expect("compare should include snapshot delta");
    assert_eq!(
        snapshot_delta.provider_active_profile.before.as_deref(),
        Some("deepseek-lab")
    );
    assert_eq!(
        snapshot_delta.provider_active_profile.after.as_deref(),
        Some("openai-main")
    );
    assert!(
        snapshot_delta.changed_surface_count >= 4,
        "recorded snapshot compare should report runtime surface deltas"
    );

    let rendered =
        loongclaw_daemon::runtime_experiment_cli::render_runtime_experiment_compare_text(&report);
    assert!(rendered.contains("compare_mode=snapshot_delta"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_requires_both_snapshot_paths_for_deep_compare() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-partial");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, baseline_snapshot_path, _, _) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: Some(baseline_snapshot_path.display().to_string()),
                result_snapshot: None,
                recorded_snapshots: false,
                json: false,
            },
        )
        .expect_err("partial snapshot input should be rejected");

    assert!(error.contains("requires --baseline-snapshot and --result-snapshot together"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_rejects_snapshot_identity_mismatch() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-mismatch");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, baseline_snapshot_path, _, _) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);
    let (other_result_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot-other.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:45:00Z".to_owned(),
            label: Some("other".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: Some(baseline_snapshot_path.display().to_string()),
                result_snapshot: Some(other_result_snapshot_path.display().to_string()),
                recorded_snapshots: false,
                json: false,
            },
        )
        .expect_err("mismatched result snapshot should be rejected");

    assert!(error.contains("result snapshot"));
    assert!(error.contains("snapshot_id"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_treats_missing_snapshot_sections_as_absent() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-missing-sections");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, baseline_snapshot_path, result_snapshot_path, _) =
        finish_runtime_experiment_with_missing_compare_sections(&root, &config_path);

    let report =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: Some(baseline_snapshot_path.display().to_string()),
                result_snapshot: Some(result_snapshot_path.display().to_string()),
                recorded_snapshots: false,
                json: true,
            },
        )
        .expect("compare should tolerate missing snapshot sections");

    let snapshot_delta = report
        .snapshot_delta
        .as_ref()
        .expect("compare should include snapshot delta");
    assert_eq!(snapshot_delta.context_engine_compaction.before, None);
    assert_eq!(snapshot_delta.memory_policy.before, None);
    assert_eq!(snapshot_delta.acp_policy.before, None);
    assert!(
        snapshot_delta.changed_surface_count >= 3,
        "missing sections should still register as changed surfaces"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_recorded_snapshots_rejects_missing_recorded_result_path() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-recorded-missing-path");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, _, _, _) = finish_runtime_experiment_with_compare_delta(&root, &config_path);
    rewrite_json_file(&run_path, |payload| {
        payload["result_snapshot"]
            .as_object_mut()
            .expect("result_snapshot should be an object")
            .remove("artifact_path");
    });

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: None,
                result_snapshot: None,
                recorded_snapshots: true,
                json: false,
            },
        )
        .expect_err("recorded snapshot compare should reject missing recorded result path");

    assert!(error.contains("missing recorded result snapshot path"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_recorded_snapshots_rejects_unresolvable_result_path() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-recorded-unresolvable-path");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, _, result_snapshot_path, _) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);
    fs::remove_file(&result_snapshot_path).expect("result snapshot fixture should be removable");

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: None,
                result_snapshot: None,
                recorded_snapshots: true,
                json: false,
            },
        )
        .expect_err("recorded snapshot compare should reject an unresolvable result path");

    assert!(error.contains("recorded result snapshot path"));
    assert!(error.contains("cannot be resolved"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_compare_recorded_snapshots_rejects_unavailable_result_stage() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-compare-recorded-no-result");
    let config_path = write_runtime_experiment_config(&root);
    let (baseline_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &baseline_snapshot_path, None);

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_compare_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCompareCommandOptions {
                run: run_path.display().to_string(),
                baseline_snapshot: None,
                result_snapshot: None,
                recorded_snapshots: true,
                json: false,
            },
        )
        .expect_err("planned run should not deep-compare recorded snapshots");

    assert!(error.contains("has no result snapshot"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_restore_uses_recorded_result_snapshot_path() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-restore-stage-result");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, _, result_snapshot_path, finished) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);

    let execution =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_restore_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreCommandOptions {
                run: run_path.display().to_string(),
                stage:
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreStage::Result,
                config: Some(config_path.display().to_string()),
                json: false,
                apply: false,
            },
        )
        .expect("runtime experiment restore should resolve the result snapshot");

    assert_eq!(
        execution.stage,
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreStage::Result
    );
    assert_eq!(
        execution.snapshot_path,
        canonical_display_path(&result_snapshot_path)
    );
    assert_eq!(
        execution.restore.lineage.snapshot_id,
        finished
            .result_snapshot
            .as_ref()
            .expect("finished run should record result snapshot")
            .snapshot_id
    );
    assert!(!execution.restore.applied);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_restore_rejects_missing_recorded_stage_path() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-restore-stage-missing-path");
    let config_path = write_runtime_experiment_config(&root);
    let (run_path, _, _, _) = finish_runtime_experiment_with_compare_delta(&root, &config_path);
    rewrite_json_file(&run_path, |payload| {
        payload["result_snapshot"]
            .as_object_mut()
            .expect("result_snapshot should be an object")
            .remove("artifact_path");
    });

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_restore_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreCommandOptions {
                run: run_path.display().to_string(),
                stage:
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreStage::Result,
                config: Some(config_path.display().to_string()),
                json: false,
                apply: false,
            },
        )
        .expect_err("runtime experiment restore should reject old artifacts without a path");

    assert!(error.contains("missing recorded result snapshot path"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_experiment_restore_rejects_unavailable_result_stage() {
    let root = unique_temp_dir("loongclaw-runtime-experiment-restore-stage-unavailable");
    let config_path = write_runtime_experiment_config(&root);
    let (baseline_snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-16T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &baseline_snapshot_path, None);

    let error =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_restore_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreCommandOptions {
                run: run_path.display().to_string(),
                stage:
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreStage::Result,
                config: Some(config_path.display().to_string()),
                json: false,
                apply: false,
            },
        )
        .expect_err("planned runs should not expose a result stage restore");

    assert!(error.contains("has no recorded result snapshot to restore"));

    fs::remove_dir_all(&root).ok();
}
