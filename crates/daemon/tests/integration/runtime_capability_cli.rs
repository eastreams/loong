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

fn start_runtime_experiment_variant(
    root: &Path,
    snapshot_path: &Path,
    slug: &str,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let run_path = root.join(format!("artifacts/runtime-experiment-{slug}.json"));
    let run = loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_start_command(
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentStartCommandOptions {
            snapshot: snapshot_path.display().to_string(),
            output: run_path.display().to_string(),
            mutation_summary: format!("enable browser preview skill ({slug})"),
            experiment_id: Some("exp-42".to_owned()),
            label: Some(format!("browser-preview-{slug}")),
            tag: vec!["browser".to_owned(), slug.to_owned()],
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

fn finish_runtime_experiment_variant(
    root: &Path,
    config_path: &Path,
    slug: &str,
    cost_delta: f64,
    warnings: &[&str],
    decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        root,
        config_path,
        &format!("artifacts/runtime-snapshot-{slug}.json"),
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-17T12:00:00Z".to_owned(),
            label: Some(format!("baseline-{slug}")),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment_variant(root, &baseline_snapshot_path, slug);
    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        config_path,
        &format!("artifacts/runtime-snapshot-result-{slug}.json"),
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-17T12:30:00Z".to_owned(),
            label: Some(format!("candidate-{slug}")),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some(baseline_snapshot_id),
        },
    );

    let finished =
        loongclaw_daemon::runtime_experiment_cli::execute_runtime_experiment_finish_command(
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishCommandOptions {
                run: run_path.display().to_string(),
                result_snapshot: result_snapshot_path.display().to_string(),
                evaluation_summary: format!("provider and tool policy updated ({slug})"),
                metric: vec![
                    "task_success=1".to_owned(),
                    format!("cost_delta={cost_delta}"),
                ],
                warning: warnings.iter().map(|warning| (*warning).to_owned()).collect(),
                decision,
                status:
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
                json: false,
            },
        )
        .expect("runtime experiment finish should succeed");
    (run_path, finished)
}

fn propose_runtime_capability_variant(
    root: &Path,
    run_path: &Path,
    slug: &str,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityArtifactDocument,
) {
    let candidate_path = root.join(format!("artifacts/runtime-capability-{slug}.json"));
    let candidate = propose_runtime_capability_variant_with_target(
        root,
        run_path,
        slug,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    (candidate_path, candidate)
}

fn propose_runtime_capability_variant_with_target(
    root: &Path,
    run_path: &Path,
    slug: &str,
    target: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget,
    target_summary: &str,
    bounded_scope: &str,
    required_capabilities: &[&str],
    tags: &[&str],
) -> loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityArtifactDocument {
    let candidate_path = root.join(format!("artifacts/runtime-capability-{slug}.json"));
    loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
            run: run_path.display().to_string(),
            output: candidate_path.display().to_string(),
            target,
            target_summary: target_summary.to_owned(),
            bounded_scope: bounded_scope.to_owned(),
            required_capability: required_capabilities
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            tag: tags.iter().map(|value| (*value).to_owned()).collect(),
            label: Some(format!("runtime-capability-{slug}")),
            json: false,
        },
    )
    .expect("runtime capability propose should succeed")
}

fn review_runtime_capability_variant(
    candidate_path: &Path,
    decision: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision,
    slug: &str,
) -> loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityArtifactDocument {
    loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_review_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewCommandOptions {
            candidate: candidate_path.display().to_string(),
            decision,
            review_summary: format!("reviewed runtime capability candidate {slug}"),
            warning: Vec::new(),
            json: false,
        },
    )
    .expect("runtime capability review should succeed")
}

fn rewrite_runtime_capability_created_at(candidate_path: &Path, created_at: &str) {
    let mut payload = serde_json::from_str::<Value>(
        &fs::read_to_string(candidate_path).expect("read runtime capability artifact"),
    )
    .expect("decode runtime capability artifact");
    let created_at_value = payload
        .as_object_mut()
        .and_then(|artifact| artifact.get_mut("created_at"))
        .expect("runtime capability artifact should include created_at");
    *created_at_value = Value::String(created_at.to_owned());
    fs::write(
        candidate_path,
        serde_json::to_string_pretty(&payload).expect("encode runtime capability artifact"),
    )
    .expect("rewrite runtime capability artifact");
}

