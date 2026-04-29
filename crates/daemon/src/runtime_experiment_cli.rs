use crate::{
    RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION, RuntimeSnapshotArtifactDocument,
    sha2::{self, Digest},
};
use clap::{Args, Subcommand, ValueEnum};
use loong_spec::CliResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const RUNTIME_EXPERIMENT_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum RuntimeExperimentCommands {
    /// Create a new experiment-run artifact from a baseline runtime snapshot
    Start(RuntimeExperimentStartCommandOptions),
    /// Attach result snapshot and evaluation details to an experiment run
    Finish(RuntimeExperimentFinishCommandOptions),
    /// Load and render one persisted experiment-run artifact
    Show(RuntimeExperimentShowCommandOptions),
    /// Compare one experiment run and, optionally, matching runtime snapshots
    Compare(RuntimeExperimentCompareCommandOptions),
    /// Restore one recorded baseline/result stage through the snapshot restore pipeline
    Restore(RuntimeExperimentRestoreCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExperimentStartCommandOptions {
    #[arg(long)]
    pub snapshot: String,
    #[arg(long)]
    pub output: String,
    #[arg(long)]
    pub mutation_summary: String,
    #[arg(long)]
    pub experiment_id: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long = "tag")]
    pub tag: Vec<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExperimentFinishCommandOptions {
    #[arg(long)]
    pub run: String,
    #[arg(long)]
    pub result_snapshot: String,
    #[arg(long)]
    pub evaluation_summary: String,
    #[arg(long = "metric")]
    pub metric: Vec<String>,
    #[arg(long = "warning")]
    pub warning: Vec<String>,
    #[arg(long, value_enum)]
    pub decision: RuntimeExperimentDecision,
    #[arg(long, value_enum, default_value_t = RuntimeExperimentFinishStatus::Completed)]
    pub status: RuntimeExperimentFinishStatus,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExperimentShowCommandOptions {
    #[arg(long)]
    pub run: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExperimentCompareCommandOptions {
    #[arg(long)]
    pub run: String,
    #[arg(long)]
    pub baseline_snapshot: Option<String>,
    #[arg(long)]
    pub result_snapshot: Option<String>,
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["baseline_snapshot", "result_snapshot"]
    )]
    pub recorded_snapshots: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeExperimentRestoreCommandOptions {
    #[arg(long)]
    pub run: String,
    #[arg(long, value_enum)]
    pub stage: RuntimeExperimentRestoreStage,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
    #[arg(long, default_value_t = false)]
    pub apply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExperimentDecision {
    Undecided,
    Promoted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExperimentStatus {
    Planned,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RuntimeExperimentFinishStatus {
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExperimentCompareMode {
    RecordOnly,
    SnapshotDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExperimentRestoreStage {
    Baseline,
    Result,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeExperimentArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeExperimentMutationSummary {
    pub summary: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeExperimentSnapshotSummary {
    pub snapshot_id: String,
    pub created_at: String,
    pub label: Option<String>,
    pub experiment_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub artifact_path: Option<String>,
    pub capability_snapshot_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeExperimentEvaluation {
    pub summary: String,
    pub metrics: BTreeMap<String, f64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeExperimentArtifactDocument {
    pub schema: RuntimeExperimentArtifactSchema,
    pub run_id: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub label: Option<String>,
    pub experiment_id: String,
    pub status: RuntimeExperimentStatus,
    pub decision: RuntimeExperimentDecision,
    pub mutation: RuntimeExperimentMutationSummary,
    pub baseline_snapshot: RuntimeExperimentSnapshotSummary,
    pub result_snapshot: Option<RuntimeExperimentSnapshotSummary>,
    pub evaluation: Option<RuntimeExperimentEvaluation>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeExperimentCompareReport {
    pub run_id: String,
    pub experiment_id: String,
    pub label: Option<String>,
    pub status: RuntimeExperimentStatus,
    pub decision: RuntimeExperimentDecision,
    pub mutation: RuntimeExperimentMutationSummary,
    pub baseline_snapshot: RuntimeExperimentSnapshotSummary,
    pub result_snapshot: Option<RuntimeExperimentSnapshotSummary>,
    pub evaluation: Option<RuntimeExperimentEvaluation>,
    pub compare_mode: RuntimeExperimentCompareMode,
    pub snapshot_delta: Option<RuntimeExperimentSnapshotDelta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeExperimentRestoreExecution {
    pub run_id: String,
    pub experiment_id: String,
    pub stage: RuntimeExperimentRestoreStage,
    pub snapshot_path: String,
    pub restore: crate::runtime_restore_cli::RuntimeRestoreExecution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeExperimentSnapshotDelta {
    pub changed_surface_count: usize,
    pub provider_active_profile: RuntimeExperimentScalarCompare,
    pub provider_active_model: RuntimeExperimentScalarCompare,
    pub context_engine_selected: RuntimeExperimentScalarCompare,
    pub context_engine_compaction: RuntimeExperimentScalarCompare,
    pub memory_selected: RuntimeExperimentScalarCompare,
    pub memory_policy: RuntimeExperimentScalarCompare,
    pub acp_selected: RuntimeExperimentScalarCompare,
    pub acp_policy: RuntimeExperimentScalarCompare,
    pub enabled_channel_ids: RuntimeExperimentSetCompare,
    pub enabled_runtime_backed_channel_ids: RuntimeExperimentSetCompare,
    pub enabled_service_channel_ids: RuntimeExperimentSetCompare,
    pub enabled_plugin_backed_channel_ids: RuntimeExperimentSetCompare,
    pub enabled_outbound_only_channel_ids: RuntimeExperimentSetCompare,
    pub visible_tool_names: RuntimeExperimentSetCompare,
    pub capability_snapshot_sha256: RuntimeExperimentScalarCompare,
    pub external_skill_ids: RuntimeExperimentSetCompare,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeExperimentScalarCompare {
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeExperimentSetCompare {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl RuntimeExperimentScalarCompare {
    fn changed(&self) -> bool {
        self.before != self.after
    }
}

impl RuntimeExperimentSetCompare {
    fn changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

impl RuntimeExperimentSnapshotDelta {
    pub(crate) fn changed_surfaces(&self) -> Vec<String> {
        let mut surfaces = Vec::new();
        if self.provider_active_profile.changed() {
            surfaces.push("provider_active_profile".to_owned());
        }
        if self.provider_active_model.changed() {
            surfaces.push("provider_active_model".to_owned());
        }
        if self.context_engine_selected.changed() {
            surfaces.push("context_engine_selected".to_owned());
        }
        if self.context_engine_compaction.changed() {
            surfaces.push("context_engine_compaction".to_owned());
        }
        if self.memory_selected.changed() {
            surfaces.push("memory_selected".to_owned());
        }
        if self.memory_policy.changed() {
            surfaces.push("memory_policy".to_owned());
        }
        if self.acp_selected.changed() {
            surfaces.push("acp_selected".to_owned());
        }
        if self.acp_policy.changed() {
            surfaces.push("acp_policy".to_owned());
        }
        if self.enabled_channel_ids.changed() {
            surfaces.push("enabled_channel_ids".to_owned());
        }
        if self.enabled_runtime_backed_channel_ids.changed() {
            surfaces.push("enabled_runtime_backed_channel_ids".to_owned());
        }
        if self.enabled_service_channel_ids.changed() {
            surfaces.push("enabled_service_channel_ids".to_owned());
        }
        if self.enabled_plugin_backed_channel_ids.changed() {
            surfaces.push("enabled_plugin_backed_channel_ids".to_owned());
        }
        if self.enabled_outbound_only_channel_ids.changed() {
            surfaces.push("enabled_outbound_only_channel_ids".to_owned());
        }
        if self.visible_tool_names.changed() {
            surfaces.push("visible_tool_names".to_owned());
        }
        if self.capability_snapshot_sha256.changed() {
            surfaces.push("capability_snapshot_sha256".to_owned());
        }
        if self.external_skill_ids.changed() {
            surfaces.push("external_skill_ids".to_owned());
        }
        surfaces
    }
}

pub fn run_runtime_experiment_cli(command: RuntimeExperimentCommands) -> CliResult<()> {
    match command {
        RuntimeExperimentCommands::Start(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_experiment_start_command(options)?;
            emit_runtime_experiment_artifact(&artifact, as_json)
        }
        RuntimeExperimentCommands::Finish(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_experiment_finish_command(options)?;
            emit_runtime_experiment_artifact(&artifact, as_json)
        }
        RuntimeExperimentCommands::Show(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_experiment_show_command(options)?;
            emit_runtime_experiment_artifact(&artifact, as_json)
        }
        RuntimeExperimentCommands::Compare(options) => {
            let as_json = options.json;
            let report = execute_runtime_experiment_compare_command(options)?;
            emit_runtime_experiment_compare_report(&report, as_json)
        }
        RuntimeExperimentCommands::Restore(options) => {
            let as_json = options.json;
            let execution = execute_runtime_experiment_restore_command(options)?;
            emit_runtime_experiment_restore_execution(&execution, as_json)
        }
    }
}

pub fn execute_runtime_experiment_start_command(
    options: RuntimeExperimentStartCommandOptions,
) -> CliResult<RuntimeExperimentArtifactDocument> {
    let baseline = load_runtime_snapshot_artifact(Path::new(&options.snapshot))?;
    let experiment_id = resolve_experiment_id(
        options.experiment_id.as_deref(),
        baseline.lineage.experiment_id.as_deref(),
    )?;
    let created_at = now_rfc3339()?;
    let label = optional_arg(options.label.as_deref());
    let mutation_summary = required_trimmed_arg("mutation_summary", &options.mutation_summary)?;
    let tags = normalize_repeated_values(&options.tag);
    let baseline_snapshot = build_snapshot_summary(&baseline, Some(Path::new(&options.snapshot)))?;
    let run_id = compute_run_id(
        &created_at,
        label.as_deref(),
        &experiment_id,
        &baseline_snapshot,
        &mutation_summary,
        &tags,
    )?;
    let artifact = RuntimeExperimentArtifactDocument {
        schema: RuntimeExperimentArtifactSchema {
            version: RUNTIME_EXPERIMENT_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: "runtime_experiment".to_owned(),
            purpose: "snapshot_evaluation_record".to_owned(),
        },
        run_id,
        created_at,
        finished_at: None,
        label,
        experiment_id,
        status: RuntimeExperimentStatus::Planned,
        decision: RuntimeExperimentDecision::Undecided,
        mutation: RuntimeExperimentMutationSummary {
            summary: mutation_summary,
            tags,
        },
        baseline_snapshot,
        result_snapshot: None,
        evaluation: None,
    };
    persist_runtime_experiment_artifact(&options.output, &artifact)?;
    Ok(artifact)
}

pub fn execute_runtime_experiment_finish_command(
    options: RuntimeExperimentFinishCommandOptions,
) -> CliResult<RuntimeExperimentArtifactDocument> {
    let mut artifact = load_runtime_experiment_artifact(Path::new(&options.run))?;
    if artifact.status != RuntimeExperimentStatus::Planned {
        return Err(format!(
            "runtime experiment run {} is already {}",
            options.run,
            render_status(artifact.status)
        ));
    }

    let result_snapshot = load_runtime_snapshot_artifact(Path::new(&options.result_snapshot))?;
    let result_experiment_id = optional_arg(result_snapshot.lineage.experiment_id.as_deref());
    if let Some(result_experiment_id) = result_experiment_id.as_deref()
        && result_experiment_id != artifact.experiment_id
    {
        return Err(format!(
            "runtime experiment result snapshot experiment_id `{result_experiment_id}` does not match run experiment_id `{}`",
            artifact.experiment_id
        ));
    }

    let mut warnings = normalize_warnings(&options.warning);
    if result_experiment_id.is_none() {
        warnings.push(format!(
            "result snapshot {} is missing experiment_id; operator-confirmed lineage is required",
            options.result_snapshot
        ));
    }

    artifact.finished_at = Some(now_rfc3339()?);
    artifact.status = match options.status {
        RuntimeExperimentFinishStatus::Completed => RuntimeExperimentStatus::Completed,
        RuntimeExperimentFinishStatus::Aborted => RuntimeExperimentStatus::Aborted,
    };
    artifact.decision = options.decision;
    artifact.result_snapshot = Some(build_snapshot_summary(
        &result_snapshot,
        Some(Path::new(&options.result_snapshot)),
    )?);
    artifact.evaluation = Some(RuntimeExperimentEvaluation {
        summary: required_trimmed_arg("evaluation_summary", &options.evaluation_summary)?,
        metrics: parse_metrics(&options.metric)?,
        warnings,
    });
    persist_runtime_experiment_artifact(&options.run, &artifact)?;
    Ok(artifact)
}

pub fn execute_runtime_experiment_show_command(
    options: RuntimeExperimentShowCommandOptions,
) -> CliResult<RuntimeExperimentArtifactDocument> {
    load_runtime_experiment_artifact(Path::new(&options.run))
}

pub fn execute_runtime_experiment_compare_command(
    options: RuntimeExperimentCompareCommandOptions,
) -> CliResult<RuntimeExperimentCompareReport> {
    let artifact = load_runtime_experiment_artifact(Path::new(&options.run))?;
    let snapshot_delta = load_runtime_experiment_compare_snapshot_delta(
        &artifact,
        &options.run,
        options.baseline_snapshot.as_deref(),
        options.result_snapshot.as_deref(),
        options.recorded_snapshots,
    )?;
    let compare_mode = if snapshot_delta.is_some() {
        RuntimeExperimentCompareMode::SnapshotDelta
    } else {
        RuntimeExperimentCompareMode::RecordOnly
    };

    Ok(RuntimeExperimentCompareReport {
        run_id: artifact.run_id,
        experiment_id: artifact.experiment_id,
        label: artifact.label,
        status: artifact.status,
        decision: artifact.decision,
        mutation: artifact.mutation,
        baseline_snapshot: artifact.baseline_snapshot,
        result_snapshot: artifact.result_snapshot,
        evaluation: artifact.evaluation,
        compare_mode,
        snapshot_delta,
    })
}

pub(crate) fn derive_recorded_snapshot_delta_for_run(
    artifact: &RuntimeExperimentArtifactDocument,
    run_path: &str,
) -> CliResult<Option<RuntimeExperimentSnapshotDelta>> {
    let Some(result_snapshot) = artifact.result_snapshot.as_ref() else {
        return Ok(None);
    };
    if artifact.baseline_snapshot.artifact_path.is_none() || result_snapshot.artifact_path.is_none()
    {
        return Ok(None);
    }
    load_runtime_experiment_compare_snapshot_delta(artifact, run_path, None, None, true)
}

pub fn execute_runtime_experiment_restore_command(
    options: RuntimeExperimentRestoreCommandOptions,
) -> CliResult<RuntimeExperimentRestoreExecution> {
    let RuntimeExperimentRestoreCommandOptions {
        run,
        stage,
        config,
        json: _,
        apply,
    } = options;
    let artifact = load_runtime_experiment_artifact(Path::new(&run))?;
    let snapshot_path =
        resolve_runtime_experiment_restore_snapshot_path(&artifact, Path::new(&run), stage)?;
    let restore = crate::runtime_restore_cli::execute_runtime_restore_command(
        crate::runtime_restore_cli::RuntimeRestoreCommandOptions {
            config,
            snapshot: snapshot_path.clone(),
            json: false,
            apply,
        },
    )?;

    Ok(RuntimeExperimentRestoreExecution {
        run_id: artifact.run_id,
        experiment_id: artifact.experiment_id,
        stage,
        snapshot_path,
        restore,
    })
}

fn emit_runtime_experiment_artifact(
    artifact: &RuntimeExperimentArtifactDocument,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(artifact)
            .map_err(|error| format!("serialize runtime experiment artifact failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_experiment_text(artifact));
    Ok(())
}

fn emit_runtime_experiment_compare_report(
    report: &RuntimeExperimentCompareReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime experiment compare report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_experiment_compare_text(report));
    Ok(())
}

fn emit_runtime_experiment_restore_execution(
    execution: &RuntimeExperimentRestoreExecution,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(execution).map_err(|error| {
            format!("serialize runtime experiment restore execution failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_experiment_restore_text(execution));
    Ok(())
}

fn load_runtime_snapshot_artifact(path: &Path) -> CliResult<RuntimeSnapshotArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime snapshot artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<RuntimeSnapshotArtifactDocument>(&raw).map_err(|error| {
            format!(
                "decode runtime snapshot artifact {} failed: {error}",
                path.display()
            )
        })?;
    if artifact.schema.version != RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime snapshot artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            artifact.schema.version,
            RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    Ok(artifact)
}

fn load_runtime_experiment_artifact(path: &Path) -> CliResult<RuntimeExperimentArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime experiment artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<RuntimeExperimentArtifactDocument>(&raw).map_err(|error| {
            format!(
                "decode runtime experiment artifact {} failed: {error}",
                path.display()
            )
        })?;
    if artifact.schema.version != RUNTIME_EXPERIMENT_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime experiment artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            artifact.schema.version,
            RUNTIME_EXPERIMENT_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    Ok(artifact)
}

fn load_runtime_experiment_compare_snapshot_delta(
    artifact: &RuntimeExperimentArtifactDocument,
    run_path: &str,
    baseline_snapshot_path: Option<&str>,
    result_snapshot_path: Option<&str>,
    recorded_snapshots: bool,
) -> CliResult<Option<RuntimeExperimentSnapshotDelta>> {
    match (baseline_snapshot_path, result_snapshot_path, recorded_snapshots) {
        (Some(_), _, true) | (_, Some(_), true) => Err(
            "runtime experiment compare cannot combine --recorded-snapshots with manual snapshot paths"
                .to_owned(),
        ),
        (None, None, false) => Ok(None),
        (None, None, true) => {
            let run_path = Path::new(run_path);
            let recorded_result_snapshot = artifact.result_snapshot.as_ref().ok_or_else(|| {
                format!(
                    "runtime experiment compare cannot load recorded snapshots for run {} because the run has no result snapshot",
                    run_path.display()
                )
            })?;
            let baseline_snapshot_path = resolve_runtime_experiment_compare_snapshot_path(
                &artifact.baseline_snapshot,
                run_path,
                "baseline",
            )?;
            let result_snapshot_path = resolve_runtime_experiment_compare_snapshot_path(
                recorded_result_snapshot,
                run_path,
                "result",
            )?;
            let baseline_snapshot =
                load_runtime_snapshot_artifact(Path::new(&baseline_snapshot_path))?;
            let result_snapshot = load_runtime_snapshot_artifact(Path::new(&result_snapshot_path))?;
            validate_compare_snapshot_identity(
                "baseline",
                &artifact.baseline_snapshot.snapshot_id,
                &baseline_snapshot.lineage.snapshot_id,
                Path::new(&baseline_snapshot_path),
            )?;
            validate_compare_snapshot_identity(
                "result",
                &recorded_result_snapshot.snapshot_id,
                &result_snapshot.lineage.snapshot_id,
                Path::new(&result_snapshot_path),
            )?;
            Ok(Some(build_runtime_experiment_snapshot_delta(
                &baseline_snapshot,
                &result_snapshot,
            )))
        }
        (Some(_), None, false) | (None, Some(_), false) => {
            Err("runtime experiment compare requires --baseline-snapshot and --result-snapshot together".to_owned())
        }
        (Some(baseline_snapshot_path), Some(result_snapshot_path), false) => {
            let recorded_result_snapshot = artifact.result_snapshot.as_ref().ok_or_else(|| {
                format!(
                    "runtime experiment compare cannot load snapshot delta for run {} because the run has no result snapshot",
                    run_path
                )
            })?;
            let baseline_snapshot =
                load_runtime_snapshot_artifact(Path::new(baseline_snapshot_path))?;
            let result_snapshot = load_runtime_snapshot_artifact(Path::new(result_snapshot_path))?;
            validate_compare_snapshot_identity(
                "baseline",
                &artifact.baseline_snapshot.snapshot_id,
                &baseline_snapshot.lineage.snapshot_id,
                Path::new(baseline_snapshot_path),
            )?;
            validate_compare_snapshot_identity(
                "result",
                &recorded_result_snapshot.snapshot_id,
                &result_snapshot.lineage.snapshot_id,
                Path::new(result_snapshot_path),
            )?;
            Ok(Some(build_runtime_experiment_snapshot_delta(
                &baseline_snapshot,
                &result_snapshot,
            )))
        }
    }
}

fn resolve_runtime_experiment_compare_snapshot_path(
    summary: &RuntimeExperimentSnapshotSummary,
    run_path: &Path,
    stage: &str,
) -> CliResult<String> {
    let artifact_path = summary.artifact_path.as_deref().ok_or_else(|| {
        format!(
            "runtime experiment run {} is missing recorded {} snapshot path",
            run_path.display(),
            stage
        )
    })?;

    canonicalize_snapshot_artifact_path(artifact_path).map_err(|error| {
        format!(
            "runtime experiment run {} recorded {} snapshot path {} cannot be resolved: {error}",
            run_path.display(),
            stage,
            artifact_path
        )
    })
}

fn resolve_runtime_experiment_restore_snapshot_path(
    artifact: &RuntimeExperimentArtifactDocument,
    run_path: &Path,
    stage: RuntimeExperimentRestoreStage,
) -> CliResult<String> {
    let summary = match stage {
        RuntimeExperimentRestoreStage::Baseline => &artifact.baseline_snapshot,
        RuntimeExperimentRestoreStage::Result => {
            artifact.result_snapshot.as_ref().ok_or_else(|| {
                format!(
                    "runtime experiment run {} has no recorded result snapshot to restore",
                    run_path.display()
                )
            })?
        }
    };
    let artifact_path = summary.artifact_path.as_deref().ok_or_else(|| {
        format!(
            "runtime experiment run {} is missing recorded {} snapshot path",
            run_path.display(),
            stage.as_str()
        )
    })?;

    canonicalize_snapshot_artifact_path(artifact_path).map_err(|error| {
        format!(
            "runtime experiment run {} recorded {} snapshot path {} cannot be resolved: {error}",
            run_path.display(),
            stage.as_str(),
            artifact_path
        )
    })
}

fn validate_compare_snapshot_identity(
    kind: &str,
    expected_snapshot_id: &str,
    actual_snapshot_id: &str,
    path: &Path,
) -> CliResult<()> {
    if expected_snapshot_id == actual_snapshot_id {
        return Ok(());
    }
    Err(format!(
        "runtime experiment compare {kind} snapshot {} has snapshot_id `{actual_snapshot_id}` but run recorded `{expected_snapshot_id}`",
        path.display()
    ))
}

fn resolve_experiment_id(explicit: Option<&str>, baseline: Option<&str>) -> CliResult<String> {
    let explicit = optional_arg(explicit);
    let baseline = optional_arg(baseline);
    match (explicit, baseline) {
        (Some(explicit), Some(baseline)) if explicit != baseline => Err(format!(
            "runtime experiment start --experiment-id `{explicit}` does not match baseline snapshot experiment_id `{baseline}`"
        )),
        (Some(explicit), _) => Ok(explicit),
        (None, Some(baseline)) => Ok(baseline),
        (None, None) => Err(
            "runtime experiment start requires --experiment-id when the baseline snapshot artifact does not declare experiment_id".to_owned(),
        ),
    }
}

fn now_rfc3339() -> CliResult<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format runtime experiment timestamp failed: {error}"))
}

fn optional_arg(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_trimmed_arg(name: &str, raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("runtime experiment {name} cannot be empty"));
    }
    Ok(trimmed.to_owned())
}

fn normalize_repeated_values(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn normalize_warnings(values: &[String]) -> Vec<String> {
    normalize_repeated_values(values)
}

fn parse_metrics(values: &[String]) -> CliResult<BTreeMap<String, f64>> {
    let mut metrics = BTreeMap::new();
    for raw in values {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("runtime experiment metric entries cannot be empty".to_owned());
        }
        let (key, value) = trimmed.split_once('=').ok_or_else(|| {
            format!("runtime experiment metric `{trimmed}` must use key=value syntax")
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "runtime experiment metric `{trimmed}` is missing a metric name"
            ));
        }
        let value = value.trim().parse::<f64>().map_err(|error| {
            format!("runtime experiment metric `{trimmed}` must be numeric: {error}")
        })?;
        if metrics.insert(key.to_owned(), value).is_some() {
            return Err(format!(
                "runtime experiment metric `{key}` was provided more than once"
            ));
        }
    }
    Ok(metrics)
}

fn build_snapshot_summary(
    artifact: &RuntimeSnapshotArtifactDocument,
    artifact_path: Option<&Path>,
) -> CliResult<RuntimeExperimentSnapshotSummary> {
    Ok(RuntimeExperimentSnapshotSummary {
        snapshot_id: artifact.lineage.snapshot_id.clone(),
        created_at: artifact.lineage.created_at.clone(),
        label: artifact.lineage.label.clone(),
        experiment_id: artifact.lineage.experiment_id.clone(),
        parent_snapshot_id: artifact.lineage.parent_snapshot_id.clone(),
        artifact_path: artifact_path
            .map(|path| canonicalize_snapshot_artifact_path(&path.display().to_string()))
            .transpose()?,
        capability_snapshot_sha256: artifact
            .tools
            .get("capability_snapshot_sha256")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn canonicalize_snapshot_artifact_path(path: &str) -> CliResult<String> {
    dunce::canonicalize(path)
        .map(|resolved| resolved.display().to_string())
        .map_err(|error| {
            format!(
                "canonicalize snapshot artifact path {} failed: {error}",
                path
            )
        })
}

fn build_runtime_experiment_snapshot_delta(
    baseline: &RuntimeSnapshotArtifactDocument,
    result: &RuntimeSnapshotArtifactDocument,
) -> RuntimeExperimentSnapshotDelta {
    let provider_active_profile = compare_optional_strings(
        snapshot_provider_active_profile_id(baseline),
        snapshot_provider_active_profile_id(result),
    );
    let provider_active_model = compare_optional_strings(
        snapshot_provider_active_model(baseline),
        snapshot_provider_active_model(result),
    );
    let context_engine_selected = compare_optional_strings(
        snapshot_context_engine_selected_id(baseline),
        snapshot_context_engine_selected_id(result),
    );
    let context_engine_compaction = compare_optional_strings(
        snapshot_context_engine_compaction_summary(baseline),
        snapshot_context_engine_compaction_summary(result),
    );
    let memory_selected = compare_optional_strings(
        snapshot_memory_selected_id(baseline),
        snapshot_memory_selected_id(result),
    );
    let memory_policy = compare_optional_strings(
        snapshot_memory_policy_summary(baseline),
        snapshot_memory_policy_summary(result),
    );
    let acp_selected = compare_optional_strings(
        snapshot_acp_selected_id(baseline),
        snapshot_acp_selected_id(result),
    );
    let acp_policy = compare_optional_strings(
        snapshot_acp_policy_summary(baseline),
        snapshot_acp_policy_summary(result),
    );
    let enabled_channel_ids = compare_string_sets(
        snapshot_enabled_channel_ids(baseline),
        snapshot_enabled_channel_ids(result),
    );
    let enabled_runtime_backed_channel_ids = compare_string_sets(
        snapshot_enabled_runtime_backed_channel_ids(baseline),
        snapshot_enabled_runtime_backed_channel_ids(result),
    );
    let enabled_service_channel_ids = compare_string_sets(
        snapshot_enabled_service_channel_ids(baseline),
        snapshot_enabled_service_channel_ids(result),
    );
    let enabled_plugin_backed_channel_ids = compare_string_sets(
        snapshot_enabled_plugin_backed_channel_ids(baseline),
        snapshot_enabled_plugin_backed_channel_ids(result),
    );
    let enabled_outbound_only_channel_ids = compare_string_sets(
        snapshot_enabled_outbound_only_channel_ids(baseline),
        snapshot_enabled_outbound_only_channel_ids(result),
    );
    let visible_tool_names = compare_string_sets(
        snapshot_visible_tool_names(baseline),
        snapshot_visible_tool_names(result),
    );
    let capability_snapshot_sha256 = compare_optional_strings(
        snapshot_capability_snapshot_sha256(baseline),
        snapshot_capability_snapshot_sha256(result),
    );
    let external_skill_ids = compare_string_sets(
        snapshot_external_skill_ids(baseline),
        snapshot_external_skill_ids(result),
    );
    let changed_surface_count = usize::from(provider_active_profile.changed())
        + usize::from(provider_active_model.changed())
        + usize::from(context_engine_selected.changed())
        + usize::from(context_engine_compaction.changed())
        + usize::from(memory_selected.changed())
        + usize::from(memory_policy.changed())
        + usize::from(acp_selected.changed())
        + usize::from(acp_policy.changed())
        + usize::from(enabled_channel_ids.changed())
        + usize::from(enabled_runtime_backed_channel_ids.changed())
        + usize::from(enabled_service_channel_ids.changed())
        + usize::from(enabled_plugin_backed_channel_ids.changed())
        + usize::from(enabled_outbound_only_channel_ids.changed())
        + usize::from(visible_tool_names.changed())
        + usize::from(capability_snapshot_sha256.changed())
        + usize::from(external_skill_ids.changed());

    RuntimeExperimentSnapshotDelta {
        changed_surface_count,
        provider_active_profile,
        provider_active_model,
        context_engine_selected,
        context_engine_compaction,
        memory_selected,
        memory_policy,
        acp_selected,
        acp_policy,
        enabled_channel_ids,
        enabled_runtime_backed_channel_ids,
        enabled_service_channel_ids,
        enabled_plugin_backed_channel_ids,
        enabled_outbound_only_channel_ids,
        visible_tool_names,
        capability_snapshot_sha256,
        external_skill_ids,
    }
}

fn compare_optional_strings(
    before: Option<String>,
    after: Option<String>,
) -> RuntimeExperimentScalarCompare {
    RuntimeExperimentScalarCompare { before, after }
}

fn compare_string_sets(before: Vec<String>, after: Vec<String>) -> RuntimeExperimentSetCompare {
    let before = before.into_iter().collect::<BTreeSet<_>>();
    let after = after.into_iter().collect::<BTreeSet<_>>();
    RuntimeExperimentSetCompare {
        added: after.difference(&before).cloned().collect(),
        removed: before.difference(&after).cloned().collect(),
    }
}

fn snapshot_provider_active_profile_id(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Option<String> {
    json_string_path(&snapshot.provider, &["active_profile_id"])
}

fn snapshot_provider_active_model(snapshot: &RuntimeSnapshotArtifactDocument) -> Option<String> {
    let active_profile_id = snapshot_provider_active_profile_id(snapshot)?;
    snapshot
        .provider
        .get("profiles")
        .and_then(Value::as_array)?
        .iter()
        .find(|profile| {
            profile
                .get("profile_id")
                .and_then(Value::as_str)
                .is_some_and(|profile_id| profile_id == active_profile_id)
        })
        .and_then(|profile| profile.get("model"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn snapshot_context_engine_selected_id(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Option<String> {
    json_string_path(&snapshot.context_engine, &["selected", "id"])
}

fn snapshot_context_engine_compaction_summary(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Option<String> {
    json_value_path(&snapshot.context_engine, &["compaction"])
        .and_then(Value::as_object)
        .filter(|compaction| !compaction.is_empty())?;
    let enabled = render_optional_bool(json_bool_path(
        &snapshot.context_engine,
        &["compaction", "enabled"],
    ));
    let min_messages = render_optional_u64(json_u64_path(
        &snapshot.context_engine,
        &["compaction", "min_messages"],
    ));
    let trigger_estimated_tokens = render_optional_u64(json_u64_path(
        &snapshot.context_engine,
        &["compaction", "trigger_estimated_tokens"],
    ));
    let preserve_recent_estimated_tokens = render_optional_u64(json_u64_path(
        &snapshot.context_engine,
        &["compaction", "preserve_recent_estimated_tokens"],
    ));
    let fail_open = render_optional_bool(json_bool_path(
        &snapshot.context_engine,
        &["compaction", "fail_open"],
    ));
    Some(format!(
        "enabled:{enabled} min_messages:{min_messages} trigger_estimated_tokens:{trigger_estimated_tokens} preserve_recent_estimated_tokens:{preserve_recent_estimated_tokens} fail_open:{fail_open}"
    ))
}

fn snapshot_memory_selected_id(snapshot: &RuntimeSnapshotArtifactDocument) -> Option<String> {
    json_string_path(&snapshot.memory_system, &["selected", "id"])
}

fn snapshot_memory_policy_summary(snapshot: &RuntimeSnapshotArtifactDocument) -> Option<String> {
    json_value_path(&snapshot.memory_system, &["policy"])
        .and_then(Value::as_object)
        .filter(|policy| !policy.is_empty())?;
    Some(format!(
        "backend:{} profile:{} mode:{} ingest_mode:{} fail_open:{} strict_mode_requested:{} strict_mode_active:{} effective_fail_open:{}",
        json_string_path(&snapshot.memory_system, &["policy", "backend"])
            .unwrap_or_else(|| "-".to_owned()),
        json_string_path(&snapshot.memory_system, &["policy", "profile"])
            .unwrap_or_else(|| "-".to_owned()),
        json_string_path(&snapshot.memory_system, &["policy", "mode"])
            .unwrap_or_else(|| "-".to_owned()),
        json_string_path(&snapshot.memory_system, &["policy", "ingest_mode"])
            .unwrap_or_else(|| "-".to_owned()),
        render_optional_bool(json_bool_path(
            &snapshot.memory_system,
            &["policy", "fail_open"]
        )),
        render_optional_bool(json_bool_path(
            &snapshot.memory_system,
            &["policy", "strict_mode_requested"]
        )),
        render_optional_bool(json_bool_path(
            &snapshot.memory_system,
            &["policy", "strict_mode_active"]
        )),
        render_optional_bool(json_bool_path(
            &snapshot.memory_system,
            &["policy", "effective_fail_open"]
        )),
    ))
}

fn snapshot_acp_selected_id(snapshot: &RuntimeSnapshotArtifactDocument) -> Option<String> {
    json_string_path(&snapshot.acp, &["selected", "id"])
}

fn snapshot_acp_policy_summary(snapshot: &RuntimeSnapshotArtifactDocument) -> Option<String> {
    snapshot.acp.as_object().filter(|acp| !acp.is_empty())?;
    Some(format!(
        "enabled:{} dispatch_enabled:{} conversation_routing:{} thread_routing:{} default_agent:{} allowed_agents:{}",
        render_optional_bool(json_bool_path(&snapshot.acp, &["enabled"])),
        render_optional_bool(json_bool_path(
            &snapshot.acp,
            &["control_plane", "dispatch_enabled"]
        )),
        json_string_path(&snapshot.acp, &["control_plane", "conversation_routing"])
            .unwrap_or_else(|| "-".to_owned()),
        json_string_path(&snapshot.acp, &["control_plane", "thread_routing"])
            .unwrap_or_else(|| "-".to_owned()),
        json_string_path(&snapshot.acp, &["control_plane", "default_agent"])
            .unwrap_or_else(|| "-".to_owned()),
        render_string_values(&json_string_array_path(
            &snapshot.acp,
            &["control_plane", "allowed_agents"]
        )),
    ))
}

fn snapshot_enabled_channel_ids(snapshot: &RuntimeSnapshotArtifactDocument) -> Vec<String> {
    json_string_array_path(&snapshot.channels, &["enabled_channel_ids"])
}

fn snapshot_enabled_service_channel_ids(snapshot: &RuntimeSnapshotArtifactDocument) -> Vec<String> {
    json_string_array_path(&snapshot.channels, &["enabled_service_channel_ids"])
}

fn snapshot_enabled_runtime_backed_channel_ids(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Vec<String> {
    json_string_array_path(&snapshot.channels, &["enabled_runtime_backed_channel_ids"])
}

fn snapshot_enabled_plugin_backed_channel_ids(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Vec<String> {
    json_string_array_path(&snapshot.channels, &["enabled_plugin_backed_channel_ids"])
}

fn snapshot_enabled_outbound_only_channel_ids(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Vec<String> {
    json_string_array_path(&snapshot.channels, &["enabled_outbound_only_channel_ids"])
}

fn snapshot_visible_tool_names(snapshot: &RuntimeSnapshotArtifactDocument) -> Vec<String> {
    json_string_array_path(&snapshot.tools, &["visible_tool_names"])
}

fn snapshot_capability_snapshot_sha256(
    snapshot: &RuntimeSnapshotArtifactDocument,
) -> Option<String> {
    json_string_path(&snapshot.tools, &["capability_snapshot_sha256"])
}

fn snapshot_external_skill_ids(snapshot: &RuntimeSnapshotArtifactDocument) -> Vec<String> {
    json_object_array_string_field_path(
        &snapshot.external_skills,
        &["inventory", "skills"],
        "skill_id",
    )
}

fn json_value_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn json_string_path(value: &Value, path: &[&str]) -> Option<String> {
    json_value_path(value, path)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn json_bool_path(value: &Value, path: &[&str]) -> Option<bool> {
    json_value_path(value, path).and_then(Value::as_bool)
}

fn json_u64_path(value: &Value, path: &[&str]) -> Option<u64> {
    json_value_path(value, path).and_then(Value::as_u64)
}

fn json_string_array_path(value: &Value, path: &[&str]) -> Vec<String> {
    let values = json_value_path(value, path)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    values.into_iter().collect()
}

fn json_object_array_string_field_path(value: &Value, path: &[&str], field: &str) -> Vec<String> {
    let values = json_value_path(value, path)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get(field))
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    values.into_iter().collect()
}

fn compute_run_id(
    created_at: &str,
    label: Option<&str>,
    experiment_id: &str,
    baseline_snapshot: &RuntimeExperimentSnapshotSummary,
    mutation_summary: &str,
    tags: &[String],
) -> CliResult<String> {
    let encoded = serde_json::to_vec(&json!({
        "created_at": created_at,
        "label": label,
        "experiment_id": experiment_id,
        "baseline_snapshot_id": baseline_snapshot.snapshot_id,
        "mutation_summary": mutation_summary,
        "tags": tags,
    }))
    .map_err(|error| format!("serialize runtime experiment run_id input failed: {error}"))?;
    Ok(hex::encode(sha2::Sha256::digest(encoded)))
}

fn persist_runtime_experiment_artifact(
    output: &str,
    artifact: &RuntimeExperimentArtifactDocument,
) -> CliResult<()> {
    let output_path = PathBuf::from(output);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime experiment artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("serialize runtime experiment artifact failed: {error}"))?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "write runtime experiment artifact {} failed: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}

pub fn render_runtime_experiment_text(artifact: &RuntimeExperimentArtifactDocument) -> String {
    let body = [
        format!("run_id={}", artifact.run_id),
        format!("experiment_id={}", artifact.experiment_id),
        format!(
            "baseline_snapshot_id={}",
            artifact.baseline_snapshot.snapshot_id
        ),
        format!(
            "result_snapshot_id={}",
            artifact
                .result_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot_id.as_str())
                .unwrap_or("-")
        ),
        format!("status={}", render_status(artifact.status)),
        format!("decision={}", render_decision(artifact.decision)),
        format!("metrics={}", render_metrics(artifact.evaluation.as_ref())),
        format!("warnings={}", render_warnings(artifact.evaluation.as_ref())),
        format!(
            "baseline_snapshot_path={}",
            artifact
                .baseline_snapshot
                .artifact_path
                .as_deref()
                .unwrap_or("-")
        ),
        format!(
            "result_snapshot_path={}",
            artifact
                .result_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.artifact_path.as_deref())
                .unwrap_or("-")
        ),
        format!("mutation_summary={}", artifact.mutation.summary),
        format!(
            "mutation_tags={}",
            render_string_values(&artifact.mutation.tags)
        ),
    ]
    .join("\n");
    wrap_runtime_experiment_surface("experiment run", body)
}

pub fn render_runtime_experiment_compare_text(report: &RuntimeExperimentCompareReport) -> String {
    let mut lines = vec![
        format!("run_id={}", report.run_id),
        format!("experiment_id={}", report.experiment_id),
        format!(
            "baseline_snapshot_id={}",
            report.baseline_snapshot.snapshot_id
        ),
        format!(
            "result_snapshot_id={}",
            report
                .result_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot_id.as_str())
                .unwrap_or("-")
        ),
        format!("status={}", render_status(report.status)),
        format!("decision={}", render_decision(report.decision)),
        format!(
            "evaluation_summary={}",
            render_evaluation_summary(report.evaluation.as_ref())
        ),
        format!("metrics={}", render_metrics(report.evaluation.as_ref())),
        format!("warnings={}", render_warnings(report.evaluation.as_ref())),
        format!(
            "baseline_snapshot_path={}",
            report
                .baseline_snapshot
                .artifact_path
                .as_deref()
                .unwrap_or("-")
        ),
        format!(
            "result_snapshot_path={}",
            report
                .result_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.artifact_path.as_deref())
                .unwrap_or("-")
        ),
        format!("compare_mode={}", render_compare_mode(report.compare_mode)),
        format!("mutation_summary={}", report.mutation.summary),
        format!(
            "mutation_tags={}",
            render_string_values(&report.mutation.tags)
        ),
    ];

    if let Some(snapshot_delta) = report.snapshot_delta.as_ref() {
        lines.push(format!(
            "snapshot_delta_changed_surfaces={}",
            snapshot_delta.changed_surface_count
        ));
        push_scalar_compare_line(
            &mut lines,
            "provider_active_profile",
            &snapshot_delta.provider_active_profile,
        );
        push_scalar_compare_line(
            &mut lines,
            "provider_active_model",
            &snapshot_delta.provider_active_model,
        );
        push_scalar_compare_line(
            &mut lines,
            "context_engine_selected",
            &snapshot_delta.context_engine_selected,
        );
        push_scalar_compare_line(
            &mut lines,
            "context_engine_compaction",
            &snapshot_delta.context_engine_compaction,
        );
        push_scalar_compare_line(
            &mut lines,
            "memory_selected",
            &snapshot_delta.memory_selected,
        );
        push_scalar_compare_line(&mut lines, "memory_policy", &snapshot_delta.memory_policy);
        push_scalar_compare_line(&mut lines, "acp_selected", &snapshot_delta.acp_selected);
        push_scalar_compare_line(&mut lines, "acp_policy", &snapshot_delta.acp_policy);
        push_set_compare_lines(
            &mut lines,
            "enabled_channel_ids",
            &snapshot_delta.enabled_channel_ids,
        );
        push_set_compare_lines(
            &mut lines,
            "enabled_runtime_backed_channel_ids",
            &snapshot_delta.enabled_runtime_backed_channel_ids,
        );
        push_set_compare_lines(
            &mut lines,
            "enabled_service_channel_ids",
            &snapshot_delta.enabled_service_channel_ids,
        );
        push_set_compare_lines(
            &mut lines,
            "enabled_plugin_backed_channel_ids",
            &snapshot_delta.enabled_plugin_backed_channel_ids,
        );
        push_set_compare_lines(
            &mut lines,
            "enabled_outbound_only_channel_ids",
            &snapshot_delta.enabled_outbound_only_channel_ids,
        );
        push_set_compare_lines(
            &mut lines,
            "visible_tool_names",
            &snapshot_delta.visible_tool_names,
        );
        push_scalar_compare_line(
            &mut lines,
            "capability_snapshot_sha256",
            &snapshot_delta.capability_snapshot_sha256,
        );
        push_set_compare_lines(
            &mut lines,
            "external_skill_ids",
            &snapshot_delta.external_skill_ids,
        );
        if snapshot_delta.changed_surface_count == 0 {
            lines.push("snapshot_delta=none".to_owned());
        }
    }

    wrap_runtime_experiment_surface("experiment compare", lines.join("\n"))
}

fn render_runtime_experiment_restore_text(execution: &RuntimeExperimentRestoreExecution) -> String {
    let body = [
        format!("run_id={}", execution.run_id),
        format!("experiment_id={}", execution.experiment_id),
        format!("stage={}", execution.stage.as_str()),
        crate::runtime_restore_cli::render_runtime_restore_text(&execution.restore),
    ]
    .join("\n");
    wrap_runtime_experiment_surface("experiment restore", body)
}

fn wrap_runtime_experiment_surface(title: &str, body: String) -> String {
    crate::render_operator_shell_surface_from_body(title, "runtime experiment", body)
}

fn render_evaluation_summary(evaluation: Option<&RuntimeExperimentEvaluation>) -> String {
    evaluation
        .map(|evaluation| evaluation.summary.as_str())
        .filter(|summary| !summary.is_empty())
        .unwrap_or("-")
        .to_owned()
}

fn render_metrics(evaluation: Option<&RuntimeExperimentEvaluation>) -> String {
    evaluation
        .map(|evaluation| {
            if evaluation.metrics.is_empty() {
                "-".to_owned()
            } else {
                evaluation
                    .metrics
                    .iter()
                    .map(|(key, value)| format!("{key}:{value}"))
                    .collect::<Vec<_>>()
                    .join(",")
            }
        })
        .unwrap_or_else(|| "-".to_owned())
}

fn render_warnings(evaluation: Option<&RuntimeExperimentEvaluation>) -> String {
    evaluation
        .map(|evaluation| {
            if evaluation.warnings.is_empty() {
                "-".to_owned()
            } else {
                evaluation.warnings.join(" | ")
            }
        })
        .unwrap_or_else(|| "-".to_owned())
}

fn render_string_values(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn render_optional_string(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

fn render_optional_bool(value: Option<bool>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn render_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn push_scalar_compare_line(
    lines: &mut Vec<String>,
    name: &str,
    compare: &RuntimeExperimentScalarCompare,
) {
    if compare.changed() {
        lines.push(format!(
            "{name}={} -> {}",
            render_optional_string(compare.before.as_deref()),
            render_optional_string(compare.after.as_deref())
        ));
    }
}

fn push_set_compare_lines(
    lines: &mut Vec<String>,
    name: &str,
    compare: &RuntimeExperimentSetCompare,
) {
    if compare.changed() {
        lines.push(format!(
            "{name}_added={}",
            render_string_values(&compare.added)
        ));
        lines.push(format!(
            "{name}_removed={}",
            render_string_values(&compare.removed)
        ));
    }
}

fn render_status(status: RuntimeExperimentStatus) -> &'static str {
    match status {
        RuntimeExperimentStatus::Planned => "planned",
        RuntimeExperimentStatus::Completed => "completed",
        RuntimeExperimentStatus::Aborted => "aborted",
    }
}

fn render_decision(decision: RuntimeExperimentDecision) -> &'static str {
    match decision {
        RuntimeExperimentDecision::Undecided => "undecided",
        RuntimeExperimentDecision::Promoted => "promoted",
        RuntimeExperimentDecision::Rejected => "rejected",
    }
}

fn render_compare_mode(mode: RuntimeExperimentCompareMode) -> &'static str {
    match mode {
        RuntimeExperimentCompareMode::RecordOnly => "record_only",
        RuntimeExperimentCompareMode::SnapshotDelta => "snapshot_delta",
    }
}

impl RuntimeExperimentRestoreStage {
    fn as_str(self) -> &'static str {
        match self {
            RuntimeExperimentRestoreStage::Baseline => "baseline",
            RuntimeExperimentRestoreStage::Result => "result",
        }
    }
}
