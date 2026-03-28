use crate::Capability;
use crate::runtime_experiment_cli::{
    RuntimeExperimentArtifactDocument, RuntimeExperimentDecision,
    RuntimeExperimentShowCommandOptions, RuntimeExperimentSnapshotDelta, RuntimeExperimentStatus,
    derive_recorded_snapshot_delta_for_run, execute_runtime_experiment_show_command,
};
use crate::sha2::{self, Digest};
use clap::{Args, Subcommand, ValueEnum};
use loongclaw_spec::CliResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_CAPABILITY_ARTIFACT_SURFACE: &str = "runtime_capability";
pub const RUNTIME_CAPABILITY_ARTIFACT_PURPOSE: &str = "promotion_candidate_record";

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCapabilityCommands {
    /// Create one capability-candidate artifact from one finished experiment run
    Propose(RuntimeCapabilityProposeCommandOptions),
    /// Record one explicit operator review decision for a capability candidate
    Review(RuntimeCapabilityReviewCommandOptions),
    /// Load and render one persisted capability-candidate artifact
    Show(RuntimeCapabilityShowCommandOptions),
    /// Aggregate candidate artifacts into deterministic capability families and readiness states
    Index(RuntimeCapabilityIndexCommandOptions),
    /// Derive one dry-run promotion plan from one indexed capability family
    Plan(RuntimeCapabilityPlanCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityProposeCommandOptions {
    #[arg(long)]
    pub run: String,
    #[arg(long)]
    pub output: String,
    #[arg(long, value_enum)]
    pub target: RuntimeCapabilityTarget,
    #[arg(long)]
    pub target_summary: String,
    #[arg(long)]
    pub bounded_scope: String,
    #[arg(long = "required-capability")]
    pub required_capability: Vec<String>,
    #[arg(long = "tag")]
    pub tag: Vec<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityReviewCommandOptions {
    #[arg(long)]
    pub candidate: String,
    #[arg(long, value_enum)]
    pub decision: RuntimeCapabilityReviewDecision,
    #[arg(long)]
    pub review_summary: String,
    #[arg(long = "warning")]
    pub warning: Vec<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityShowCommandOptions {
    #[arg(long)]
    pub candidate: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityIndexCommandOptions {
    #[arg(long)]
    pub root: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityPlanCommandOptions {
    #[arg(long)]
    pub root: String,
    #[arg(long)]
    pub family_id: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityTarget {
    ManagedSkill,
    ProgrammaticFlow,
    ProfileNoteAddendum,
    #[value(alias = "memory_stage_profile")]
    MemoryStageProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityStatus {
    Proposed,
    Reviewed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityDecision {
    Undecided,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RuntimeCapabilityReviewDecision {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityProposal {
    pub target: RuntimeCapabilityTarget,
    pub summary: String,
    pub bounded_scope: String,
    pub tags: Vec<String>,
    pub required_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeCapabilitySourceRunSummary {
    pub run_id: String,
    pub experiment_id: String,
    pub label: Option<String>,
    pub status: RuntimeExperimentStatus,
    pub decision: RuntimeExperimentDecision,
    pub mutation_summary: String,
    pub baseline_snapshot_id: String,
    pub result_snapshot_id: Option<String>,
    pub evaluation_summary: String,
    pub metrics: std::collections::BTreeMap<String, f64>,
    pub warnings: Vec<String>,
    pub snapshot_delta: Option<RuntimeExperimentSnapshotDelta>,
    pub artifact_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityReview {
    pub summary: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeCapabilityArtifactDocument {
    pub schema: RuntimeCapabilityArtifactSchema,
    pub candidate_id: String,
    pub created_at: String,
    pub reviewed_at: Option<String>,
    pub label: Option<String>,
    pub status: RuntimeCapabilityStatus,
    pub decision: RuntimeCapabilityDecision,
    pub proposal: RuntimeCapabilityProposal,
    pub source_run: RuntimeCapabilitySourceRunSummary,
    pub review: Option<RuntimeCapabilityReview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityFamilyReadinessStatus {
    Ready,
    NotReady,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityFamilyReadinessCheckStatus {
    Pass,
    NeedsEvidence,
    Blocked,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityFamilyReadinessCheck {
    pub dimension: String,
    pub status: RuntimeCapabilityFamilyReadinessCheckStatus,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityFamilyReadiness {
    pub status: RuntimeCapabilityFamilyReadinessStatus,
    pub checks: Vec<RuntimeCapabilityFamilyReadinessCheck>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeCapabilityMetricRange {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilitySourceDecisionRollup {
    pub promoted: usize,
    pub rejected: usize,
    pub undecided: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RuntimeCapabilityEvidenceDigest {
    pub total_candidates: usize,
    pub reviewed_candidates: usize,
    pub undecided_candidates: usize,
    pub accepted_candidates: usize,
    pub rejected_candidates: usize,
    pub distinct_source_run_count: usize,
    pub distinct_experiment_count: usize,
    pub latest_candidate_at: Option<String>,
    pub latest_reviewed_at: Option<String>,
    pub source_decisions: RuntimeCapabilitySourceDecisionRollup,
    pub unique_warnings: Vec<String>,
    pub delta_candidate_count: usize,
    pub changed_surfaces: Vec<String>,
    pub metric_ranges: BTreeMap<String, RuntimeCapabilityMetricRange>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeCapabilityFamilySummary {
    pub family_id: String,
    pub proposal: RuntimeCapabilityProposal,
    pub candidate_ids: Vec<String>,
    pub evidence: RuntimeCapabilityEvidenceDigest,
    pub readiness: RuntimeCapabilityFamilyReadiness,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeCapabilityIndexReport {
    pub generated_at: String,
    pub root: String,
    pub total_candidate_count: usize,
    pub family_count: usize,
    pub families: Vec<RuntimeCapabilityFamilySummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityPromotionArtifactPlan {
    pub target_kind: RuntimeCapabilityTarget,
    pub artifact_kind: String,
    pub artifact_id: String,
    pub delivery_surface: String,
    pub summary: String,
    pub bounded_scope: String,
    pub required_capabilities: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityPromotionProvenance {
    pub candidate_ids: Vec<String>,
    pub source_run_ids: Vec<String>,
    pub experiment_ids: Vec<String>,
    pub source_run_artifact_paths: Vec<String>,
    pub latest_candidate_at: Option<String>,
    pub latest_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeCapabilityPromotionPlanReport {
    pub generated_at: String,
    pub root: String,
    pub family_id: String,
    pub promotable: bool,
    pub proposal: RuntimeCapabilityProposal,
    pub evidence: RuntimeCapabilityEvidenceDigest,
    pub readiness: RuntimeCapabilityFamilyReadiness,
    pub planned_artifact: RuntimeCapabilityPromotionArtifactPlan,
    pub blockers: Vec<RuntimeCapabilityFamilyReadinessCheck>,
    pub approval_checklist: Vec<String>,
    pub rollback_hints: Vec<String>,
    pub provenance: RuntimeCapabilityPromotionProvenance,
}

pub fn run_runtime_capability_cli(command: RuntimeCapabilityCommands) -> CliResult<()> {
    match command {
        RuntimeCapabilityCommands::Propose(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_capability_propose_command(options)?;
            emit_runtime_capability_artifact(&artifact, as_json)
        }
        RuntimeCapabilityCommands::Review(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_capability_review_command(options)?;
            emit_runtime_capability_artifact(&artifact, as_json)
        }
        RuntimeCapabilityCommands::Show(options) => {
            let as_json = options.json;
            let artifact = execute_runtime_capability_show_command(options)?;
            emit_runtime_capability_artifact(&artifact, as_json)
        }
        RuntimeCapabilityCommands::Index(options) => {
            let as_json = options.json;
            let report = execute_runtime_capability_index_command(options)?;
            emit_runtime_capability_index_report(&report, as_json)
        }
        RuntimeCapabilityCommands::Plan(options) => {
            let as_json = options.json;
            let report = execute_runtime_capability_plan_command(options)?;
            emit_runtime_capability_promotion_plan(&report, as_json)
        }
    }
}

pub fn execute_runtime_capability_propose_command(
    options: RuntimeCapabilityProposeCommandOptions,
) -> CliResult<RuntimeCapabilityArtifactDocument> {
    let run = execute_runtime_experiment_show_command(RuntimeExperimentShowCommandOptions {
        run: options.run.clone(),
        json: false,
    })?;
    validate_proposable_run(&run, &options.run)?;

    let created_at = now_rfc3339()?;
    let label = optional_arg(options.label.as_deref());
    let summary = required_trimmed_arg("target_summary", &options.target_summary)?;
    let bounded_scope = required_trimmed_arg("bounded_scope", &options.bounded_scope)?;
    let tags = normalize_repeated_values(&options.tag);
    let required_capabilities = parse_required_capabilities(&options.required_capability)?;
    let source_run = build_source_run_summary(&run, Some(Path::new(&options.run)))?;
    let candidate_id = compute_candidate_id(
        &created_at,
        label.as_deref(),
        &source_run,
        options.target,
        &summary,
        &bounded_scope,
        &tags,
        &required_capabilities,
    )?;
    let artifact = RuntimeCapabilityArtifactDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_ARTIFACT_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_ARTIFACT_PURPOSE.to_owned(),
        },
        candidate_id,
        created_at,
        reviewed_at: None,
        label,
        status: RuntimeCapabilityStatus::Proposed,
        decision: RuntimeCapabilityDecision::Undecided,
        proposal: RuntimeCapabilityProposal {
            target: options.target,
            summary,
            bounded_scope,
            tags,
            required_capabilities,
        },
        source_run,
        review: None,
    };
    persist_runtime_capability_artifact(&options.output, &artifact)?;
    Ok(artifact)
}

pub fn execute_runtime_capability_review_command(
    options: RuntimeCapabilityReviewCommandOptions,
) -> CliResult<RuntimeCapabilityArtifactDocument> {
    let mut artifact = load_runtime_capability_artifact(Path::new(&options.candidate))?;
    if artifact.status != RuntimeCapabilityStatus::Proposed {
        return Err(format!(
            "runtime capability candidate {} is already reviewed",
            options.candidate
        ));
    }

    artifact.reviewed_at = Some(now_rfc3339()?);
    artifact.status = RuntimeCapabilityStatus::Reviewed;
    artifact.decision = match options.decision {
        RuntimeCapabilityReviewDecision::Accepted => RuntimeCapabilityDecision::Accepted,
        RuntimeCapabilityReviewDecision::Rejected => RuntimeCapabilityDecision::Rejected,
    };
    artifact.review = Some(RuntimeCapabilityReview {
        summary: required_trimmed_arg("review_summary", &options.review_summary)?,
        warnings: normalize_repeated_values(&options.warning),
    });
    persist_runtime_capability_artifact(&options.candidate, &artifact)?;
    Ok(artifact)
}

pub fn execute_runtime_capability_show_command(
    options: RuntimeCapabilityShowCommandOptions,
) -> CliResult<RuntimeCapabilityArtifactDocument> {
    load_runtime_capability_artifact(Path::new(&options.candidate))
}

pub fn execute_runtime_capability_index_command(
    options: RuntimeCapabilityIndexCommandOptions,
) -> CliResult<RuntimeCapabilityIndexReport> {
    let root_path = Path::new(&options.root);
    let root = canonicalize_existing_path(root_path)?;
    let families_by_id = collect_runtime_capability_family_artifacts(root_path)?;
    let total_candidate_count = families_by_id.values().map(Vec::len).sum();

    let mut families = Vec::new();
    for (family_id, artifacts) in families_by_id {
        families.push(build_runtime_capability_family_summary(
            family_id, artifacts,
        )?);
    }

    Ok(RuntimeCapabilityIndexReport {
        generated_at: now_rfc3339()?,
        root,
        total_candidate_count,
        family_count: families.len(),
        families,
    })
}

pub fn execute_runtime_capability_plan_command(
    options: RuntimeCapabilityPlanCommandOptions,
) -> CliResult<RuntimeCapabilityPromotionPlanReport> {
    let root_path = Path::new(&options.root);
    let root = canonicalize_existing_path(root_path)?;
    let families_by_id = collect_runtime_capability_family_artifacts(root_path)?;
    let family_artifacts = families_by_id
        .get(&options.family_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "runtime capability family `{}` not found under {}",
                options.family_id, root
            )
        })?;
    let family = build_runtime_capability_family_summary(
        options.family_id.clone(),
        family_artifacts.clone(),
    )?;
    let planned_artifact =
        build_runtime_capability_promotion_artifact(&family.family_id, &family.proposal);
    let blockers = family
        .readiness
        .checks
        .iter()
        .filter(|check| check.status != RuntimeCapabilityFamilyReadinessCheckStatus::Pass)
        .cloned()
        .collect::<Vec<_>>();

    Ok(RuntimeCapabilityPromotionPlanReport {
        generated_at: now_rfc3339()?,
        root,
        family_id: family.family_id.clone(),
        promotable: family.readiness.status == RuntimeCapabilityFamilyReadinessStatus::Ready,
        proposal: family.proposal.clone(),
        evidence: family.evidence.clone(),
        readiness: family.readiness.clone(),
        planned_artifact: planned_artifact.clone(),
        blockers,
        approval_checklist: build_runtime_capability_approval_checklist(&planned_artifact),
        rollback_hints: build_runtime_capability_rollback_hints(&planned_artifact),
        provenance: build_runtime_capability_promotion_provenance(
            &family_artifacts,
            &family.evidence,
        ),
    })
}

fn emit_runtime_capability_artifact(
    artifact: &RuntimeCapabilityArtifactDocument,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(artifact)
            .map_err(|error| format!("serialize runtime capability artifact failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_text(artifact));
    Ok(())
}

fn emit_runtime_capability_index_report(
    report: &RuntimeCapabilityIndexReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability index report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_index_text(report));
    Ok(())
}

fn emit_runtime_capability_promotion_plan(
    report: &RuntimeCapabilityPromotionPlanReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability promotion plan failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_promotion_plan_text(report));
    Ok(())
}

fn validate_proposable_run(
    run: &RuntimeExperimentArtifactDocument,
    run_path: &str,
) -> CliResult<()> {
    if run.status == RuntimeExperimentStatus::Planned {
        return Err(format!(
            "runtime capability propose requires a finished runtime experiment run; {} is still planned",
            run_path
        ));
    }
    if run.evaluation.is_none() {
        return Err(format!(
            "runtime capability propose requires evaluation data on source run {}",
            run_path
        ));
    }
    Ok(())
}

fn build_source_run_summary(
    run: &RuntimeExperimentArtifactDocument,
    artifact_path: Option<&Path>,
) -> CliResult<RuntimeCapabilitySourceRunSummary> {
    let evaluation = run
        .evaluation
        .as_ref()
        .ok_or_else(|| "runtime capability source run is missing evaluation".to_owned())?;
    let snapshot_delta = artifact_path
        .map(|path| derive_recorded_snapshot_delta_for_run(run, &path.display().to_string()))
        .transpose()?
        .flatten();
    Ok(RuntimeCapabilitySourceRunSummary {
        run_id: run.run_id.clone(),
        experiment_id: run.experiment_id.clone(),
        label: run.label.clone(),
        status: run.status,
        decision: run.decision,
        mutation_summary: run.mutation.summary.clone(),
        baseline_snapshot_id: run.baseline_snapshot.snapshot_id.clone(),
        result_snapshot_id: run
            .result_snapshot
            .as_ref()
            .map(|snapshot| snapshot.snapshot_id.clone()),
        evaluation_summary: evaluation.summary.clone(),
        metrics: evaluation.metrics.clone(),
        warnings: evaluation.warnings.clone(),
        snapshot_delta,
        artifact_path: artifact_path.map(canonicalize_existing_path).transpose()?,
    })
}

fn load_runtime_capability_artifact(path: &Path) -> CliResult<RuntimeCapabilityArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<RuntimeCapabilityArtifactDocument>(&raw).map_err(|error| {
            format!(
                "decode runtime capability artifact {} failed: {error}",
                path.display()
            )
        })?;
    if artifact.schema.version != RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime capability artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            artifact.schema.version,
            RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    validate_runtime_capability_artifact_schema(&artifact, path)?;
    validate_runtime_capability_artifact_state(&artifact, path)?;
    Ok(artifact)
}

fn validate_runtime_capability_artifact_schema(
    artifact: &RuntimeCapabilityArtifactDocument,
    path: &Path,
) -> CliResult<()> {
    if artifact.schema.surface != RUNTIME_CAPABILITY_ARTIFACT_SURFACE {
        return Err(format!(
            "runtime capability artifact {} uses unsupported schema surface {}; expected {}",
            path.display(),
            artifact.schema.surface,
            RUNTIME_CAPABILITY_ARTIFACT_SURFACE
        ));
    }
    if artifact.schema.purpose != RUNTIME_CAPABILITY_ARTIFACT_PURPOSE {
        return Err(format!(
            "runtime capability artifact {} uses unsupported schema purpose {}; expected {}",
            path.display(),
            artifact.schema.purpose,
            RUNTIME_CAPABILITY_ARTIFACT_PURPOSE
        ));
    }
    Ok(())
}

fn validate_runtime_capability_artifact_state(
    artifact: &RuntimeCapabilityArtifactDocument,
    path: &Path,
) -> CliResult<()> {
    match artifact.status {
        RuntimeCapabilityStatus::Proposed => {
            if artifact.reviewed_at.is_some()
                || artifact.review.is_some()
                || artifact.decision != RuntimeCapabilityDecision::Undecided
            {
                return Err(format!(
                    "runtime capability artifact {} has inconsistent proposed state",
                    path.display()
                ));
            }
        }
        RuntimeCapabilityStatus::Reviewed => {
            if artifact.reviewed_at.is_none()
                || artifact.review.is_none()
                || artifact.decision == RuntimeCapabilityDecision::Undecided
            {
                return Err(format!(
                    "runtime capability artifact {} has inconsistent reviewed state",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn collect_runtime_capability_artifacts(
    root: &Path,
    artifacts: &mut Vec<RuntimeCapabilityArtifactDocument>,
) -> CliResult<()> {
    let mut entries = fs::read_dir(root)
        .map_err(|error| {
            format!(
                "read runtime capability index root {} failed: {error}",
                root.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "enumerate runtime capability index root {} failed: {error}",
                root.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let entry_type = entry.file_type().map_err(|error| {
            format!(
                "inspect runtime capability index entry {} failed: {error}",
                path.display()
            )
        })?;
        if entry_type.is_symlink() {
            continue;
        }
        if entry_type.is_dir() {
            collect_runtime_capability_artifacts(&path, artifacts)?;
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(artifact) = load_supported_runtime_capability_artifact(&path)? else {
            continue;
        };
        artifacts.push(artifact);
    }
    Ok(())
}

fn collect_runtime_capability_family_artifacts(
    root: &Path,
) -> CliResult<BTreeMap<String, Vec<RuntimeCapabilityArtifactDocument>>> {
    let mut artifacts = Vec::new();
    collect_runtime_capability_artifacts(root, &mut artifacts)?;

    let mut families_by_id = BTreeMap::<String, Vec<RuntimeCapabilityArtifactDocument>>::new();
    for artifact in artifacts {
        let family_id = compute_family_id(&artifact.proposal)?;
        families_by_id.entry(family_id).or_default().push(artifact);
    }
    Ok(families_by_id)
}

fn load_supported_runtime_capability_artifact(
    path: &Path,
) -> CliResult<Option<RuntimeCapabilityArtifactDocument>> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability index entry {} failed: {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
        format!(
            "decode runtime capability index entry {} failed: {error}",
            path.display()
        )
    })?;
    let Some(surface) = value
        .get("schema")
        .and_then(|schema| schema.get("surface"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    if surface != RUNTIME_CAPABILITY_ARTIFACT_SURFACE {
        return Ok(None);
    }
    load_runtime_capability_artifact(path).map(Some)
}

fn sort_runtime_capability_artifacts(artifacts: &mut [RuntimeCapabilityArtifactDocument]) {
    artifacts.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
}

fn build_runtime_capability_family_summary(
    family_id: String,
    mut artifacts: Vec<RuntimeCapabilityArtifactDocument>,
) -> CliResult<RuntimeCapabilityFamilySummary> {
    sort_runtime_capability_artifacts(&mut artifacts);
    let proposal = artifacts
        .first()
        .map(|artifact| artifact.proposal.clone())
        .ok_or_else(|| "runtime capability family cannot be empty".to_owned())?;
    let candidate_ids = artifacts
        .iter()
        .map(|artifact| artifact.candidate_id.clone())
        .collect::<Vec<_>>();
    let evidence = build_family_evidence_digest(&artifacts);
    let readiness = evaluate_family_readiness(&artifacts, &evidence);

    Ok(RuntimeCapabilityFamilySummary {
        family_id,
        proposal,
        candidate_ids,
        evidence,
        readiness,
    })
}

fn compute_family_id(proposal: &RuntimeCapabilityProposal) -> CliResult<String> {
    let tags = normalize_repeated_values(&proposal.tags);
    let required_capabilities = parse_required_capabilities(&proposal.required_capabilities)?;
    let encoded = serde_json::to_vec(&json!({
        "target": render_target(proposal.target),
        "summary": proposal.summary.trim(),
        "bounded_scope": proposal.bounded_scope.trim(),
        "tags": tags,
        "required_capabilities": required_capabilities,
    }))
    .map_err(|error| format!("serialize runtime capability family_id input failed: {error}"))?;
    Ok(format!("{:x}", sha2::Sha256::digest(encoded)))
}

fn build_family_evidence_digest(
    artifacts: &[RuntimeCapabilityArtifactDocument],
) -> RuntimeCapabilityEvidenceDigest {
    let reviewed_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.status == RuntimeCapabilityStatus::Reviewed)
        .count();
    let undecided_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Undecided)
        .count();
    let accepted_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Accepted)
        .count();
    let rejected_candidates = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Rejected)
        .count();
    let distinct_source_run_count = artifacts
        .iter()
        .map(|artifact| artifact.source_run.run_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let distinct_experiment_count = artifacts
        .iter()
        .map(|artifact| artifact.source_run.experiment_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let latest_candidate_at = artifacts
        .iter()
        .map(|artifact| artifact.created_at.as_str())
        .max()
        .map(str::to_owned);
    let latest_reviewed_at = artifacts
        .iter()
        .filter_map(|artifact| artifact.reviewed_at.as_deref())
        .max()
        .map(str::to_owned);

    let mut promoted = 0;
    let mut rejected = 0;
    let mut undecided = 0;
    let mut unique_warnings = BTreeSet::new();
    let mut changed_surfaces = BTreeSet::new();
    let mut delta_candidate_count = 0;
    let mut metric_bounds = BTreeMap::<String, RuntimeCapabilityMetricRange>::new();

    for artifact in artifacts {
        match artifact.source_run.decision {
            RuntimeExperimentDecision::Promoted => promoted += 1,
            RuntimeExperimentDecision::Rejected => rejected += 1,
            RuntimeExperimentDecision::Undecided => undecided += 1,
        }

        if let Some(snapshot_delta) = artifact.source_run.snapshot_delta.as_ref() {
            delta_candidate_count += 1;
            changed_surfaces.extend(snapshot_delta.changed_surfaces());
        }

        if artifact.decision == RuntimeCapabilityDecision::Accepted {
            for warning in &artifact.source_run.warnings {
                unique_warnings.insert(warning.clone());
            }
        }

        for (metric, value) in &artifact.source_run.metrics {
            let entry = metric_bounds.entry(metric.clone()).or_insert_with(|| {
                RuntimeCapabilityMetricRange {
                    min: *value,
                    max: *value,
                }
            });
            entry.min = entry.min.min(*value);
            entry.max = entry.max.max(*value);
        }
    }

    RuntimeCapabilityEvidenceDigest {
        total_candidates: artifacts.len(),
        reviewed_candidates,
        undecided_candidates,
        accepted_candidates,
        rejected_candidates,
        distinct_source_run_count,
        distinct_experiment_count,
        latest_candidate_at,
        latest_reviewed_at,
        source_decisions: RuntimeCapabilitySourceDecisionRollup {
            promoted,
            rejected,
            undecided,
        },
        unique_warnings: unique_warnings.into_iter().collect(),
        delta_candidate_count,
        changed_surfaces: changed_surfaces.into_iter().collect(),
        metric_ranges: metric_bounds,
    }
}

fn evaluate_family_readiness(
    artifacts: &[RuntimeCapabilityArtifactDocument],
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadiness {
    let review_consensus = evaluate_review_consensus(evidence);
    let stability = evaluate_stability(evidence);
    let accepted_source_integrity = evaluate_accepted_source_integrity(artifacts, evidence);
    let warning_pressure = evaluate_warning_pressure(evidence);
    let mut checks = vec![
        review_consensus,
        stability,
        accepted_source_integrity,
        warning_pressure,
    ];
    checks.extend(evaluate_target_specific_readiness(artifacts));
    let status = if checks
        .iter()
        .any(|check| check.status == RuntimeCapabilityFamilyReadinessCheckStatus::Blocked)
    {
        RuntimeCapabilityFamilyReadinessStatus::Blocked
    } else if checks
        .iter()
        .all(|check| check.status == RuntimeCapabilityFamilyReadinessCheckStatus::Pass)
    {
        RuntimeCapabilityFamilyReadinessStatus::Ready
    } else {
        RuntimeCapabilityFamilyReadinessStatus::NotReady
    };
    RuntimeCapabilityFamilyReadiness { status, checks }
}

fn evaluate_target_specific_readiness(
    artifacts: &[RuntimeCapabilityArtifactDocument],
) -> Vec<RuntimeCapabilityFamilyReadinessCheck> {
    let Some(target) = artifacts.first().map(|artifact| artifact.proposal.target) else {
        return Vec::new();
    };

    match target {
        RuntimeCapabilityTarget::MemoryStageProfile => {
            let accepted_artifacts = artifacts
                .iter()
                .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Accepted)
                .cloned()
                .collect::<Vec<_>>();
            let accepted_evidence = if accepted_artifacts.is_empty() {
                RuntimeCapabilityEvidenceDigest::default()
            } else {
                build_family_evidence_digest(&accepted_artifacts)
            };
            vec![evaluate_memory_stage_profile_delta_evidence(
                &accepted_evidence,
            )]
        }
        RuntimeCapabilityTarget::ManagedSkill
        | RuntimeCapabilityTarget::ProgrammaticFlow
        | RuntimeCapabilityTarget::ProfileNoteAddendum => Vec::new(),
    }
}

fn evaluate_memory_stage_profile_delta_evidence(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let has_memory_surface = evidence.changed_surfaces.iter().any(|surface| {
        matches!(
            surface.as_str(),
            "memory_selected"
                | "memory_policy"
                | "context_engine_selected"
                | "context_engine_compaction"
        )
    });

    let (status, summary) = if evidence.delta_candidate_count == 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            "memory-stage-profile families need snapshot-delta evidence from finished experiments"
                .to_owned(),
        )
    } else if !has_memory_surface {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            "snapshot-delta evidence must include memory or context-engine surfaces".to_owned(),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "snapshot-delta evidence includes memory/context-engine surface changes".to_owned(),
        )
    };

    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "memory_delta_evidence".to_owned(),
        status,
        summary,
    }
}

fn evaluate_review_consensus(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let (status, summary) = if evidence.rejected_candidates > 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Blocked,
            format!(
                "{} candidate(s) in this family were explicitly rejected",
                evidence.rejected_candidates
            ),
        )
    } else if evidence.undecided_candidates > 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            format!(
                "{} candidate(s) still require operator review",
                evidence.undecided_candidates
            ),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "all candidate evidence is reviewed and accepted".to_owned(),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "review_consensus".to_owned(),
        status,
        summary,
    }
}

fn evaluate_stability(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let (status, summary) = if evidence.distinct_source_run_count >= 2 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            format!(
                "family is supported by {} distinct source runs",
                evidence.distinct_source_run_count
            ),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            "family needs repeated evidence from at least two distinct source runs".to_owned(),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "stability".to_owned(),
        status,
        summary,
    }
}

fn evaluate_accepted_source_integrity(
    artifacts: &[RuntimeCapabilityArtifactDocument],
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    if evidence.accepted_candidates == 0 {
        return RuntimeCapabilityFamilyReadinessCheck {
            dimension: "accepted_source_integrity".to_owned(),
            status: RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            summary: "family has no accepted candidates yet".to_owned(),
        };
    }

    let invalid_sources = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Accepted)
        .filter(|artifact| {
            artifact.source_run.status != RuntimeExperimentStatus::Completed
                || artifact.source_run.decision != RuntimeExperimentDecision::Promoted
                || artifact.source_run.result_snapshot_id.is_none()
        })
        .count();

    let (status, summary) = if invalid_sources > 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Blocked,
            format!(
                "{} accepted candidate(s) came from incomplete or non-promoted source runs",
                invalid_sources
            ),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "accepted candidates all trace back to completed promoted runs".to_owned(),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "accepted_source_integrity".to_owned(),
        status,
        summary,
    }
}

fn evaluate_warning_pressure(
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityFamilyReadinessCheck {
    let (status, summary) = if evidence.accepted_candidates == 0 {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            "warning pressure cannot be evaluated before the family has accepted evidence"
                .to_owned(),
        )
    } else if evidence.unique_warnings.is_empty() {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::Pass,
            "accepted candidates carry no source warnings".to_owned(),
        )
    } else {
        (
            RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence,
            format!(
                "accepted evidence still carries warnings: {}",
                evidence.unique_warnings.join(" | ")
            ),
        )
    };
    RuntimeCapabilityFamilyReadinessCheck {
        dimension: "warning_pressure".to_owned(),
        status,
        summary,
    }
}

fn now_rfc3339() -> CliResult<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("format runtime capability timestamp failed: {error}"))
}

fn optional_arg(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_trimmed_arg(name: &str, raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("runtime capability {name} cannot be empty"));
    }
    Ok(trimmed.to_owned())
}

fn normalize_repeated_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parse_required_capabilities(values: &[String]) -> CliResult<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for raw in values {
        let value = normalize_required_capability(raw)?;
        normalized.insert(value);
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_required_capability(raw: &str) -> CliResult<String> {
    Capability::parse(raw)
        .map(|capability| capability.as_str().to_owned())
        .ok_or_else(|| {
            format!(
                "runtime capability required capability `{}` is unknown",
                raw.trim()
            )
        })
}

fn compute_candidate_id(
    created_at: &str,
    label: Option<&str>,
    source_run: &RuntimeCapabilitySourceRunSummary,
    target: RuntimeCapabilityTarget,
    summary: &str,
    bounded_scope: &str,
    tags: &[String],
    required_capabilities: &[String],
) -> CliResult<String> {
    let encoded = serde_json::to_vec(&json!({
        "created_at": created_at,
        "label": label,
        "source_run_id": source_run.run_id,
        "target": render_target(target),
        "summary": summary,
        "bounded_scope": bounded_scope,
        "tags": tags,
        "required_capabilities": required_capabilities,
    }))
    .map_err(|error| format!("serialize runtime capability candidate_id input failed: {error}"))?;
    Ok(format!("{:x}", sha2::Sha256::digest(encoded)))
}

fn persist_runtime_capability_artifact(
    output: &str,
    artifact: &RuntimeCapabilityArtifactDocument,
) -> CliResult<()> {
    let output_path = PathBuf::from(output);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime capability artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("serialize runtime capability artifact failed: {error}"))?;
    fs::write(&output_path, encoded).map_err(|error| {
        format!(
            "write runtime capability artifact {} failed: {error}",
            output_path.display()
        )
    })?;
    Ok(())
}

fn canonicalize_existing_path(path: &Path) -> CliResult<String> {
    dunce::canonicalize(path)
        .map(|resolved| resolved.display().to_string())
        .map_err(|error| {
            format!(
                "canonicalize artifact path {} failed: {error}",
                path.display()
            )
        })
}

pub fn render_runtime_capability_text(artifact: &RuntimeCapabilityArtifactDocument) -> String {
    [
        format!("candidate_id={}", artifact.candidate_id),
        format!("status={}", render_capability_status(artifact.status)),
        format!("decision={}", render_capability_decision(artifact.decision)),
        format!("target={}", render_target(artifact.proposal.target)),
        format!("target_summary={}", artifact.proposal.summary),
        format!("bounded_scope={}", artifact.proposal.bounded_scope),
        format!(
            "required_capabilities={}",
            render_string_values(&artifact.proposal.required_capabilities)
        ),
        format!("tags={}", render_string_values(&artifact.proposal.tags)),
        format!("source_run_id={}", artifact.source_run.run_id),
        format!("source_experiment_id={}", artifact.source_run.experiment_id),
        format!(
            "source_run_status={}",
            render_experiment_status(artifact.source_run.status)
        ),
        format!(
            "source_run_decision={}",
            render_experiment_decision(artifact.source_run.decision)
        ),
        format!(
            "source_metrics={}",
            render_metrics(&artifact.source_run.metrics)
        ),
        format!(
            "source_warnings={}",
            render_string_values_with_separator(&artifact.source_run.warnings, " | ")
        ),
        format!(
            "source_snapshot_delta_changed_surface_count={}",
            artifact
                .source_run
                .snapshot_delta
                .as_ref()
                .map(|delta| delta.changed_surface_count.to_string())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "source_snapshot_delta_changed_surfaces={}",
            artifact
                .source_run
                .snapshot_delta
                .as_ref()
                .map(|delta| render_string_values(&delta.changed_surfaces()))
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "review_summary={}",
            artifact
                .review
                .as_ref()
                .map(|review| review.summary.as_str())
                .unwrap_or("-")
        ),
        format!(
            "review_warnings={}",
            artifact
                .review
                .as_ref()
                .map(|review| render_string_values_with_separator(&review.warnings, " | "))
                .unwrap_or_else(|| "-".to_owned())
        ),
    ]
    .join("\n")
}

pub fn render_runtime_capability_index_text(report: &RuntimeCapabilityIndexReport) -> String {
    let mut lines = vec![
        format!("root={}", report.root),
        format!("family_count={}", report.family_count),
        format!("total_candidate_count={}", report.total_candidate_count),
    ];

    for family in &report.families {
        lines.push(String::new());
        lines.push(format!("family_id={}", family.family_id));
        lines.push(format!(
            "readiness={}",
            render_family_readiness_status(family.readiness.status)
        ));
        lines.push(format!("target={}", render_target(family.proposal.target)));
        lines.push(format!("target_summary={}", family.proposal.summary));
        lines.push(format!("bounded_scope={}", family.proposal.bounded_scope));
        lines.push(format!(
            "candidate_ids={}",
            render_string_values(&family.candidate_ids)
        ));
        lines.push(format!(
            "evidence_counts=total:{} reviewed:{} accepted:{} rejected:{} undecided:{}",
            family.evidence.total_candidates,
            family.evidence.reviewed_candidates,
            family.evidence.accepted_candidates,
            family.evidence.rejected_candidates,
            family.evidence.undecided_candidates
        ));
        lines.push(format!(
            "distinct_source_runs={}",
            family.evidence.distinct_source_run_count
        ));
        lines.push(format!(
            "distinct_experiments={}",
            family.evidence.distinct_experiment_count
        ));
        lines.push(format!(
            "metric_ranges={}",
            render_metric_ranges(&family.evidence.metric_ranges)
        ));
        lines.push(format!(
            "warnings={}",
            render_string_values_with_separator(&family.evidence.unique_warnings, " | ")
        ));
        lines.push(format!(
            "delta_evidence_candidates={}",
            family.evidence.delta_candidate_count
        ));
        lines.push(format!(
            "delta_changed_surfaces={}",
            render_string_values(&family.evidence.changed_surfaces)
        ));
        lines.push(format!(
            "checks={}",
            family
                .readiness
                .checks
                .iter()
                .map(render_family_readiness_check)
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    lines.join("\n")
}

fn render_metrics(metrics: &std::collections::BTreeMap<String, f64>) -> String {
    if metrics.is_empty() {
        "-".to_owned()
    } else {
        metrics
            .iter()
            .map(|(key, value)| format!("{key}:{value}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn render_string_values(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn render_string_values_with_separator(values: &[String], separator: &str) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(separator)
    }
}

fn render_metric_ranges(ranges: &BTreeMap<String, RuntimeCapabilityMetricRange>) -> String {
    if ranges.is_empty() {
        "-".to_owned()
    } else {
        ranges
            .iter()
            .map(|(key, range)| format!("{key}:{}..{}", range.min, range.max))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn render_family_readiness_check(check: &RuntimeCapabilityFamilyReadinessCheck) -> String {
    format!(
        "{}:{}:{}",
        check.dimension,
        render_family_readiness_check_status(check.status),
        check.summary
    )
}

fn render_target(target: RuntimeCapabilityTarget) -> &'static str {
    match target {
        RuntimeCapabilityTarget::ManagedSkill => "managed_skill",
        RuntimeCapabilityTarget::ProgrammaticFlow => "programmatic_flow",
        RuntimeCapabilityTarget::ProfileNoteAddendum => "profile_note_addendum",
        RuntimeCapabilityTarget::MemoryStageProfile => "memory_stage_profile",
    }
}

fn render_capability_status(status: RuntimeCapabilityStatus) -> &'static str {
    match status {
        RuntimeCapabilityStatus::Proposed => "proposed",
        RuntimeCapabilityStatus::Reviewed => "reviewed",
    }
}

fn render_capability_decision(decision: RuntimeCapabilityDecision) -> &'static str {
    match decision {
        RuntimeCapabilityDecision::Undecided => "undecided",
        RuntimeCapabilityDecision::Accepted => "accepted",
        RuntimeCapabilityDecision::Rejected => "rejected",
    }
}

fn render_experiment_status(status: RuntimeExperimentStatus) -> &'static str {
    match status {
        RuntimeExperimentStatus::Planned => "planned",
        RuntimeExperimentStatus::Completed => "completed",
        RuntimeExperimentStatus::Aborted => "aborted",
    }
}

fn render_experiment_decision(decision: RuntimeExperimentDecision) -> &'static str {
    match decision {
        RuntimeExperimentDecision::Undecided => "undecided",
        RuntimeExperimentDecision::Promoted => "promoted",
        RuntimeExperimentDecision::Rejected => "rejected",
    }
}

fn render_family_readiness_status(status: RuntimeCapabilityFamilyReadinessStatus) -> &'static str {
    match status {
        RuntimeCapabilityFamilyReadinessStatus::Ready => "ready",
        RuntimeCapabilityFamilyReadinessStatus::NotReady => "not_ready",
        RuntimeCapabilityFamilyReadinessStatus::Blocked => "blocked",
    }
}

fn render_family_readiness_check_status(
    status: RuntimeCapabilityFamilyReadinessCheckStatus,
) -> &'static str {
    match status {
        RuntimeCapabilityFamilyReadinessCheckStatus::Pass => "pass",
        RuntimeCapabilityFamilyReadinessCheckStatus::NeedsEvidence => "needs_evidence",
        RuntimeCapabilityFamilyReadinessCheckStatus::Blocked => "blocked",
    }
}

fn build_runtime_capability_promotion_artifact(
    family_id: &str,
    proposal: &RuntimeCapabilityProposal,
) -> RuntimeCapabilityPromotionArtifactPlan {
    let (artifact_kind, delivery_surface, id_prefix) =
        runtime_capability_promotion_target_contract(proposal.target);
    let artifact_id = format!(
        "{id_prefix}-{}-{}",
        slugify_runtime_capability_identifier(&proposal.summary),
        family_id.chars().take(12).collect::<String>()
    );

    RuntimeCapabilityPromotionArtifactPlan {
        target_kind: proposal.target,
        artifact_kind: artifact_kind.to_owned(),
        artifact_id,
        delivery_surface: delivery_surface.to_owned(),
        summary: proposal.summary.clone(),
        bounded_scope: proposal.bounded_scope.clone(),
        required_capabilities: proposal.required_capabilities.clone(),
        tags: proposal.tags.clone(),
    }
}

fn runtime_capability_promotion_target_contract(
    target: RuntimeCapabilityTarget,
) -> (&'static str, &'static str, &'static str) {
    match target {
        RuntimeCapabilityTarget::ManagedSkill => {
            ("managed_skill_bundle", "managed_skills", "managed-skill")
        }
        RuntimeCapabilityTarget::ProgrammaticFlow => (
            "programmatic_flow_spec",
            "programmatic_flows",
            "programmatic-flow",
        ),
        RuntimeCapabilityTarget::ProfileNoteAddendum => {
            ("profile_note_addendum", "profile_note", "profile-note")
        }
        RuntimeCapabilityTarget::MemoryStageProfile => (
            "memory_stage_profile",
            "memory_stage_profiles",
            "memory-stage-profile",
        ),
    }
}

fn slugify_runtime_capability_identifier(raw: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "capability".to_owned()
    } else {
        trimmed
    }
}

fn build_runtime_capability_approval_checklist(
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
) -> Vec<String> {
    let mut checklist = vec![
        "confirm summary and bounded scope still describe exactly one lower-layer artifact"
            .to_owned(),
        "confirm required capabilities remain least-privilege for the planned artifact".to_owned(),
        "confirm provenance references still represent the intended behavior to codify".to_owned(),
        format!(
            "confirm the chosen delivery surface `{}` matches the target kind",
            planned_artifact.delivery_surface
        ),
    ];
    checklist.push(match planned_artifact.target_kind {
        RuntimeCapabilityTarget::ManagedSkill => {
            "confirm the behavior belongs in a reusable managed skill".to_owned()
        }
        RuntimeCapabilityTarget::ProgrammaticFlow => {
            "confirm the behavior can be expressed as a deterministic programmatic flow".to_owned()
        }
        RuntimeCapabilityTarget::ProfileNoteAddendum => {
            "confirm the behavior belongs in advisory profile guidance rather than executable logic"
                .to_owned()
        }
        RuntimeCapabilityTarget::MemoryStageProfile => {
            "confirm the behavior belongs in a governed memory stage profile rather than live runtime mutation"
                .to_owned()
        }
    });
    checklist
}

fn build_runtime_capability_rollback_hints(
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
) -> Vec<String> {
    let capture_hint = match planned_artifact.target_kind {
        RuntimeCapabilityTarget::MemoryStageProfile => format!(
            "capture the current `{}` state before applying this memory stage profile",
            planned_artifact.delivery_surface
        ),
        RuntimeCapabilityTarget::ManagedSkill
        | RuntimeCapabilityTarget::ProgrammaticFlow
        | RuntimeCapabilityTarget::ProfileNoteAddendum => format!(
            "capture the current `{}` state before applying artifact `{}`",
            planned_artifact.delivery_surface, planned_artifact.artifact_id
        ),
    };
    vec![
        capture_hint,
        format!(
            "remove or revert `{}` from `{}` if downstream validation fails",
            planned_artifact.artifact_id, planned_artifact.delivery_surface
        ),
        "keep candidate ids and source-run references attached to the rollback record".to_owned(),
    ]
}

fn build_runtime_capability_promotion_provenance(
    artifacts: &[RuntimeCapabilityArtifactDocument],
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> RuntimeCapabilityPromotionProvenance {
    let mut ordered_artifacts = artifacts.to_vec();
    sort_runtime_capability_artifacts(&mut ordered_artifacts);

    RuntimeCapabilityPromotionProvenance {
        candidate_ids: ordered_artifacts
            .iter()
            .map(|artifact| artifact.candidate_id.clone())
            .collect(),
        source_run_ids: ordered_artifacts
            .iter()
            .map(|artifact| artifact.source_run.run_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        experiment_ids: ordered_artifacts
            .iter()
            .map(|artifact| artifact.source_run.experiment_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        source_run_artifact_paths: ordered_artifacts
            .iter()
            .filter_map(|artifact| artifact.source_run.artifact_path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        latest_candidate_at: evidence.latest_candidate_at.clone(),
        latest_reviewed_at: evidence.latest_reviewed_at.clone(),
    }
}

pub fn render_runtime_capability_promotion_plan_text(
    report: &RuntimeCapabilityPromotionPlanReport,
) -> String {
    [
        format!("family_id={}", report.family_id),
        format!("promotable={}", report.promotable),
        format!(
            "readiness={}",
            render_family_readiness_status(report.readiness.status)
        ),
        format!(
            "target={}",
            render_target(report.planned_artifact.target_kind)
        ),
        format!("artifact_kind={}", report.planned_artifact.artifact_kind),
        format!("artifact_id={}", report.planned_artifact.artifact_id),
        format!(
            "delivery_surface={}",
            report.planned_artifact.delivery_surface
        ),
        format!("target_summary={}", report.planned_artifact.summary),
        format!("bounded_scope={}", report.planned_artifact.bounded_scope),
        format!(
            "required_capabilities={}",
            render_string_values(&report.planned_artifact.required_capabilities)
        ),
        format!(
            "tags={}",
            render_string_values(&report.planned_artifact.tags)
        ),
        format!(
            "delta_evidence_candidates={}",
            report.evidence.delta_candidate_count
        ),
        format!(
            "delta_changed_surfaces={}",
            render_string_values(&report.evidence.changed_surfaces)
        ),
        format!(
            "blockers={}",
            render_family_readiness_checks(&report.blockers)
        ),
        format!(
            "checks={}",
            render_family_readiness_checks(&report.readiness.checks)
        ),
        format!(
            "approval_checklist={}",
            render_string_values_with_separator(&report.approval_checklist, " | ")
        ),
        format!(
            "rollback_hints={}",
            render_string_values_with_separator(&report.rollback_hints, " | ")
        ),
        format!(
            "provenance_candidate_ids={}",
            render_string_values(&report.provenance.candidate_ids)
        ),
        format!(
            "provenance_source_run_ids={}",
            render_string_values(&report.provenance.source_run_ids)
        ),
        format!(
            "provenance_experiment_ids={}",
            render_string_values(&report.provenance.experiment_ids)
        ),
        format!(
            "provenance_source_run_artifact_paths={}",
            render_string_values_with_separator(
                &report.provenance.source_run_artifact_paths,
                " | "
            )
        ),
    ]
    .join("\n")
}

fn render_family_readiness_checks(checks: &[RuntimeCapabilityFamilyReadinessCheck]) -> String {
    if checks.is_empty() {
        "-".to_owned()
    } else {
        checks
            .iter()
            .map(render_family_readiness_check)
            .collect::<Vec<_>>()
            .join(" | ")
    }
}
