#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::MutexGuard,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();
    let canonical_temp_dir = dunce::canonicalize(&temp_dir).unwrap_or(temp_dir);
    canonical_temp_dir.join(format!("{prefix}-{nanos}"))
}

fn normalized_path_text(value: &str) -> String {
    value.replace('\\', "/")
}

fn canonicalized_path_text(path: &Path) -> String {
    let canonical_path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical_path.display().to_string()
}

fn artifact_path_suffix(path: &Path) -> String {
    let normalized_path = normalized_path_text(&path.display().to_string());
    let suffix_parts = normalized_path.rsplit('/').take(2).collect::<Vec<_>>();
    let ordered_suffix_parts = suffix_parts.into_iter().rev().collect::<Vec<_>>();
    ordered_suffix_parts.join("/")
}

struct RuntimeCapabilityEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl RuntimeCapabilityEnvironmentGuard {
    fn set(root: &Path) -> Self {
        let lock = super::lock_daemon_test_environment();
        let home = root.join("home");
        let loongclaw_home = home.join(mvp::config::HOME_DIR_NAME);
        fs::create_dir_all(&loongclaw_home).expect("create isolated loongclaw home");
        let home_text = home.to_string_lossy().into_owned();
        let loongclaw_home_text = loongclaw_home.to_string_lossy().into_owned();

        let pairs = [
            ("HOME", Some(home_text.as_str())),
            ("LOONGCLAW_HOME", Some(loongclaw_home_text.as_str())),
            ("LOONGCLAW_BROWSER_COMPANION_READY", None),
        ];
        let mut saved = Vec::new();
        for (key, value) in pairs {
            saved.push((key.to_owned(), std::env::var_os(key)));
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }

        Self { _lock: lock, saved }
    }
}

impl Drop for RuntimeCapabilityEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(&key, value);
                },
                None => unsafe {
                    std::env::remove_var(&key);
                },
            }
        }
    }
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
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "demo-token".to_owned(),
                )),
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