fn make_runtime_capability_review_state_inconsistent(candidate_path: &Path) {
    let mut payload = serde_json::from_str::<Value>(
        &fs::read_to_string(candidate_path).expect("read runtime capability artifact"),
    )
    .expect("decode runtime capability artifact");
    payload["status"] = Value::String("reviewed".to_owned());
    fs::write(
        candidate_path,
        serde_json::to_string_pretty(&payload).expect("encode malformed capability candidate"),
    )
    .expect("persist malformed capability candidate");
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

#[test]
fn runtime_capability_show_rejects_inconsistent_review_state() {
    let root = unique_temp_dir("loongclaw-runtime-capability-show-invalid-state");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability.json");

    loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
            run: run_path.display().to_string(),
            output: candidate_path.display().to_string(),
            target: loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
            target_summary: "Codify browser preview onboarding as a reusable managed skill"
                .to_owned(),
            bounded_scope: "Browser preview onboarding and companion readiness checks only"
                .to_owned(),
            required_capability: vec!["invoke_tool".to_owned(), "memory_read".to_owned()],
            tag: vec!["browser".to_owned(), "onboarding".to_owned()],
            label: Some("browser-preview-invalid-state".to_owned()),
            json: false,
        },
    )
    .expect("runtime capability propose should succeed");

    make_runtime_capability_review_state_inconsistent(&candidate_path);

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_show_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityShowCommandOptions {
            candidate: candidate_path.display().to_string(),
            json: false,
        },
    )
    .expect_err("inconsistent review state should be rejected");

    assert!(error.contains("inconsistent"), "error: {error}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_groups_related_candidates_and_reports_ready_family() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-ready");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (candidate_a_path, _) = propose_runtime_capability_variant(&root, &run_a_path, "a");
    let (candidate_b_path, _) = propose_runtime_capability_variant(&root, &run_b_path, "b");
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "b",
    );

    fs::write(
        root.join("artifacts/ignore-me.json"),
        "{\"hello\":\"world\"}",
    )
    .expect("write unrelated json fixture");

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");

    assert_eq!(report.total_candidate_count, 2);
    assert_eq!(report.family_count, 1);

    let family = report
        .families
        .first()
        .expect("one capability family should be reported");
    assert_eq!(
        family.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::Ready
    );
    assert_eq!(family.evidence.total_candidates, 2);
    assert_eq!(family.evidence.accepted_candidates, 2);
    assert_eq!(family.evidence.distinct_source_run_count, 2);
    assert_eq!(
        family
            .evidence
            .metric_ranges
            .get("cost_delta")
            .expect("cost delta range should exist")
            .min,
        -0.4
    );
    assert_eq!(
        family
            .evidence
            .metric_ranges
            .get("cost_delta")
            .expect("cost delta range should exist")
            .max,
        -0.2
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_marks_family_not_ready_when_evidence_is_incomplete() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-not-ready");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "solo",
        -0.2,
        &["manual verification only"],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (candidate_path, _) = propose_runtime_capability_variant(&root, &run_path, "solo");
    review_runtime_capability_variant(
        &candidate_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "solo",
    );

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");

    let family = report
        .families
        .first()
        .expect("one capability family should be reported");
    assert_eq!(
        family.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::NotReady
    );
    assert!(
        family.readiness.checks.iter().any(|check| {
            check.dimension == "stability"
                && check.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence
        }),
        "stability should require repeated evidence"
    );
    assert!(
        family.readiness.checks.iter().any(|check| {
            check.dimension == "warning_pressure"
                && check.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence
        }),
        "warnings should keep the family out of ready state"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_marks_family_blocked_on_conflicting_reviews() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-blocked");
    let config_path = write_runtime_capability_config(&root);
    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "accept",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "reject",
        -0.1,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (candidate_a_path, _) = propose_runtime_capability_variant(&root, &run_a_path, "accept");
    let (candidate_b_path, _) = propose_runtime_capability_variant(&root, &run_b_path, "reject");
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "accept",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Rejected,
        "reject",
    );

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");

    let family = report
        .families
        .first()
        .expect("one capability family should be reported");
    assert_eq!(
        family.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::Blocked
    );
    assert!(
        family.readiness.checks.iter().any(|check| {
            check.dimension == "review_consensus"
                && check.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::Blocked
        }),
        "review consensus should block mixed accepted/rejected evidence"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_rejects_malformed_supported_artifact_during_scan() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-malformed");
    let config_path = write_runtime_capability_config(&root);
    let (valid_run_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "valid",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (invalid_run_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "invalid",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (valid_candidate_path, _) =
        propose_runtime_capability_variant(&root, &valid_run_path, "valid");
    review_runtime_capability_variant(
        &valid_candidate_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "valid",
    );
    let (invalid_candidate_path, _) =
        propose_runtime_capability_variant(&root, &invalid_run_path, "invalid");
    make_runtime_capability_review_state_inconsistent(&invalid_candidate_path);

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
            root: root.join("artifacts").display().to_string(),
            json: false,
        },
    )
    .expect_err("malformed supported artifacts should abort index scans");

    assert!(
        error.contains("inconsistent reviewed state"),
        "error should surface the invalid review state: {error}"
    );
    assert!(
        error.contains(&invalid_candidate_path.display().to_string()),
        "error should identify the malformed artifact path: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_builds_promotable_managed_skill_plan() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-ready");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "ready-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "ready-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path = root.join("artifacts/runtime-capability-ready-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-ready-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "ready-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "ready-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "ready-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "ready-b",
    );

    let index_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");
    let family = index_report
        .families
        .first()
        .expect("one capability family should be reported");

    let plan = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_plan_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityPlanCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect("runtime capability plan should succeed");

    assert!(plan.promotable, "ready family should be promotable");
    assert_eq!(
        plan.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::Ready
    );
    assert_eq!(plan.planned_artifact.artifact_kind, "managed_skill_bundle");
    assert_eq!(plan.planned_artifact.delivery_surface, "managed_skills");
    assert!(
        plan.planned_artifact
            .artifact_id
            .starts_with("managed-skill-"),
        "artifact id should carry a managed-skill prefix"
    );
    assert!(
        plan.planned_artifact
            .artifact_id
            .ends_with(&family.family_id[..12]),
        "artifact id should be family-derived"
    );
    assert!(
        plan.blockers.is_empty(),
        "ready family should have no blockers"
    );
    assert!(
        plan.approval_checklist
            .iter()
            .any(|item| item.contains("managed skill")),
        "checklist should include the target-specific managed skill review item"
    );
    assert!(
        plan.rollback_hints
            .iter()
            .any(|hint| hint.contains("managed_skills")),
        "rollback hints should mention the managed skill delivery surface"
    );
    assert_eq!(plan.provenance.candidate_ids.len(), 2);
    assert_eq!(plan.provenance.source_run_ids.len(), 2);
    assert!(
        plan.provenance.source_run_artifact_paths.contains(
            &fs::canonicalize(&run_a_path)
                .expect("canonicalize run a path")
                .display()
                .to_string()
        )
    );
    assert!(
        plan.provenance.source_run_artifact_paths.contains(
            &fs::canonicalize(&run_b_path)
                .expect("canonicalize run b path")
                .display()
                .to_string()
        )
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_rejects_malformed_supported_artifact_during_scan() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-malformed");
    let config_path = write_runtime_capability_config(&root);
    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "ready-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "ready-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_bad_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "bad",
        -0.1,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (candidate_a_path, _) = propose_runtime_capability_variant(&root, &run_a_path, "ready-a");
    let (candidate_b_path, _) = propose_runtime_capability_variant(&root, &run_b_path, "ready-b");
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "ready-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "ready-b",
    );

    let family_id =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("index should succeed before introducing malformed artifacts")
        .families
        .first()
        .expect("one capability family should be reported")
        .family_id
        .clone();

    let (invalid_candidate_path, _) =
        propose_runtime_capability_variant(&root, &run_bad_path, "bad");
    make_runtime_capability_review_state_inconsistent(&invalid_candidate_path);

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_plan_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityPlanCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id,
            json: false,
        },
    )
    .expect_err("malformed supported artifacts should abort plan scans");

    assert!(
        error.contains("inconsistent reviewed state"),
        "error should surface the invalid review state: {error}"
    );
    assert!(
        error.contains(&invalid_candidate_path.display().to_string()),
        "error should identify the malformed artifact path: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_reports_missing_evidence_for_programmatic_flow_family() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-not-ready");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "flow",
        -0.2,
        &["manual verification only"],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let candidate_path = root.join("artifacts/runtime-capability-flow.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_path,
        "flow",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ProgrammaticFlow,
        "Codify runtime compare summarization as a reusable flow",
        "Runtime experiment compare report generation only",
        &["invoke_tool", "memory_read"],
        &["runtime", "compare"],
    );
    review_runtime_capability_variant(
        &candidate_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "flow",
    );

    let index_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");
    let family = index_report
        .families
        .first()
        .expect("one capability family should be reported");

    let plan = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_plan_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityPlanCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect("runtime capability plan should succeed");

    assert!(
        !plan.promotable,
        "not-ready family should not be promotable"
    );
    assert_eq!(
        plan.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::NotReady
    );
    assert_eq!(
        plan.planned_artifact.artifact_kind,
        "programmatic_flow_spec"
    );
    assert_eq!(plan.planned_artifact.delivery_surface, "programmatic_flows");
    assert!(
        plan.blockers.iter().any(|blocker| {
            blocker.dimension == "stability"
                && blocker.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence
        }),
        "stability should surface as a missing-evidence blocker"
    );
    assert!(
        plan.blockers.iter().any(|blocker| {
            blocker.dimension == "warning_pressure"
                && blocker.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence
        }),
        "warnings should surface as missing-evidence blockers"
    );
    assert!(
        plan.approval_checklist
            .iter()
            .any(|item| item.contains("programmatic flow")),
        "checklist should include the target-specific programmatic flow review item"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_reports_blocked_profile_note_family() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-blocked");
    let config_path = write_runtime_capability_config(&root);
    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "note-a",
        -0.1,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "note-b",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let candidate_a_path = root.join("artifacts/runtime-capability-note-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-note-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "note-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ProfileNoteAddendum,
        "Record browser preview operator guidance in profile memory",
        "Browser preview operator guidance only",
        &["memory_write"],
        &["profile", "guidance"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "note-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ProfileNoteAddendum,
        "Record browser preview operator guidance in profile memory",
        "Browser preview operator guidance only",
        &["memory_write"],
        &["profile", "guidance"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "note-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Rejected,
        "note-b",
    );

    let index_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");
    let family = index_report
        .families
        .first()
        .expect("one capability family should be reported");

    let plan = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_plan_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityPlanCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect("runtime capability plan should succeed");

    assert!(!plan.promotable, "blocked family should not be promotable");
    assert_eq!(
        plan.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::Blocked
    );
    assert_eq!(plan.planned_artifact.artifact_kind, "profile_note_addendum");
    assert_eq!(plan.planned_artifact.delivery_surface, "profile_note");
    assert!(
        plan.blockers.iter().any(|blocker| {
            blocker.dimension == "review_consensus"
                && blocker.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::Blocked
        }),
        "blocked review consensus should surface as a hard-stop blocker"
    );
    assert!(
        plan.approval_checklist
            .iter()
            .any(|item| item.contains("advisory profile guidance")),
        "checklist should include the target-specific profile-note review item"
    );
    assert!(
        plan.rollback_hints
            .iter()
            .any(|hint| hint.contains("profile_note")),
        "rollback hints should mention the profile note delivery surface"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_provenance_candidate_ids_follow_family_order() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-provenance-order");
    let config_path = write_runtime_capability_config(&root);
    let (run_z_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "z-run",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "a-run",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let candidate_z_path = root.join("artifacts/runtime-capability-zzz-first.json");
    let candidate_a_path = root.join("artifacts/runtime-capability-aaa-second.json");
    let candidate_z = propose_runtime_capability_variant_with_target(
        &root,
        &run_z_path,
        "zzz-first",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    let candidate_a = propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "aaa-second",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    rewrite_runtime_capability_created_at(&candidate_z_path, "2026-03-18T08:00:00Z");
    rewrite_runtime_capability_created_at(&candidate_a_path, "2026-03-18T08:00:01Z");
    review_runtime_capability_variant(
        &candidate_z_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "zzz-first",
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "aaa-second",
    );

    let index_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");
    let family = index_report
        .families
        .first()
        .expect("one capability family should be reported");
    let plan = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_plan_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityPlanCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect("runtime capability plan should succeed");

    assert_eq!(
        family.candidate_ids,
        vec![candidate_z.candidate_id, candidate_a.candidate_id],
        "family summary should use semantic candidate order"
    );
    assert_eq!(
        plan.provenance.candidate_ids, family.candidate_ids,
        "planner provenance should preserve the family candidate order"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_rejects_unknown_family_id() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-missing-family");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment(&root, &config_path);
    propose_runtime_capability_variant(&root, &run_path, "missing");

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_plan_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityPlanCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: "missing-family".to_owned(),
            json: false,
        },
    )
    .expect_err("unknown family id should be rejected");

    assert!(
        error.contains("missing-family"),
        "error should name the requested family id: {error}"
    );

    fs::remove_dir_all(&root).ok();
}
