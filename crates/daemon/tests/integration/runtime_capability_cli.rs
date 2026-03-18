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

fn write_runtime_capability_config(root: &Path) -> PathBuf {
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

fn start_runtime_experiment(
    root: &Path,
    snapshot_path: &Path,
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
            experiment_id: Some("exp-42".to_owned()),
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
            created_at: "2026-03-17T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(root, &baseline_snapshot_path);
    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        config_path,
        "artifacts/runtime-snapshot-result.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-17T12:30:00Z".to_owned(),
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
    (run_path, finished)
}

#[test]
fn runtime_capability_propose_persists_candidate_from_finished_run() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, run) = finish_runtime_experiment(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability.json");

    let candidate =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
                run: run_path.display().to_string(),
                output: candidate_path.display().to_string(),
                target:
                    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
                target_summary: "Codify browser preview onboarding as a reusable managed skill"
                    .to_owned(),
                bounded_scope: "Browser preview onboarding and companion readiness checks only"
                    .to_owned(),
                required_capability: vec![
                    "invoke_tool".to_owned(),
                    "memory_read".to_owned(),
                    "invoke_tool".to_owned(),
                ],
                tag: vec![
                    "browser".to_owned(),
                    "onboarding".to_owned(),
                    "browser".to_owned(),
                ],
                label: Some("browser-preview-skill-candidate".to_owned()),
                json: false,
            },
        )
        .expect("runtime capability propose should succeed");

    assert_eq!(
        candidate.label.as_deref(),
        Some("browser-preview-skill-candidate")
    );
    assert_eq!(
        candidate.source_run.run_id, run.run_id,
        "candidate should retain source run linkage"
    );
    assert_eq!(
        candidate.proposal.required_capabilities,
        vec!["invoke_tool".to_owned(), "memory_read".to_owned()]
    );
    assert_eq!(
        candidate.proposal.tags,
        vec!["browser".to_owned(), "onboarding".to_owned()]
    );
    assert!(
        candidate_path.exists(),
        "propose should persist the candidate artifact"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_propose_rejects_planned_runs() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose-planned");
    let config_path = write_runtime_capability_config(&root);
    let (snapshot_path, _) = write_snapshot_artifact(
        &root,
        &config_path,
        "artifacts/runtime-snapshot.json",
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-17T12:00:00Z".to_owned(),
            label: Some("baseline".to_owned()),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment(&root, &snapshot_path);

    let error =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
                run: run_path.display().to_string(),
                output: root
                    .join("artifacts/runtime-capability.json")
                    .display()
                    .to_string(),
                target:
                    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
                target_summary: "Codify browser preview onboarding as a reusable managed skill"
                    .to_owned(),
                bounded_scope: "Browser preview onboarding and companion readiness checks only"
                    .to_owned(),
                required_capability: vec!["invoke_tool".to_owned()],
                tag: vec!["browser".to_owned()],
                label: None,
                json: false,
            },
        )
        .expect_err("planned run should be rejected");

    assert!(error.contains("finished"), "error: {error}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_propose_rejects_unknown_required_capability() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose-capability");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment(&root, &config_path);

    let error =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
                run: run_path.display().to_string(),
                output: root
                    .join("artifacts/runtime-capability.json")
                    .display()
                    .to_string(),
                target: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ProgrammaticFlow,
                target_summary: "Codify runtime comparison as a reusable flow".to_owned(),
                bounded_scope: "Runtime experiment compare reports only".to_owned(),
                required_capability: vec!["totally_unknown".to_owned()],
                tag: vec!["runtime".to_owned()],
                label: None,
                json: false,
            },
        )
        .expect_err("unknown capabilities should be rejected");

    assert!(error.contains("totally_unknown"), "error: {error}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_review_records_terminal_decision_once() {
    let root = unique_temp_dir("loongclaw-runtime-capability-review");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability.json");

    let proposed =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
                run: run_path.display().to_string(),
                output: candidate_path.display().to_string(),
                target:
                    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
                target_summary: "Codify browser preview onboarding as a reusable managed skill"
                    .to_owned(),
                bounded_scope: "Browser preview onboarding and companion readiness checks only"
                    .to_owned(),
                required_capability: vec!["invoke_tool".to_owned(), "memory_read".to_owned()],
                tag: vec!["browser".to_owned(), "onboarding".to_owned()],
                label: None,
                json: false,
            },
        )
        .expect("runtime capability propose should succeed");

    let reviewed =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_review_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewCommandOptions {
                candidate: candidate_path.display().to_string(),
                decision: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
                review_summary:
                    "Promotion target is bounded and evidence supports manual codification"
                        .to_owned(),
                warning: vec!["still requires manual implementation".to_owned()],
                json: false,
            },
        )
        .expect("runtime capability review should succeed");

    assert_eq!(reviewed.candidate_id, proposed.candidate_id);
    assert!(
        reviewed.reviewed_at.is_some(),
        "review should record a terminal timestamp"
    );

    let error =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_review_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewCommandOptions {
                candidate: candidate_path.display().to_string(),
                decision: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Rejected,
                review_summary: "second review should fail".to_owned(),
                warning: Vec::new(),
                json: false,
            },
        )
        .expect_err("double review should fail");

    assert!(error.contains("already reviewed"), "error: {error}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_show_round_trips_the_persisted_artifact() {
    let root = unique_temp_dir("loongclaw-runtime-capability-show");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability.json");

    let proposed =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
                run: run_path.display().to_string(),
                output: candidate_path.display().to_string(),
                target: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ProfileNoteAddendum,
                target_summary: "Persist browser preview operator guidance".to_owned(),
                bounded_scope: "Imported operator guidance only".to_owned(),
                required_capability: vec!["memory_write".to_owned()],
                tag: vec!["memory".to_owned(), "guidance".to_owned()],
                label: Some("browser-preview-guidance".to_owned()),
                json: false,
            },
        )
        .expect("runtime capability propose should succeed");

    let shown = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_show_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityShowCommandOptions {
            candidate: candidate_path.display().to_string(),
            json: false,
        },
    )
    .expect("show should round-trip the persisted artifact");

    assert_eq!(shown, proposed);

    fs::remove_dir_all(&root).ok();
}