fn rewrite_runtime_capability_compare_config(config_path: &Path) {
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

fn rewrite_runtime_capability_compare_config_for_memory_stage_profile(config_path: &Path) {
    let (_, mut config) = mvp::config::load(Some(
        config_path
            .to_str()
            .expect("config path should be valid utf-8"),
    ))
    .expect("load config fixture");
    config.memory.profile = mvp::config::MemoryProfile::WindowPlusSummary;
    config.conversation.compact_min_messages = Some(8);
    config.conversation.compact_trigger_estimated_tokens = Some(512);
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
    let _env_guard = RuntimeCapabilityEnvironmentGuard::set(root);
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

fn finish_runtime_experiment_with_compare_delta(
    root: &Path,
    config_path: &Path,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let _env_guard = RuntimeCapabilityEnvironmentGuard::set(root);
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

    rewrite_runtime_capability_compare_config(config_path);

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
                decision:
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
                status:
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed,
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

fn finish_runtime_experiment_variant_with_compare_delta(
    root: &Path,
    slug: &str,
    cost_delta: f64,
    warnings: &[&str],
    decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let config_path = write_runtime_capability_config(root);
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        root,
        &config_path,
        &format!("artifacts/runtime-snapshot-{slug}.json"),
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-17T12:00:00Z".to_owned(),
            label: Some(format!("baseline-{slug}")),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment_variant(root, &baseline_snapshot_path, slug);

    rewrite_runtime_capability_compare_config(&config_path);

    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        &config_path,
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

fn finish_runtime_experiment_variant_with_memory_compare_delta(
    root: &Path,
    slug: &str,
    cost_delta: f64,
    warnings: &[&str],
    decision: loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision,
) -> (
    PathBuf,
    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentArtifactDocument,
) {
    let _env_guard = RuntimeCapabilityEnvironmentGuard::set(root);
    let config_path = write_runtime_capability_config(root);
    let (baseline_snapshot_path, baseline_snapshot_payload) = write_snapshot_artifact(
        root,
        &config_path,
        &format!("artifacts/runtime-snapshot-{slug}.json"),
        loongclaw_daemon::RuntimeSnapshotArtifactMetadata {
            created_at: "2026-03-17T12:00:00Z".to_owned(),
            label: Some(format!("baseline-{slug}")),
            experiment_id: Some("exp-42".to_owned()),
            parent_snapshot_id: Some("snapshot-parent".to_owned()),
        },
    );
    let (run_path, _) = start_runtime_experiment_variant(root, &baseline_snapshot_path, slug);

    rewrite_runtime_capability_compare_config_for_memory_stage_profile(&config_path);

    let baseline_snapshot_id = snapshot_id_from_payload(&baseline_snapshot_payload);
    let (result_snapshot_path, _) = write_snapshot_artifact(
        root,
        &config_path,
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
                evaluation_summary: format!("memory and context policy updated ({slug})"),
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
    let _env_guard = RuntimeCapabilityEnvironmentGuard::set(root);
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
    let status = payload
        .as_object_mut()
        .and_then(|artifact| artifact.get_mut("status"))
        .expect("runtime capability artifact should include status");
    *status = Value::String("reviewed".to_owned());
    fs::write(
        candidate_path,
        serde_json::to_string_pretty(&payload).expect("encode malformed capability candidate"),
    )
    .expect("persist malformed capability candidate");
}

fn rewrite_runtime_capability_proposal(
    candidate_path: &Path,
    summary: &str,
    bounded_scope: &str,
    required_capabilities: &[&str],
    tags: &[&str],
) {
    let mut payload = serde_json::from_str::<Value>(
        &fs::read_to_string(candidate_path).expect("read runtime capability artifact"),
    )
    .expect("decode runtime capability artifact");
    let proposal = payload
        .as_object_mut()
        .and_then(|artifact| artifact.get_mut("proposal"))
        .and_then(Value::as_object_mut)
        .expect("runtime capability artifact should include proposal");
    proposal.insert("summary".to_owned(), Value::String(summary.to_owned()));
    proposal.insert(
        "bounded_scope".to_owned(),
        Value::String(bounded_scope.to_owned()),
    );
    proposal.insert(
        "required_capabilities".to_owned(),
        Value::Array(
            required_capabilities
                .iter()
                .map(|value| Value::String((*value).to_owned()))
                .collect(),
        ),
    );
    proposal.insert(
        "tags".to_owned(),
        Value::Array(
            tags.iter()
                .map(|value| Value::String((*value).to_owned()))
                .collect(),
        ),
    );
    fs::write(
        candidate_path,
        serde_json::to_string_pretty(&payload).expect("encode runtime capability artifact"),
    )
    .expect("persist runtime capability artifact");
}

fn rewrite_runtime_capability_schema(candidate_path: &Path, surface: &str, purpose: &str) {
    let mut payload = serde_json::from_str::<Value>(
        &fs::read_to_string(candidate_path).expect("read runtime capability artifact"),
    )
    .expect("decode runtime capability artifact");
    let schema = payload
        .as_object_mut()
        .and_then(|artifact| artifact.get_mut("schema"))
        .and_then(Value::as_object_mut)
        .expect("runtime capability artifact should include schema");
    schema.insert("surface".to_owned(), Value::String(surface.to_owned()));
    schema.insert("purpose".to_owned(), Value::String(purpose.to_owned()));
    fs::write(
        candidate_path,
        serde_json::to_string_pretty(&payload).expect("encode runtime capability artifact"),
    )
    .expect("persist runtime capability artifact");
}

fn try_symlink_dir(original: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(original, link).is_ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (original, link);
        false
    }
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
fn runtime_capability_propose_roundtrips_memory_stage_profile_target() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose-memory-stage-profile");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _run) = finish_runtime_experiment(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability-memory-stage-profile.json");

    loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
            run: run_path.display().to_string(),
            output: candidate_path.display().to_string(),
            target:
                loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
            target_summary: "Promote governed memory pipeline intent into a reusable profile"
                .to_owned(),
            bounded_scope: "Governed memory pipeline promotion intent only".to_owned(),
            required_capability: vec!["memory_read".to_owned()],
            tag: vec!["memory".to_owned(), "pipeline".to_owned()],
            label: Some("memory-stage-profile-candidate".to_owned()),
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
    .expect("memory_stage_profile artifacts should round-trip through show");
    let payload = serde_json::to_value(&shown).expect("serialize runtime capability artifact");
    assert_eq!(
        payload.pointer("/proposal/target").and_then(Value::as_str),
        Some("memory_stage_profile")
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_propose_persists_snapshot_delta_when_recorded_snapshots_exist() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose-snapshot-delta");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _baseline_snapshot_path, _result_snapshot_path, _run) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability-delta.json");

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
                required_capability: vec!["invoke_tool".to_owned(), "memory_read".to_owned()],
                tag: vec!["browser".to_owned(), "onboarding".to_owned()],
                label: Some("browser-preview-snapshot-delta".to_owned()),
                json: false,
            },
        )
        .expect("runtime capability propose should succeed");

    let payload = serde_json::to_value(&candidate).expect("serialize candidate");
    let snapshot_delta = payload
        .pointer("/source_run/snapshot_delta")
        .and_then(Value::as_object)
        .expect("candidate should persist snapshot-backed delta evidence");
    let changed_surface_count = snapshot_delta
        .get("changed_surface_count")
        .and_then(Value::as_u64)
        .expect("snapshot delta should include changed surface count");
    assert!(
        changed_surface_count > 0,
        "snapshot delta should record at least one changed surface"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_propose_leaves_snapshot_delta_empty_without_recorded_snapshots() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose-no-snapshot-delta");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _run) = finish_runtime_experiment(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability-no-snapshot-delta.json");

    rewrite_json_file(&run_path, |payload| {
        payload["baseline_snapshot"]["artifact_path"] = Value::Null;
        payload["result_snapshot"]["artifact_path"] = Value::Null;
    });

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
                required_capability: vec!["invoke_tool".to_owned(), "memory_read".to_owned()],
                tag: vec!["browser".to_owned(), "onboarding".to_owned()],
                label: Some("browser-preview-no-snapshot-delta".to_owned()),
                json: false,
            },
        )
        .expect("runtime capability propose should succeed");

    let payload = serde_json::to_value(&candidate).expect("serialize candidate");
    assert!(
        payload
            .pointer("/source_run/snapshot_delta")
            .is_some_and(Value::is_null),
        "candidate should keep snapshot delta empty when recorded snapshots are unavailable"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_propose_rejects_broken_recorded_snapshot_delta() {
    let root = unique_temp_dir("loongclaw-runtime-capability-propose-broken-snapshot-delta");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _baseline_snapshot_path, result_snapshot_path, _run) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);

    fs::remove_file(&result_snapshot_path).expect("remove result snapshot to break recorded delta");

    let error =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_propose_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityProposeCommandOptions {
                run: run_path.display().to_string(),
                output: root
                    .join("artifacts/runtime-capability-broken-snapshot-delta.json")
                    .display()
                    .to_string(),
                target:
                    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
                target_summary: "Codify browser preview onboarding as a reusable managed skill"
                    .to_owned(),
                bounded_scope: "Browser preview onboarding and companion readiness checks only"
                    .to_owned(),
                required_capability: vec!["invoke_tool".to_owned(), "memory_read".to_owned()],
                tag: vec!["browser".to_owned(), "onboarding".to_owned()],
                label: Some("browser-preview-broken-snapshot-delta".to_owned()),
                json: false,
            },
        )
        .expect_err("broken recorded snapshots should reject capability proposal");

    assert!(error.contains("snapshot"), "error: {error}");

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
fn runtime_capability_show_accepts_artifacts_missing_snapshot_delta_field() {
    let root = unique_temp_dir("loongclaw-runtime-capability-show-legacy-delta");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _, _, _) = finish_runtime_experiment_with_compare_delta(&root, &config_path);
    let candidate_path = root.join("artifacts/runtime-capability-legacy.json");

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
            label: Some("browser-preview-legacy".to_owned()),
            json: false,
        },
    )
    .expect("runtime capability propose should succeed");

    rewrite_json_file(&candidate_path, |payload| {
        payload["source_run"]
            .as_object_mut()
            .expect("source_run should be an object")
            .remove("snapshot_delta");
    });

    let shown = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_show_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityShowCommandOptions {
            candidate: candidate_path.display().to_string(),
            json: false,
        },
    )
    .expect("show should keep backward compatibility with legacy artifacts");

    assert!(
        shown.source_run.snapshot_delta.is_none(),
        "missing snapshot_delta should deserialize as None"
    );

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
fn runtime_capability_show_rejects_wrong_schema_purpose() {
    let root = unique_temp_dir("loongclaw-runtime-capability-show-wrong-purpose");
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
            label: Some("browser-preview-invalid-schema".to_owned()),
            json: false,
        },
    )
    .expect("runtime capability propose should succeed");

    rewrite_runtime_capability_schema(
        &candidate_path,
        "runtime_capability",
        "promotion_plan_record",
    );

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_show_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityShowCommandOptions {
            candidate: candidate_path.display().to_string(),
            json: false,
        },
    )
    .expect_err("wrong-purpose artifacts should be rejected");

    assert!(error.contains("unsupported schema"), "error: {error}");
    assert!(error.contains("promotion_plan_record"), "error: {error}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_show_rejects_wrong_schema_surface() {
    let root = unique_temp_dir("loongclaw-runtime-capability-show-wrong-surface");
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
            label: Some("browser-preview-invalid-schema".to_owned()),
            json: false,
        },
    )
    .expect("runtime capability propose should succeed");

    rewrite_runtime_capability_schema(
        &candidate_path,
        "runtime_capability_preview",
        "promotion_candidate_record",
    );

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_show_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityShowCommandOptions {
            candidate: candidate_path.display().to_string(),
            json: false,
        },
    )
    .expect_err("wrong-surface artifacts should be rejected");

    assert!(error.contains("unsupported schema"), "error: {error}");
    assert!(
        error.contains("runtime_capability_preview"),
        "error: {error}"
    );

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
fn runtime_capability_index_reports_delta_evidence_digest() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-delta-digest");
    let (run_a_path, _) = finish_runtime_experiment_variant_with_compare_delta(
        &root,
        "delta-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_compare_delta(
        &root,
        "delta-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (candidate_a_path, _) = propose_runtime_capability_variant(&root, &run_a_path, "delta-a");
    let (candidate_b_path, _) = propose_runtime_capability_variant(&root, &run_b_path, "delta-b");
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "delta-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "delta-b",
    );

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should succeed");
    let payload = serde_json::to_value(&report).expect("serialize index report");
    let evidence = payload
        .pointer("/families/0/evidence")
        .and_then(Value::as_object)
        .expect("index report should expose one family evidence object");
    assert_eq!(
        evidence
            .get("delta_candidate_count")
            .and_then(Value::as_u64)
            .expect("evidence should include delta candidate count"),
        2
    );
    let changed_surfaces = evidence
        .get("changed_surfaces")
        .and_then(Value::as_array)
        .expect("evidence should include changed surfaces");
    assert!(
        !changed_surfaces.is_empty(),
        "delta evidence digest should list at least one changed surface"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_ignores_symlinked_directories_during_scan() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-symlink");
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

    let external_root = unique_temp_dir("loongclaw-runtime-capability-index-symlink-external");
    fs::create_dir_all(&external_root).expect("create external fixture root");
    fs::write(
        external_root.join("runtime-capability-bad.json"),
        r#"{
  "schema": {
    "version": 1,
    "surface": "runtime_capability",
    "purpose": "promotion_candidate_record"
  }
}"#,
    )
    .expect("write malformed supported artifact");

    let symlink_path = root.join("artifacts/external-scan");
    if !try_symlink_dir(&external_root, &symlink_path) {
        fs::remove_dir_all(&external_root).ok();
        fs::remove_dir_all(&root).ok();
        return;
    }

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should skip symlinked directories");

    assert_eq!(report.total_candidate_count, 2);
    assert_eq!(report.family_count, 1);

    fs::remove_dir_all(&external_root).ok();
    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_normalizes_family_ids_for_equivalent_persisted_proposals() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-family-id-normalization");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "canonical-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "canonical-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (candidate_a_path, _) =
        propose_runtime_capability_variant(&root, &run_a_path, "canonical-a");
    let (candidate_b_path, _) =
        propose_runtime_capability_variant(&root, &run_b_path, "canonical-b");
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "canonical-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "canonical-b",
    );

    rewrite_runtime_capability_proposal(
        &candidate_b_path,
        "  Codify browser preview onboarding as a reusable managed skill  ",
        "  Browser preview onboarding and companion readiness checks only  ",
        &[" memory_read ", "invoke_tool"],
        &[" onboarding ", "browser"],
    );

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability index should normalize equivalent persisted proposals");

    assert_eq!(report.total_candidate_count, 2);
    assert_eq!(
        report.family_count, 1,
        "equivalent persisted proposals should stay in one family"
    );
    assert_eq!(
        report
            .families
            .first()
            .expect("one capability family should be reported")
            .candidate_ids
            .len(),
        2
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
fn runtime_capability_index_marks_memory_stage_profile_not_ready_without_memory_delta_evidence() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-memory-stage-profile-not-ready");
    let config_path = write_runtime_capability_config(&root);
    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path = root.join("artifacts/runtime-capability-memory-stage-profile-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-memory-stage-profile-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-b",
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
            check.dimension == "memory_delta_evidence"
                && check.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence
        }),
        "memory-stage-profile families should require memory/context delta evidence"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_uses_accepted_memory_delta_evidence_only() {
    let root =
        unique_temp_dir("loongclaw-runtime-capability-index-memory-stage-profile-accepted-only");
    let config_path = write_runtime_capability_config(&root);
    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-accepted",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-rejected",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-accepted.json");
    let candidate_b_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-rejected.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-accepted",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-rejected",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-accepted",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Rejected,
        "memory-stage-profile-rejected",
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
            check.dimension == "memory_delta_evidence"
                && check.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence
        }),
        "memory delta readiness should ignore rejected-only delta evidence"
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
        normalized_path_text(&error).contains(&artifact_path_suffix(&invalid_candidate_path)),
        "error should identify the malformed artifact path: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_index_rejects_wrong_schema_purpose_during_scan() {
    let root = unique_temp_dir("loongclaw-runtime-capability-index-wrong-purpose");
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
    rewrite_runtime_capability_schema(
        &invalid_candidate_path,
        "runtime_capability",
        "promotion_plan_record",
    );

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
            root: root.join("artifacts").display().to_string(),
            json: false,
        },
    )
    .expect_err("wrong-purpose supported artifacts should abort index scans");

    assert!(
        error.contains("unsupported schema purpose"),
        "error should surface the invalid schema purpose: {error}"
    );
    assert!(
        error.contains("promotion_plan_record"),
        "error should surface the invalid purpose value: {error}"
    );
    assert!(
        normalized_path_text(&error).contains(&artifact_path_suffix(&invalid_candidate_path)),
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
        plan.provenance
            .source_run_artifact_paths
            .contains(&canonicalized_path_text(&run_a_path))
    );
    assert!(
        plan.provenance
            .source_run_artifact_paths
            .contains(&canonicalized_path_text(&run_b_path))
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_surfaces_delta_evidence_digest() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-delta-digest");
    let (run_a_path, _) = finish_runtime_experiment_variant_with_compare_delta(
        &root,
        "plan-delta-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_compare_delta(
        &root,
        "plan-delta-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let (candidate_a_path, _) =
        propose_runtime_capability_variant(&root, &run_a_path, "plan-delta-a");
    let (candidate_b_path, _) =
        propose_runtime_capability_variant(&root, &run_b_path, "plan-delta-b");
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "plan-delta-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "plan-delta-b",
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
    let payload = serde_json::to_value(&plan).expect("serialize plan");
    let evidence = payload
        .get("evidence")
        .and_then(Value::as_object)
        .expect("plan should include family evidence");
    assert_eq!(
        evidence
            .get("delta_candidate_count")
            .and_then(Value::as_u64)
            .expect("plan evidence should include delta candidate count"),
        2
    );
    let changed_surfaces = evidence
        .get("changed_surfaces")
        .and_then(Value::as_array)
        .expect("plan evidence should include changed surfaces");
    assert!(
        !changed_surfaces.is_empty(),
        "plan evidence should surface at least one changed surface"
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
        normalized_path_text(&error).contains(&artifact_path_suffix(&invalid_candidate_path)),
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
fn runtime_capability_plan_uses_memory_stage_profile_dry_run_artifact_surface() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-memory-stage-profile");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path = root.join("artifacts/runtime-capability-memory-stage-profile-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-memory-stage-profile-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-b",
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
    let payload = serde_json::to_value(&plan).expect("serialize runtime capability plan");

    assert_eq!(
        payload
            .pointer("/planned_artifact/target_kind")
            .and_then(Value::as_str),
        Some("memory_stage_profile")
    );
    assert_eq!(plan.planned_artifact.artifact_kind, "memory_stage_profile");
    assert_eq!(
        plan.planned_artifact.delivery_surface,
        "memory_stage_profiles"
    );
    assert!(
        plan.planned_artifact
            .artifact_id
            .starts_with("memory-stage-profile-"),
        "artifact id should carry the new memory-stage-profile prefix"
    );
    assert!(
        plan.approval_checklist
            .iter()
            .any(|item| item.contains("memory stage profile")),
        "checklist should include the target-specific memory stage profile review item"
    );
    assert!(
        plan.rollback_hints
            .iter()
            .any(|hint| hint.contains("memory_stage_profiles")),
        "rollback hints should mention the memory stage profile delivery surface"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_scopes_memory_stage_profile_payload_provenance_to_accepted_evidence() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-memory-stage-profile-provenance");
    write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path = root.join("artifacts/runtime-capability-memory-stage-profile-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-memory-stage-profile-b.json");
    let candidate_a = propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    let _candidate_b = propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    rewrite_json_file(&candidate_b_path, |payload| {
        let changed_surface_count = payload
            .pointer("/source_run/snapshot_delta/changed_surface_count")
            .and_then(Value::as_u64)
            .expect(
                "candidate fixture should include source_run.snapshot_delta.changed_surface_count",
            );
        *payload
            .pointer_mut("/source_run/snapshot_delta/changed_surface_count")
            .expect(
                "candidate fixture should include source_run.snapshot_delta.changed_surface_count",
            ) = Value::from(changed_surface_count + 1);
        let acp_policy_after = payload
            .pointer_mut("/source_run/snapshot_delta/acp_policy/after")
            .expect("candidate fixture should include source_run.snapshot_delta.acp_policy.after");
        *acp_policy_after = Value::String("rejected-only-policy".to_owned());
    });
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Rejected,
        "memory-stage-profile-b",
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
    let payload = serde_json::to_value(&plan).expect("serialize runtime capability plan");
    let planned_payload = payload
        .pointer("/planned_payload/memory_stage_profile")
        .expect("memory-stage-profile plan should include a promoted payload");
    let family_changed_surfaces = payload
        .pointer("/evidence/changed_surfaces")
        .and_then(Value::as_array)
        .expect("plan should preserve broader report-level changed surfaces")
        .iter()
        .map(|value| value.as_str().expect("changed surface should be a string"))
        .collect::<Vec<_>>();
    assert!(
        family_changed_surfaces.contains(&"acp_policy"),
        "broader report-level evidence should still include rejected family evidence"
    );

    assert_eq!(
        planned_payload
            .pointer("/schema_version")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        planned_payload
            .pointer("/artifact_kind")
            .and_then(Value::as_str),
        Some("memory_stage_profile")
    );
    assert_eq!(
        planned_payload
            .pointer("/profile/id")
            .and_then(Value::as_str),
        Some(plan.planned_artifact.artifact_id.as_str())
    );
    assert_eq!(
        planned_payload
            .pointer("/profile/summary")
            .and_then(Value::as_str),
        Some("Promote governed memory pipeline intent into a reusable profile")
    );
    assert_eq!(
        planned_payload
            .pointer("/profile/review_scope")
            .and_then(Value::as_str),
        Some("Governed memory pipeline promotion intent only")
    );
    let required_capabilities = planned_payload
        .pointer("/profile/required_capabilities")
        .and_then(Value::as_array)
        .expect("payload should include the profile required capabilities")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("required capability should be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(required_capabilities, vec!["memory_read"]);
    let tags = planned_payload
        .pointer("/profile/tags")
        .and_then(Value::as_array)
        .expect("payload should include the profile tags")
        .iter()
        .map(|value| value.as_str().expect("tag should be a string"))
        .collect::<Vec<_>>();
    assert_eq!(tags, vec!["memory", "pipeline"]);
    assert_eq!(
        planned_payload
            .pointer("/provenance/family_id")
            .and_then(Value::as_str),
        Some(family.family_id.as_str())
    );
    let accepted_candidate_ids = planned_payload
        .pointer("/provenance/accepted_candidate_ids")
        .and_then(Value::as_array)
        .expect("payload should include the accepted candidate ids")
        .iter()
        .map(|value| value.as_str().expect("candidate id should be a string"))
        .collect::<Vec<_>>();
    assert_eq!(
        accepted_candidate_ids,
        vec![candidate_a.candidate_id.as_str()]
    );
    let changed_surfaces = planned_payload
        .pointer("/provenance/evidence_digest/changed_surfaces")
        .and_then(Value::as_array)
        .expect("payload should include the compact changed-surfaces digest")
        .iter()
        .map(|value| value.as_str().expect("changed surface should be a string"))
        .collect::<Vec<_>>();
    assert_eq!(
        changed_surfaces,
        vec!["context_engine_compaction", "memory_policy"]
    );
    assert!(
        !changed_surfaces.contains(&"acp_policy"),
        "payload provenance digest should exclude rejected-only changed surfaces"
    );

    let rendered =
        loongclaw_daemon::runtime_capability_cli::render_runtime_capability_promotion_plan_text(
            &plan,
        );
    assert!(
        rendered.contains(&format!(
            "planned_payload=profile_id={}",
            plan.planned_artifact.artifact_id
        )),
        "rendered text should mention the payload compactly when present"
    );
    assert!(
        !rendered.contains("planned_payload=null"),
        "rendered text should omit null planned payload noise"
    );
    assert!(
        !rendered.contains("memory_stage_profile:memory_stage_profile"),
        "rendered text should not repeat the payload discriminator"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_omits_memory_stage_profile_payload_for_other_targets() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-non-memory-payload");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "managed-skill-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "managed-skill-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path = root.join("artifacts/runtime-capability-managed-skill-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-managed-skill-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "managed-skill-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "managed-skill-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "managed-skill-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "managed-skill-b",
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
    let payload = serde_json::to_value(&plan).expect("serialize runtime capability plan");
    let rendered =
        loongclaw_daemon::runtime_capability_cli::render_runtime_capability_promotion_plan_text(
            &plan,
        );

    assert!(
        payload.pointer("/planned_payload").is_some(),
        "planned_payload field should always be present"
    );
    assert!(
        payload
            .pointer("/planned_payload")
            .is_some_and(Value::is_null),
        "non-memory targets should serialize planned_payload as null"
    );
    assert!(
        !rendered.contains("planned_payload="),
        "non-memory targets should not render a planned payload line"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_plan_marks_memory_stage_profile_promotable_with_memory_delta_evidence() {
    let root = unique_temp_dir("loongclaw-runtime-capability-plan-memory-stage-profile-ready");
    let (run_a_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-ready-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-ready-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-ready-a.json");
    let candidate_b_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-ready-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-ready-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-ready-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-ready-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-ready-b",
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
        plan.promotable,
        "memory-stage-profile family should be promotable"
    );
    assert_eq!(
        plan.readiness.status,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessStatus::Ready
    );
    assert!(
        plan.readiness.checks.iter().any(|check| {
            check.dimension == "memory_delta_evidence"
                && check.status
                    == loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityFamilyReadinessCheckStatus::Pass
        }),
        "ready memory-stage-profile family should pass memory delta evidence checks"
    );
    assert!(
        plan.evidence.changed_surfaces.iter().any(|surface| {
            surface == "memory_selected"
                || surface == "memory_policy"
                || surface == "context_engine_selected"
                || surface == "context_engine_compaction"
        }),
        "memory-stage-profile evidence should include memory/context surfaces"
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

#[test]
fn runtime_capability_apply_materializes_memory_stage_profile_artifact() {
    let root = unique_temp_dir("loongclaw-runtime-capability-apply-memory-stage-profile");
    let (run_a_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-apply-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-apply-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-apply-a.json");
    let candidate_b_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-apply-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-apply-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-apply-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-apply-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-apply-b",
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

    let report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
                root: root.join("artifacts").display().to_string(),
                family_id: family.family_id.clone(),
                json: false,
            },
        )
        .expect("runtime capability apply should succeed");

    assert_eq!(
        report.outcome,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyOutcome::Applied
    );
    assert_eq!(
        report.planned_artifact.target_kind,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile
    );
    let output_path = PathBuf::from(&report.output_path);
    let actual_output_path_suffix = artifact_path_suffix(&output_path);
    let expected_output_path_suffix = format!(
        "memory_stage_profiles/{}.json",
        report.planned_artifact.artifact_id
    );
    assert!(
        actual_output_path_suffix == expected_output_path_suffix,
        "apply should materialize the memory-stage-profile artifact under the delivery surface"
    );
    assert!(
        output_path.exists(),
        "apply should persist the output artifact"
    );

    let persisted_payload = serde_json::from_str::<Value>(
        &fs::read_to_string(&output_path).expect("read apply output artifact"),
    )
    .expect("decode apply output artifact");

    assert_eq!(
        persisted_payload
            .pointer("/schema/surface")
            .and_then(Value::as_str),
        Some("memory_stage_profile")
    );
    assert_eq!(
        persisted_payload
            .pointer("/schema/purpose")
            .and_then(Value::as_str),
        Some("runtime_capability_apply_output")
    );
    assert_eq!(
        persisted_payload
            .pointer("/artifact_id")
            .and_then(Value::as_str),
        Some(report.planned_artifact.artifact_id.as_str())
    );
    assert_eq!(
        persisted_payload
            .pointer("/delivery_surface")
            .and_then(Value::as_str),
        Some("memory_stage_profiles")
    );
    assert_eq!(
        persisted_payload
            .pointer("/profile/summary")
            .and_then(Value::as_str),
        Some("Promote governed memory pipeline intent into a reusable profile")
    );

    let reindexed_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_index_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityIndexCommandOptions {
                root: root.join("artifacts").display().to_string(),
                json: false,
            },
        )
        .expect("runtime capability re-index should still succeed");

    assert_eq!(
        reindexed_report.total_candidate_count, 2,
        "materialized apply outputs should not be mistaken for runtime-capability candidates"
    );
    assert_eq!(
        reindexed_report.family_count, 1,
        "materialized apply outputs should stay outside capability-family aggregation"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_apply_rejects_unknown_family_id() {
    let root = unique_temp_dir("loongclaw-runtime-capability-apply-missing-family");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _) = finish_runtime_experiment(&root, &config_path);
    propose_runtime_capability_variant(&root, &run_path, "missing");

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: "missing-family".to_owned(),
            json: false,
        },
    )
    .expect_err("unknown family id should be rejected during apply");

    assert!(
        error.contains("missing-family"),
        "error should name the requested family id: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_apply_rejects_non_promotable_memory_stage_profile_family() {
    let root = unique_temp_dir("loongclaw-runtime-capability-apply-memory-stage-profile-blocked");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-blocked-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "memory-stage-profile-blocked-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-blocked-a.json");
    let candidate_b_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-blocked-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-blocked-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-blocked-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-blocked-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-blocked-b",
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

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect_err("non-promotable memory-stage-profile family should be rejected");

    assert!(
        error.contains("not promotable"),
        "apply should explain why materialization was refused: {error}"
    );
    assert!(
        error.contains("memory_delta_evidence"),
        "apply should surface the missing readiness dimension: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_apply_rejects_unsupported_target_kind() {
    let root = unique_temp_dir("loongclaw-runtime-capability-apply-unsupported-target");
    let config_path = write_runtime_capability_config(&root);

    let (run_a_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "managed-skill-apply-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant(
        &root,
        &config_path,
        "managed-skill-apply-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path = root.join("artifacts/runtime-capability-managed-skill-apply-a.json");
    let candidate_b_path = root.join("artifacts/runtime-capability-managed-skill-apply-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "managed-skill-apply-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "managed-skill-apply-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "managed-skill-apply-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "managed-skill-apply-b",
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

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect_err("unsupported target kinds should be rejected");

    assert!(
        error.contains("memory_stage_profile"),
        "apply should state the only supported target kind: {error}"
    );
    assert!(
        error.contains("managed_skill"),
        "apply should name the unsupported planned target: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_apply_is_idempotent_when_existing_output_matches() {
    let root = unique_temp_dir("loongclaw-runtime-capability-apply-idempotent");
    let (run_a_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-idempotent-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-idempotent-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-idempotent-a.json");
    let candidate_b_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-idempotent-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-idempotent-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-idempotent-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-idempotent-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-idempotent-b",
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

    let first_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
                root: root.join("artifacts").display().to_string(),
                family_id: family.family_id.clone(),
                json: false,
            },
        )
        .expect("first runtime capability apply should succeed");

    let first_output = fs::read_to_string(&first_report.output_path)
        .expect("read first apply output artifact for idempotence check");

    let second_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
                root: root.join("artifacts").display().to_string(),
                family_id: family.family_id.clone(),
                json: false,
            },
        )
        .expect("second runtime capability apply should be idempotent");

    let second_output = fs::read_to_string(&second_report.output_path)
        .expect("read second apply output artifact for idempotence check");

    assert_eq!(
        second_report.outcome,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyOutcome::AlreadyApplied
    );
    assert_eq!(
        first_report.output_path, second_report.output_path,
        "idempotent apply should reuse the same output path"
    );
    assert_eq!(
        first_output, second_output,
        "idempotent apply should not rewrite matching output content"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_apply_rejects_conflicting_existing_output() {
    let root = unique_temp_dir("loongclaw-runtime-capability-apply-conflict");
    let (run_a_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-conflict-a",
        -0.2,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );
    let (run_b_path, _) = finish_runtime_experiment_variant_with_memory_compare_delta(
        &root,
        "memory-stage-profile-conflict-b",
        -0.4,
        &[],
        loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted,
    );

    let candidate_a_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-conflict-a.json");
    let candidate_b_path =
        root.join("artifacts/runtime-capability-memory-stage-profile-conflict-b.json");
    propose_runtime_capability_variant_with_target(
        &root,
        &run_a_path,
        "memory-stage-profile-conflict-a",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    propose_runtime_capability_variant_with_target(
        &root,
        &run_b_path,
        "memory-stage-profile-conflict-b",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::MemoryStageProfile,
        "Promote governed memory pipeline intent into a reusable profile",
        "Governed memory pipeline promotion intent only",
        &["memory_read"],
        &["memory", "pipeline"],
    );
    review_runtime_capability_variant(
        &candidate_a_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-conflict-a",
    );
    review_runtime_capability_variant(
        &candidate_b_path,
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted,
        "memory-stage-profile-conflict-b",
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

    let first_report =
        loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
                root: root.join("artifacts").display().to_string(),
                family_id: family.family_id.clone(),
                json: false,
            },
        )
        .expect("first runtime capability apply should succeed");

    let output_path = PathBuf::from(&first_report.output_path);
    rewrite_json_file(&output_path, |payload| {
        let profile_summary = payload
            .pointer_mut("/profile/summary")
            .expect("apply output should include profile.summary");
        *profile_summary = Value::String("conflicting manual edit".to_owned());
    });

    let error = loongclaw_daemon::runtime_capability_cli::execute_runtime_capability_apply_command(
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityApplyCommandOptions {
            root: root.join("artifacts").display().to_string(),
            family_id: family.family_id.clone(),
            json: false,
        },
    )
    .expect_err("conflicting existing output should be rejected");

    assert!(
        error.contains("different content"),
        "apply should reject conflicting materialized output: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runtime_capability_show_text_renders_snapshot_delta_summary() {
    let root = unique_temp_dir("loongclaw-runtime-capability-show-text-delta-summary");
    let config_path = write_runtime_capability_config(&root);
    let (run_path, _baseline_snapshot_path, _result_snapshot_path, _run) =
        finish_runtime_experiment_with_compare_delta(&root, &config_path);

    let candidate = propose_runtime_capability_variant_with_target(
        &root,
        &run_path,
        "show-delta",
        loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill,
        "Codify browser preview onboarding as a reusable managed skill",
        "Browser preview onboarding and companion readiness checks only",
        &["invoke_tool", "memory_read"],
        &["browser", "onboarding"],
    );

    let rendered =
        loongclaw_daemon::runtime_capability_cli::render_runtime_capability_text(&candidate);
    assert!(
        rendered.contains("source_snapshot_delta_changed_surface_count="),
        "rendered text should include the compact changed-surface count"
    );
    assert!(
        rendered.contains("source_snapshot_delta_changed_surfaces="),
        "rendered text should include compact changed-surface names"
    );

    fs::remove_dir_all(&root).ok();
}
