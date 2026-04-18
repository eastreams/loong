use crate::Capability;
use crate::mvp;
use crate::runtime_experiment_cli::{
    RuntimeExperimentArtifactDocument, RuntimeExperimentDecision,
    RuntimeExperimentShowCommandOptions, RuntimeExperimentSnapshotDelta, RuntimeExperimentStatus,
    derive_recorded_snapshot_delta_for_run, execute_runtime_experiment_show_command,
};
use crate::sha2::{self, Digest};
use clap::{Args, Subcommand, ValueEnum};
use kernel::ToolCoreRequest;
use loong_spec::CliResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_CAPABILITY_ARTIFACT_SURFACE: &str = "runtime_capability";
pub const RUNTIME_CAPABILITY_ARTIFACT_PURPOSE: &str = "promotion_candidate_record";
pub const RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE: &str = "runtime_capability_apply_output";
pub const RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE: &str = "draft_promotion_artifact";
pub const RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE: &str =
    "runtime_capability_activation_record";
pub const RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE: &str = "activation_rollback_record";

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
    /// Materialize one governed draft artifact from one promotable capability family
    Apply(RuntimeCapabilityApplyCommandOptions),
    /// Activate one governed draft artifact into the current runtime configuration
    Activate(RuntimeCapabilityActivateCommandOptions),
    /// Roll back one governed activation record from the current runtime configuration
    Rollback(RuntimeCapabilityRollbackCommandOptions),
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

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityApplyCommandOptions {
    #[arg(long)]
    pub root: String,
    #[arg(long)]
    pub family_id: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityActivateCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub artifact: String,
    #[arg(long, default_value_t = false)]
    pub apply: bool,
    #[arg(long, default_value_t = false)]
    pub replace: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityRollbackCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub record: String,
    #[arg(long, default_value_t = false)]
    pub apply: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityTarget {
    ManagedSkill,
    ProgrammaticFlow,
    ProfileNoteAddendum,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilitySourceDecisionRollup {
    pub promoted: usize,
    pub rejected: usize,
    pub undecided: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityPromotionPlannedPayload {
    pub artifact_kind: String,
    pub target: RuntimeCapabilityTarget,
    pub draft_id: String,
    pub summary: String,
    pub review_scope: String,
    pub required_capabilities: Vec<String>,
    pub tags: Vec<String>,
    pub payload: RuntimeCapabilityDraftPayload,
    pub provenance: RuntimeCapabilityPromotionPlannedPayloadProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeCapabilityDraftPayload {
    ManagedSkillBundle { files: BTreeMap<String, String> },
    ProgrammaticFlowSpec { files: BTreeMap<String, String> },
    ProfileNoteAddendum { content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityPromotionPlannedPayloadProvenance {
    pub family_id: String,
    pub accepted_candidate_ids: Vec<String>,
    pub changed_surfaces: Vec<String>,
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
    pub planned_payload: RuntimeCapabilityPromotionPlannedPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityAppliedArtifactDocument {
    pub schema: RuntimeCapabilityArtifactSchema,
    pub family_id: String,
    pub artifact_kind: String,
    pub artifact_id: String,
    pub delivery_surface: String,
    pub target: RuntimeCapabilityTarget,
    pub summary: String,
    pub bounded_scope: String,
    pub required_capabilities: Vec<String>,
    pub tags: Vec<String>,
    pub payload: RuntimeCapabilityDraftPayload,
    pub approval_checklist: Vec<String>,
    pub rollback_hints: Vec<String>,
    pub delta_candidate_count: usize,
    pub changed_surfaces: Vec<String>,
    pub candidate_ids: Vec<String>,
    pub source_run_ids: Vec<String>,
    pub experiment_ids: Vec<String>,
    pub source_run_artifact_paths: Vec<String>,
    pub latest_candidate_at: Option<String>,
    pub latest_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityApplyOutcome {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityApplyReport {
    pub generated_at: String,
    pub root: String,
    pub family_id: String,
    pub output_path: String,
    pub outcome: RuntimeCapabilityApplyOutcome,
    pub applied_artifact: RuntimeCapabilityAppliedArtifactDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityActivateOutcome {
    DryRun,
    Activated,
    AlreadyActivated,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityActivateReport {
    pub generated_at: String,
    pub artifact_path: String,
    pub config_path: String,
    pub artifact_id: String,
    pub target: RuntimeCapabilityTarget,
    pub delivery_surface: String,
    pub activation_surface: String,
    pub target_path: String,
    pub apply_requested: bool,
    pub replace_requested: bool,
    pub outcome: RuntimeCapabilityActivateOutcome,
    pub notes: Vec<String>,
    pub verification: Vec<String>,
    pub rollback_hints: Vec<String>,
    pub activation_record_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeCapabilityActivationRecordDocument {
    pub schema: RuntimeCapabilityArtifactSchema,
    pub activation_id: String,
    pub activated_at: String,
    pub artifact_path: String,
    pub config_path: String,
    pub artifact_id: String,
    pub target: RuntimeCapabilityTarget,
    pub delivery_surface: String,
    pub activation_surface: String,
    pub target_path: String,
    pub verification: Vec<String>,
    pub rollback_hints: Vec<String>,
    pub rollback: RuntimeCapabilityRollbackPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeCapabilityRollbackPayload {
    ManagedSkillBundle {
        previous_files: Option<BTreeMap<String, String>>,
    },
    ProfileNoteAddendum {
        previous_profile: mvp::config::MemoryProfile,
        previous_profile_note: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityRollbackOutcome {
    DryRun,
    RolledBack,
    AlreadyRolledBack,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCapabilityRollbackReport {
    pub generated_at: String,
    pub record_path: String,
    pub config_path: String,
    pub artifact_id: String,
    pub target: RuntimeCapabilityTarget,
    pub activation_surface: String,
    pub target_path: String,
    pub apply_requested: bool,
    pub outcome: RuntimeCapabilityRollbackOutcome,
    pub notes: Vec<String>,
    pub verification: Vec<String>,
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
        RuntimeCapabilityCommands::Apply(options) => {
            let as_json = options.json;
            let report = execute_runtime_capability_apply_command(options)?;
            emit_runtime_capability_apply_report(&report, as_json)
        }
        RuntimeCapabilityCommands::Activate(options) => {
            let as_json = options.json;
            let report = execute_runtime_capability_activate_command(options)?;
            emit_runtime_capability_activate_report(&report, as_json)
        }
        RuntimeCapabilityCommands::Rollback(options) => {
            let as_json = options.json;
            let report = execute_runtime_capability_rollback_command(options)?;
            emit_runtime_capability_rollback_report(&report, as_json)
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
        planned_payload: build_runtime_capability_promotion_planned_payload(
            &family.family_id,
            &planned_artifact,
            &family_artifacts,
            &family.evidence,
        )?,
    })
}

pub fn execute_runtime_capability_apply_command(
    options: RuntimeCapabilityApplyCommandOptions,
) -> CliResult<RuntimeCapabilityApplyReport> {
    let plan_options = RuntimeCapabilityPlanCommandOptions {
        root: options.root,
        family_id: options.family_id,
        json: false,
    };
    let plan = execute_runtime_capability_plan_command(plan_options)?;
    validate_runtime_capability_apply_plan(&plan)?;

    let root = plan.root.clone();
    let family_id = plan.family_id.clone();
    let planned_artifact = &plan.planned_artifact;
    let root_path = PathBuf::from(root.as_str());
    let output_path = resolve_runtime_capability_apply_output_path(&root_path, planned_artifact);
    let applied_artifact = build_runtime_capability_apply_artifact(&plan);
    let outcome = persist_runtime_capability_apply_artifact(&output_path, &applied_artifact)?;
    let canonical_output_path = canonicalize_existing_path(&output_path)?;

    Ok(RuntimeCapabilityApplyReport {
        generated_at: now_rfc3339()?,
        root,
        family_id,
        output_path: canonical_output_path,
        outcome,
        applied_artifact,
    })
}

pub fn execute_runtime_capability_activate_command(
    options: RuntimeCapabilityActivateCommandOptions,
) -> CliResult<RuntimeCapabilityActivateReport> {
    let artifact_path = Path::new(options.artifact.as_str());
    let applied_artifact = load_runtime_capability_apply_artifact(artifact_path)?;
    let canonical_artifact_path = canonicalize_existing_path(artifact_path)?;

    match applied_artifact.target {
        RuntimeCapabilityTarget::ManagedSkill => execute_runtime_capability_activate_managed_skill(
            options,
            canonical_artifact_path,
            applied_artifact,
        ),
        RuntimeCapabilityTarget::ProfileNoteAddendum => {
            execute_runtime_capability_activate_profile_note_addendum(
                options,
                canonical_artifact_path,
                applied_artifact,
            )
        }
        RuntimeCapabilityTarget::ProgrammaticFlow => Err(
            "runtime capability activate does not yet support programmatic_flow artifacts because no governed activation surface exists yet".to_owned(),
        ),
    }
}

pub fn execute_runtime_capability_rollback_command(
    options: RuntimeCapabilityRollbackCommandOptions,
) -> CliResult<RuntimeCapabilityRollbackReport> {
    let record_path = Path::new(options.record.as_str());
    let activation_record = load_runtime_capability_activation_record(record_path)?;
    let canonical_record_path = canonicalize_existing_path(record_path)?;

    match activation_record.target {
        RuntimeCapabilityTarget::ManagedSkill => execute_runtime_capability_rollback_managed_skill(
            options,
            canonical_record_path,
            activation_record,
        ),
        RuntimeCapabilityTarget::ProfileNoteAddendum => {
            execute_runtime_capability_rollback_profile_note_addendum(
                options,
                canonical_record_path,
                activation_record,
            )
        }
        RuntimeCapabilityTarget::ProgrammaticFlow => Err(
            "runtime capability rollback does not yet support programmatic_flow activation records because no governed activation surface exists yet".to_owned(),
        ),
    }
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

fn emit_runtime_capability_apply_report(
    report: &RuntimeCapabilityApplyReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability apply report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_apply_text(report));
    Ok(())
}

fn emit_runtime_capability_activate_report(
    report: &RuntimeCapabilityActivateReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability activate report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_activate_text(report));
    Ok(())
}

fn emit_runtime_capability_rollback_report(
    report: &RuntimeCapabilityRollbackReport,
    as_json: bool,
) -> CliResult<()> {
    if as_json {
        let pretty = serde_json::to_string_pretty(report).map_err(|error| {
            format!("serialize runtime capability rollback report failed: {error}")
        })?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_capability_rollback_text(report));
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
    Ok(hex::encode(sha2::Sha256::digest(encoded)))
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
    let checks = vec![
        review_consensus,
        stability,
        accepted_source_integrity,
        warning_pressure,
    ];
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
    Ok(hex::encode(sha2::Sha256::digest(encoded)))
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

fn validate_runtime_capability_apply_plan(
    plan: &RuntimeCapabilityPromotionPlanReport,
) -> CliResult<()> {
    if plan.promotable {
        return Ok(());
    }

    let readiness = render_family_readiness_status(plan.readiness.status);
    let blockers = render_family_readiness_checks(&plan.blockers);
    let error = format!(
        "runtime capability family `{}` is not promotable for apply; readiness={} blockers={}",
        plan.family_id, readiness, blockers
    );
    Err(error)
}

fn resolve_runtime_capability_apply_output_path(
    root: &Path,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
) -> PathBuf {
    let delivery_surface = planned_artifact.delivery_surface.as_str();
    let artifact_id = planned_artifact.artifact_id.as_str();
    let artifact_file_name = format!("{artifact_id}.json");
    root.join(delivery_surface).join(artifact_file_name)
}

fn build_runtime_capability_apply_artifact(
    plan: &RuntimeCapabilityPromotionPlanReport,
) -> RuntimeCapabilityAppliedArtifactDocument {
    let planned_artifact = &plan.planned_artifact;
    let provenance = &plan.provenance;
    let evidence = &plan.evidence;
    let planned_payload = &plan.planned_payload;

    RuntimeCapabilityAppliedArtifactDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE.to_owned(),
        },
        family_id: plan.family_id.clone(),
        artifact_kind: planned_payload.artifact_kind.clone(),
        artifact_id: planned_payload.draft_id.clone(),
        delivery_surface: planned_artifact.delivery_surface.clone(),
        target: planned_payload.target,
        summary: planned_payload.summary.clone(),
        bounded_scope: planned_payload.review_scope.clone(),
        required_capabilities: planned_payload.required_capabilities.clone(),
        tags: planned_payload.tags.clone(),
        payload: planned_payload.payload.clone(),
        approval_checklist: plan.approval_checklist.clone(),
        rollback_hints: plan.rollback_hints.clone(),
        delta_candidate_count: evidence.delta_candidate_count,
        changed_surfaces: evidence.changed_surfaces.clone(),
        candidate_ids: planned_payload.provenance.accepted_candidate_ids.clone(),
        source_run_ids: provenance.source_run_ids.clone(),
        experiment_ids: provenance.experiment_ids.clone(),
        source_run_artifact_paths: provenance.source_run_artifact_paths.clone(),
        latest_candidate_at: provenance.latest_candidate_at.clone(),
        latest_reviewed_at: provenance.latest_reviewed_at.clone(),
    }
}

fn load_runtime_capability_apply_artifact(
    path: &Path,
) -> CliResult<RuntimeCapabilityAppliedArtifactDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability apply artifact {} failed: {error}",
            path.display()
        )
    })?;
    let artifact = serde_json::from_str::<RuntimeCapabilityAppliedArtifactDocument>(&raw).map_err(
        |error| {
            format!(
                "decode runtime capability apply artifact {} failed: {error}",
                path.display()
            )
        },
    )?;
    validate_runtime_capability_apply_artifact_schema(&artifact, path)?;
    Ok(artifact)
}

fn validate_runtime_capability_apply_artifact_schema(
    artifact: &RuntimeCapabilityAppliedArtifactDocument,
    path: &Path,
) -> CliResult<()> {
    let schema = &artifact.schema;
    if schema.version != RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime capability apply artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            schema.version,
            RUNTIME_CAPABILITY_APPLY_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    if schema.surface != RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE {
        return Err(format!(
            "runtime capability apply artifact {} uses unsupported schema surface {}; expected {}",
            path.display(),
            schema.surface,
            RUNTIME_CAPABILITY_APPLY_ARTIFACT_SURFACE
        ));
    }
    if schema.purpose != RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE {
        return Err(format!(
            "runtime capability apply artifact {} uses unsupported schema purpose {}; expected {}",
            path.display(),
            schema.purpose,
            RUNTIME_CAPABILITY_APPLY_ARTIFACT_PURPOSE
        ));
    }
    Ok(())
}

fn persist_runtime_capability_apply_artifact(
    output_path: &Path,
    artifact: &RuntimeCapabilityAppliedArtifactDocument,
) -> CliResult<RuntimeCapabilityApplyOutcome> {
    let write_result = write_pretty_json_file_create_new(output_path, artifact);
    match write_result {
        Ok(()) => Ok(RuntimeCapabilityApplyOutcome::Applied),
        Err(error) if error.contains("already exists") => {
            let existing_artifact = load_runtime_capability_apply_artifact(output_path)?;
            if existing_artifact == *artifact {
                return Ok(RuntimeCapabilityApplyOutcome::AlreadyApplied);
            }

            let message = format!(
                "runtime capability apply output {} already exists with different content",
                output_path.display()
            );
            Err(message)
        }
        Err(error) => Err(error),
    }
}

fn write_pretty_json_file_create_new(path: &Path, value: &impl Serialize) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime capability apply artifact directory {} failed: {error}",
                parent.display()
            )
        })?;
    }

    let encoded = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize runtime capability apply artifact failed: {error}"))?;
    let temp_path = runtime_capability_apply_temp_path(path);
    let mut temp_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            format!(
                "write runtime capability apply artifact {} failed: {error}",
                path.display()
            )
        })?;
    let write_result = temp_file.write_all(encoded.as_slice());
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "write runtime capability apply artifact {} failed: {error}",
            path.display()
        ));
    }

    let sync_result = temp_file.sync_all();
    if let Err(error) = sync_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "write runtime capability apply artifact {} failed: {error}",
            path.display()
        ));
    }

    drop(temp_file);

    let publish_result = fs::hard_link(&temp_path, path);
    let _ = fs::remove_file(&temp_path);
    match publish_result {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err(format!(
                "runtime capability apply artifact {} already exists",
                path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "write runtime capability apply artifact {} failed: {error}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn execute_runtime_capability_activate_managed_skill(
    options: RuntimeCapabilityActivateCommandOptions,
    artifact_path: String,
    applied_artifact: RuntimeCapabilityAppliedArtifactDocument,
) -> CliResult<RuntimeCapabilityActivateReport> {
    if options.replace && !options.apply {
        return Err("runtime capability activate --replace requires --apply".to_owned());
    }

    let RuntimeCapabilityAppliedArtifactDocument {
        artifact_id,
        target,
        delivery_surface,
        payload,
        rollback_hints,
        ..
    } = applied_artifact;
    let payload = match payload {
        RuntimeCapabilityDraftPayload::ManagedSkillBundle { files } => files,
        RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { .. }
        | RuntimeCapabilityDraftPayload::ProfileNoteAddendum { .. } => {
            return Err(
                "runtime capability activate expected a managed skill bundle payload".to_owned(),
            );
        }
    };

    let (resolved_config_path, config) = mvp::config::load(options.config.as_deref())?;
    let tool_runtime =
        build_runtime_capability_activation_tool_runtime(&resolved_config_path, &config, true);
    let install_root = resolve_runtime_capability_activation_install_root(&tool_runtime)?;
    let target_path = install_root.join(artifact_id.as_str());
    let previous_files = collect_runtime_capability_bundle_files(target_path.as_path())?;
    let already_matches =
        managed_skill_payload_matches_install_root(&payload, target_path.as_path())?;
    let dry_run_target_path = canonicalize_optional_path(target_path.as_path())?;
    let dry_run_verification =
        build_managed_skill_activation_verification_hints(target_path.as_path(), payload.len());

    if !options.apply {
        let notes = vec![
            "activation is dry-run by default".to_owned(),
            "managed skill activation reuses external_skills.install under a governed runtime config"
                .to_owned(),
        ];
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: resolved_config_path.display().to_string(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "external_skills.install".to_owned(),
            target_path: dry_run_target_path,
            apply_requested: false,
            replace_requested: options.replace,
            outcome: RuntimeCapabilityActivateOutcome::DryRun,
            notes,
            verification: dry_run_verification,
            rollback_hints,
            activation_record_path: None,
        });
    }

    if already_matches {
        let notes = vec!["managed skill already matches the applied draft payload".to_owned()];
        let verified_target_path = canonicalize_existing_path(target_path.as_path())?;
        let verification =
            verify_managed_skill_activation_state(&artifact_id, target_path.as_path(), &payload)?;
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: resolved_config_path.display().to_string(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "external_skills.install".to_owned(),
            target_path: verified_target_path,
            apply_requested: true,
            replace_requested: options.replace,
            outcome: RuntimeCapabilityActivateOutcome::AlreadyActivated,
            notes,
            verification,
            rollback_hints,
            activation_record_path: None,
        });
    }

    let staging_base_root = resolve_runtime_capability_activation_staging_base_root(&tool_runtime)?;
    let staging_root =
        write_runtime_capability_draft_files_to_staging(&payload, staging_base_root.as_path())?;
    let staging_path = staging_root.display().to_string();
    let install_payload = json!({
        "path": staging_path,
        "skill_id": artifact_id,
        "replace": options.replace,
    });
    let install_request = ToolCoreRequest {
        tool_name: "external_skills.install".to_owned(),
        payload: install_payload,
    };
    let install_result = mvp::tools::execute_tool_core_with_config(install_request, &tool_runtime);
    let cleanup_result = fs::remove_dir_all(&staging_root);
    if let Err(error) = cleanup_result {
        let cleanup_error = format!(
            "cleanup managed skill staging root {} failed: {error}",
            staging_root.display()
        );
        return Err(cleanup_error);
    }
    install_result
        .map_err(|error| format!("activate managed skill `{}` failed: {error}", artifact_id))?;
    let verification =
        verify_managed_skill_activation_state(&artifact_id, target_path.as_path(), &payload)?;
    let activated_target_path = canonicalize_existing_path(target_path.as_path())?;
    let activation_record = build_runtime_capability_managed_skill_activation_record(
        artifact_path.as_str(),
        resolved_config_path.as_path(),
        artifact_id.as_str(),
        target,
        delivery_surface.as_str(),
        "external_skills.install",
        activated_target_path.as_str(),
        &verification,
        &rollback_hints,
        previous_files,
    )?;
    let activation_record_path = build_runtime_capability_activation_record_path(
        Path::new(artifact_path.as_str()),
        artifact_id.as_str(),
    )?;
    if let Err(error) = persist_runtime_capability_activation_record(
        activation_record_path.as_path(),
        &activation_record,
    ) {
        let rollback_result = rollback_managed_skill_activation_state(
            resolved_config_path.as_path(),
            config,
            artifact_id.as_str(),
            target_path.as_path(),
            activation_record.rollback.clone(),
        );
        if let Err(rollback_error) = rollback_result {
            return Err(format!(
                "persist runtime capability activation record {} failed: {error}; managed skill rollback also failed: {rollback_error}",
                activation_record_path.display()
            ));
        }
        return Err(format!(
            "persist runtime capability activation record {} failed after reverting managed skill activation: {error}",
            activation_record_path.display()
        ));
    }
    let canonical_activation_record_path =
        canonicalize_existing_path(activation_record_path.as_path())?;

    let notes =
        vec!["managed skill installed into the governed external skills runtime".to_owned()];
    Ok(RuntimeCapabilityActivateReport {
        generated_at: now_rfc3339()?,
        artifact_path,
        config_path: resolved_config_path.display().to_string(),
        artifact_id,
        target,
        delivery_surface,
        activation_surface: "external_skills.install".to_owned(),
        target_path: activated_target_path,
        apply_requested: true,
        replace_requested: options.replace,
        outcome: RuntimeCapabilityActivateOutcome::Activated,
        notes,
        verification,
        rollback_hints,
        activation_record_path: Some(canonical_activation_record_path),
    })
}

fn execute_runtime_capability_activate_profile_note_addendum(
    options: RuntimeCapabilityActivateCommandOptions,
    artifact_path: String,
    applied_artifact: RuntimeCapabilityAppliedArtifactDocument,
) -> CliResult<RuntimeCapabilityActivateReport> {
    if options.replace {
        return Err(
            "runtime capability activate --replace is not supported for profile_note_addendum artifacts"
                .to_owned(),
        );
    }

    let RuntimeCapabilityAppliedArtifactDocument {
        artifact_id,
        target,
        delivery_surface,
        payload,
        rollback_hints,
        ..
    } = applied_artifact;
    let addendum = match payload {
        RuntimeCapabilityDraftPayload::ProfileNoteAddendum { content } => content,
        RuntimeCapabilityDraftPayload::ManagedSkillBundle { .. }
        | RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { .. } => {
            return Err(
                "runtime capability activate expected a profile note addendum payload".to_owned(),
            );
        }
    };

    let (resolved_config_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let previous_profile = config.memory.profile;
    let previous_profile_note = config.memory.profile_note.clone();
    let merged_profile_note = mvp::migration::merge_profile_note_addendum(
        config.memory.profile_note.as_deref(),
        addendum.as_str(),
    );
    let canonical_config_path = canonicalize_optional_path(resolved_config_path.as_path())?;
    let dry_run_verification = build_profile_note_activation_verification_hints(
        resolved_config_path.as_path(),
        addendum.as_str(),
    );

    if !options.apply {
        let note = if merged_profile_note.is_some() {
            "profile note activation would append the advisory addendum".to_owned()
        } else {
            "profile note already contains the advisory addendum".to_owned()
        };
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: canonical_config_path.clone(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "config.memory.profile_note".to_owned(),
            target_path: canonical_config_path,
            apply_requested: false,
            replace_requested: false,
            outcome: RuntimeCapabilityActivateOutcome::DryRun,
            notes: vec![note],
            verification: dry_run_verification,
            rollback_hints,
            activation_record_path: None,
        });
    }

    let Some(merged_profile_note) = merged_profile_note else {
        let verification = verify_profile_note_addendum_activation_state(
            resolved_config_path.as_path(),
            addendum.as_str(),
        )?;
        return Ok(RuntimeCapabilityActivateReport {
            generated_at: now_rfc3339()?,
            artifact_path,
            config_path: canonical_config_path.clone(),
            artifact_id,
            target,
            delivery_surface,
            activation_surface: "config.memory.profile_note".to_owned(),
            target_path: canonical_config_path,
            apply_requested: true,
            replace_requested: false,
            outcome: RuntimeCapabilityActivateOutcome::AlreadyActivated,
            notes: vec!["profile note already contains the advisory addendum".to_owned()],
            verification,
            rollback_hints,
            activation_record_path: None,
        });
    };

    config.memory.profile = mvp::config::MemoryProfile::ProfilePlusWindow;
    config.memory.profile_note = Some(merged_profile_note);
    let resolved_config_path_string = resolved_config_path.display().to_string();
    mvp::config::write(Some(resolved_config_path_string.as_str()), &config, true)?;
    let verification = verify_profile_note_addendum_activation_state(
        resolved_config_path.as_path(),
        addendum.as_str(),
    )?;
    let canonical_record_target_path = canonical_config_path.clone();
    let activation_record = build_runtime_capability_profile_note_activation_record(
        artifact_path.as_str(),
        resolved_config_path.as_path(),
        artifact_id.as_str(),
        target,
        delivery_surface.as_str(),
        "config.memory.profile_note",
        canonical_record_target_path.as_str(),
        &verification,
        &rollback_hints,
        previous_profile,
        previous_profile_note,
    )?;
    let activation_record_path = build_runtime_capability_activation_record_path(
        Path::new(artifact_path.as_str()),
        artifact_id.as_str(),
    )?;
    if let Err(error) = persist_runtime_capability_activation_record(
        activation_record_path.as_path(),
        &activation_record,
    ) {
        let rollback_result = rollback_profile_note_addendum_activation_state(
            resolved_config_path.as_path(),
            previous_profile,
            activation_record.rollback.clone(),
        );
        if let Err(rollback_error) = rollback_result {
            return Err(format!(
                "persist runtime capability activation record {} failed: {error}; profile note rollback also failed: {rollback_error}",
                activation_record_path.display()
            ));
        }
        return Err(format!(
            "persist runtime capability activation record {} failed after reverting profile note activation: {error}",
            activation_record_path.display()
        ));
    }
    let canonical_activation_record_path =
        canonicalize_existing_path(activation_record_path.as_path())?;

    Ok(RuntimeCapabilityActivateReport {
        generated_at: now_rfc3339()?,
        artifact_path,
        config_path: canonical_config_path.clone(),
        artifact_id,
        target,
        delivery_surface,
        activation_surface: "config.memory.profile_note".to_owned(),
        target_path: canonical_config_path,
        apply_requested: true,
        replace_requested: false,
        outcome: RuntimeCapabilityActivateOutcome::Activated,
        notes: vec![
            "profile_note_addendum activation also enforces profile_plus_window memory mode"
                .to_owned(),
        ],
        verification,
        rollback_hints,
        activation_record_path: Some(canonical_activation_record_path),
    })
}

fn build_runtime_capability_activation_tool_runtime(
    resolved_config_path: &Path,
    config: &mvp::config::LoongConfig,
    external_skills_enabled: bool,
) -> mvp::tools::runtime_config::ToolRuntimeConfig {
    let mut adjusted_config = config.clone();
    adjusted_config.external_skills.enabled = external_skills_enabled;
    mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &adjusted_config,
        Some(resolved_config_path),
    )
}

fn resolve_runtime_capability_activation_install_root(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> CliResult<PathBuf> {
    if let Some(path) = tool_runtime.external_skills.install_root.clone() {
        return Ok(path);
    }

    let file_root = match tool_runtime.file_root.clone() {
        Some(path) => path,
        None => std::env::current_dir().map_err(|error| {
            format!("read current dir for managed skill activation failed: {error}")
        })?,
    };
    Ok(file_root.join("external-skills-installed"))
}

fn resolve_runtime_capability_activation_staging_base_root(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> CliResult<PathBuf> {
    let file_root = match tool_runtime.file_root.clone() {
        Some(path) => path,
        None => std::env::current_dir()
            .map_err(|error| format!("read current dir for activation staging failed: {error}"))?,
    };
    let staging_base_root = file_root.join(".runtime-capability-staging");
    Ok(staging_base_root)
}

fn managed_skill_payload_matches_install_root(
    files: &BTreeMap<String, String>,
    install_root: &Path,
) -> CliResult<bool> {
    if !install_root.exists() {
        return Ok(false);
    }

    for (relative_path, expected_contents) in files {
        let normalized_relative_path =
            normalize_runtime_capability_relative_path(relative_path.as_str())?;
        let candidate_path = install_root.join(normalized_relative_path.as_path());
        if !candidate_path.exists() {
            return Ok(false);
        }
        let actual_contents = fs::read_to_string(&candidate_path).map_err(|error| {
            format!(
                "read activated managed skill file {} failed: {error}",
                candidate_path.display()
            )
        })?;
        if actual_contents != *expected_contents {
            return Ok(false);
        }
    }

    Ok(true)
}

fn write_runtime_capability_draft_files_to_staging(
    files: &BTreeMap<String, String>,
    staging_base_root: &Path,
) -> CliResult<PathBuf> {
    let staging_root =
        build_runtime_capability_temp_dir(staging_base_root, "activate-managed-skill");
    fs::create_dir_all(&staging_root).map_err(|error| {
        format!(
            "create runtime capability staging directory {} failed: {error}",
            staging_root.display()
        )
    })?;

    for (relative_path, contents) in files {
        let normalized_relative_path =
            normalize_runtime_capability_relative_path(relative_path.as_str())?;
        let output_path = staging_root.join(normalized_relative_path.as_path());
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "create runtime capability draft parent {} failed: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(&output_path, contents).map_err(|error| {
            format!(
                "write runtime capability draft file {} failed: {error}",
                output_path.display()
            )
        })?;
    }

    Ok(staging_root)
}

fn normalize_runtime_capability_relative_path(raw: &str) -> CliResult<PathBuf> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "runtime capability draft file path {} must be relative",
            path.display()
        ));
    }

    let mut normalized_path = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(value) => normalized_path.push(value),
            std::path::Component::ParentDir => {
                return Err(format!(
                    "runtime capability draft file path {} cannot escape its bundle root",
                    path.display()
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "runtime capability draft file path {} must stay relative",
                    path.display()
                ));
            }
        }
    }

    if normalized_path.as_os_str().is_empty() {
        return Err("runtime capability draft file path cannot be empty".to_owned());
    }

    Ok(normalized_path)
}

fn build_runtime_capability_temp_dir(staging_base_root: &Path, label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let process_id = std::process::id();
    let directory_name = format!("loong-runtime-capability-{label}-{process_id}-{timestamp}");
    staging_base_root.join(directory_name)
}

fn canonicalize_optional_path(path: &Path) -> CliResult<String> {
    if path.exists() {
        return canonicalize_existing_path(path);
    }
    Ok(path.display().to_string())
}

fn runtime_capability_apply_temp_path(path: &Path) -> PathBuf {
    let parent = path.parent();
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());
    let file_name = file_name.unwrap_or("runtime-capability-apply.json");
    let process_id = std::process::id();
    let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let temp_name = format!(".{file_name}.tmp.{process_id}.{timestamp}");
    parent.join(temp_name)
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

fn build_managed_skill_activation_verification_hints(
    target_path: &Path,
    file_count: usize,
) -> Vec<String> {
    let target_display = target_path.display().to_string();
    let verify_bundle = format!(
        "verify {target_display} matches the applied managed skill bundle with {file_count} file(s)"
    );
    vec![verify_bundle]
}

fn verify_managed_skill_activation_state(
    artifact_id: &str,
    target_path: &Path,
    files: &BTreeMap<String, String>,
) -> CliResult<Vec<String>> {
    let matches_payload = managed_skill_payload_matches_install_root(files, target_path)?;
    if !matches_payload {
        let target_display = target_path.display().to_string();
        let error = format!(
            "activate managed skill `{artifact_id}` did not leave an installed bundle at {target_display} that matches the applied draft payload"
        );
        return Err(error);
    }

    let target_display = target_path.display().to_string();
    let file_count = files.len();
    let verification = format!(
        "verified {target_display} matches the applied managed skill bundle with {file_count} file(s)"
    );
    Ok(vec![verification])
}

fn build_profile_note_activation_verification_hints(
    config_path: &Path,
    addendum: &str,
) -> Vec<String> {
    let config_display = config_path.display().to_string();
    let addendum_length = addendum.chars().count();
    let verify_profile =
        format!("verify {config_display} sets memory.profile=profile_plus_window after activation");
    let verify_addendum = format!(
        "verify {config_display} persists the {addendum_length}-character advisory addendum in memory.profile_note"
    );
    vec![verify_profile, verify_addendum]
}

fn verify_profile_note_addendum_activation_state(
    config_path: &Path,
    addendum: &str,
) -> CliResult<Vec<String>> {
    let config_path_text = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_text.as_str()))?;
    let (_, reloaded_config) = load_result;
    if reloaded_config.memory.profile != mvp::config::MemoryProfile::ProfilePlusWindow {
        let error = format!(
            "runtime capability activate expected {} to set memory.profile=profile_plus_window",
            config_path.display()
        );
        return Err(error);
    }

    let persisted_profile_note = match reloaded_config.memory.profile_note.as_deref() {
        Some(value) => value,
        None => {
            let error = format!(
                "runtime capability activate expected {} to persist memory.profile_note",
                config_path.display()
            );
            return Err(error);
        }
    };
    let merged_profile_note =
        mvp::migration::merge_profile_note_addendum(Some(persisted_profile_note), addendum);
    if merged_profile_note.is_some() {
        let error = format!(
            "runtime capability activate expected {} to contain the advisory addendum in memory.profile_note",
            config_path.display()
        );
        return Err(error);
    }

    let config_display = config_path.display().to_string();
    let addendum_length = addendum.chars().count();
    let profile_verification =
        format!("verified {config_display} sets memory.profile=profile_plus_window");
    let note_verification = format!(
        "verified {config_display} persists the {addendum_length}-character advisory addendum in memory.profile_note"
    );
    let verification = vec![profile_verification, note_verification];
    Ok(verification)
}

fn build_runtime_capability_activation_record_path(
    artifact_path: &Path,
    artifact_id: &str,
) -> CliResult<PathBuf> {
    let artifact_parent = artifact_path.parent().ok_or_else(|| {
        format!(
            "runtime capability artifact {} has no parent directory for activation records",
            artifact_path.display()
        )
    })?;
    let root_path = artifact_parent
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| artifact_parent.to_path_buf());
    let record_root = root_path.join("runtime-capability-activation");
    let timestamp = now_rfc3339()?;
    let normalized_timestamp = timestamp.replace(':', "-");
    let file_name = format!("{artifact_id}-{normalized_timestamp}.json");
    let record_path = record_root.join(file_name);
    Ok(record_path)
}

fn build_runtime_capability_managed_skill_activation_record(
    artifact_path: &str,
    config_path: &Path,
    artifact_id: &str,
    target: RuntimeCapabilityTarget,
    delivery_surface: &str,
    activation_surface: &str,
    target_path: &str,
    verification: &[String],
    rollback_hints: &[String],
    previous_files: Option<BTreeMap<String, String>>,
) -> CliResult<RuntimeCapabilityActivationRecordDocument> {
    let activation_id =
        build_runtime_capability_activation_id(artifact_id, target, target_path, verification)?;
    let rollback = RuntimeCapabilityRollbackPayload::ManagedSkillBundle { previous_files };
    let record = RuntimeCapabilityActivationRecordDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE.to_owned(),
        },
        activation_id,
        activated_at: now_rfc3339()?,
        artifact_path: artifact_path.to_owned(),
        config_path: config_path.display().to_string(),
        artifact_id: artifact_id.to_owned(),
        target,
        delivery_surface: delivery_surface.to_owned(),
        activation_surface: activation_surface.to_owned(),
        target_path: target_path.to_owned(),
        verification: verification.to_vec(),
        rollback_hints: rollback_hints.to_vec(),
        rollback,
    };
    Ok(record)
}

fn build_runtime_capability_profile_note_activation_record(
    artifact_path: &str,
    config_path: &Path,
    artifact_id: &str,
    target: RuntimeCapabilityTarget,
    delivery_surface: &str,
    activation_surface: &str,
    target_path: &str,
    verification: &[String],
    rollback_hints: &[String],
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<String>,
) -> CliResult<RuntimeCapabilityActivationRecordDocument> {
    let activation_id =
        build_runtime_capability_activation_id(artifact_id, target, target_path, verification)?;
    let rollback = RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
        previous_profile,
        previous_profile_note,
    };
    let record = RuntimeCapabilityActivationRecordDocument {
        schema: RuntimeCapabilityArtifactSchema {
            version: RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION,
            surface: RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE.to_owned(),
            purpose: RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE.to_owned(),
        },
        activation_id,
        activated_at: now_rfc3339()?,
        artifact_path: artifact_path.to_owned(),
        config_path: config_path.display().to_string(),
        artifact_id: artifact_id.to_owned(),
        target,
        delivery_surface: delivery_surface.to_owned(),
        activation_surface: activation_surface.to_owned(),
        target_path: target_path.to_owned(),
        verification: verification.to_vec(),
        rollback_hints: rollback_hints.to_vec(),
        rollback,
    };
    Ok(record)
}

fn build_runtime_capability_activation_id(
    artifact_id: &str,
    target: RuntimeCapabilityTarget,
    target_path: &str,
    verification: &[String],
) -> CliResult<String> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(artifact_id.as_bytes());
    hasher.update(render_target(target).as_bytes());
    hasher.update(target_path.as_bytes());
    for item in verification {
        hasher.update(item.as_bytes());
    }
    let digest = hasher.finalize();
    let activation_digest = hex::encode(digest);
    let activation_id = format!("runtime-capability-activation-{activation_digest}");
    Ok(activation_id)
}

fn persist_runtime_capability_activation_record(
    path: &Path,
    record: &RuntimeCapabilityActivationRecordDocument,
) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create runtime capability activation record directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_vec_pretty(record).map_err(|error| {
        format!("serialize runtime capability activation record failed: {error}")
    })?;
    fs::write(path, encoded).map_err(|error| {
        format!(
            "write runtime capability activation record {} failed: {error}",
            path.display()
        )
    })?;
    Ok(())
}

fn load_runtime_capability_activation_record(
    path: &Path,
) -> CliResult<RuntimeCapabilityActivationRecordDocument> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime capability activation record {} failed: {error}",
            path.display()
        )
    })?;
    let record = serde_json::from_str::<RuntimeCapabilityActivationRecordDocument>(&raw).map_err(
        |error| {
            format!(
                "decode runtime capability activation record {} failed: {error}",
                path.display()
            )
        },
    )?;
    validate_runtime_capability_activation_record_schema(&record, path)?;
    Ok(record)
}

fn validate_runtime_capability_activation_record_schema(
    record: &RuntimeCapabilityActivationRecordDocument,
    path: &Path,
) -> CliResult<()> {
    let schema = &record.schema;
    if schema.version != RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime capability activation record {} uses unsupported schema version {}; expected {}",
            path.display(),
            schema.version,
            RUNTIME_CAPABILITY_ACTIVATION_RECORD_JSON_SCHEMA_VERSION
        ));
    }
    if schema.surface != RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE {
        return Err(format!(
            "runtime capability activation record {} uses unsupported schema surface {}; expected {}",
            path.display(),
            schema.surface,
            RUNTIME_CAPABILITY_ACTIVATION_RECORD_SURFACE
        ));
    }
    if schema.purpose != RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE {
        return Err(format!(
            "runtime capability activation record {} uses unsupported schema purpose {}; expected {}",
            path.display(),
            schema.purpose,
            RUNTIME_CAPABILITY_ACTIVATION_RECORD_PURPOSE
        ));
    }
    Ok(())
}

fn collect_runtime_capability_bundle_files(
    root: &Path,
) -> CliResult<Option<BTreeMap<String, String>>> {
    if !root.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(root).map_err(|error| {
        format!(
            "read runtime capability bundle root metadata {} failed: {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "runtime capability bundle root {} must be a directory",
            root.display()
        ));
    }

    let mut files = BTreeMap::new();
    collect_runtime_capability_bundle_files_recursive(root, root, &mut files)?;
    Ok(Some(files))
}

fn collect_runtime_capability_bundle_files_recursive(
    bundle_root: &Path,
    current_root: &Path,
    files: &mut BTreeMap<String, String>,
) -> CliResult<()> {
    let read_dir = fs::read_dir(current_root).map_err(|error| {
        format!(
            "read runtime capability bundle directory {} failed: {error}",
            current_root.display()
        )
    })?;
    let mut entries = Vec::new();
    for entry_result in read_dir {
        let entry = entry_result.map_err(|error| {
            format!(
                "read runtime capability bundle directory entry under {} failed: {error}",
                current_root.display()
            )
        })?;
        entries.push(entry.path());
    }
    entries.sort();

    for entry_path in entries {
        let entry_metadata = fs::metadata(&entry_path).map_err(|error| {
            format!(
                "read runtime capability bundle entry metadata {} failed: {error}",
                entry_path.display()
            )
        })?;
        if entry_metadata.is_dir() {
            collect_runtime_capability_bundle_files_recursive(
                bundle_root,
                entry_path.as_path(),
                files,
            )?;
            continue;
        }
        if !entry_metadata.is_file() {
            continue;
        }
        let relative_path = entry_path.strip_prefix(bundle_root).map_err(|error| {
            format!(
                "derive runtime capability bundle relative path for {} failed: {error}",
                entry_path.display()
            )
        })?;
        let relative_path_text = normalized_path_text(&relative_path.display().to_string());
        let contents = fs::read_to_string(&entry_path).map_err(|error| {
            format!(
                "read runtime capability bundle file {} failed: {error}",
                entry_path.display()
            )
        })?;
        files.insert(relative_path_text, contents);
    }
    Ok(())
}

fn execute_runtime_capability_rollback_managed_skill(
    options: RuntimeCapabilityRollbackCommandOptions,
    record_path: String,
    record: RuntimeCapabilityActivationRecordDocument,
) -> CliResult<RuntimeCapabilityRollbackReport> {
    let RuntimeCapabilityActivationRecordDocument {
        config_path,
        artifact_id,
        target,
        activation_surface,
        target_path,
        rollback,
        ..
    } = record;
    let rollback = match rollback {
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { previous_files } => previous_files,
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum { .. } => {
            return Err(
                "runtime capability rollback expected a managed skill activation record".to_owned(),
            );
        }
    };
    let target_path_buf = PathBuf::from(target_path.as_str());
    let current_files = collect_runtime_capability_bundle_files(target_path_buf.as_path())?;
    let already_rolled_back = current_files == rollback;
    let dry_run_verification = build_managed_skill_rollback_verification_hints(
        target_path_buf.as_path(),
        rollback.as_ref(),
    );

    if !options.apply {
        let note = if already_rolled_back {
            "managed skill already matches the recorded pre-activation state".to_owned()
        } else {
            "managed skill rollback would restore the recorded pre-activation state".to_owned()
        };
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path,
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: false,
            outcome: RuntimeCapabilityRollbackOutcome::DryRun,
            notes: vec![note],
            verification: dry_run_verification,
        });
    }

    if already_rolled_back {
        let verification = verify_managed_skill_rollback_state(
            artifact_id.as_str(),
            target_path_buf.as_path(),
            rollback.as_ref(),
        )?;
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path,
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: true,
            outcome: RuntimeCapabilityRollbackOutcome::AlreadyRolledBack,
            notes: vec![
                "managed skill already matches the recorded pre-activation state".to_owned(),
            ],
            verification,
        });
    }

    let config_override = options.config.unwrap_or(config_path);
    let (resolved_config_path, config) = mvp::config::load(Some(config_override.as_str()))?;
    let rollback_payload = RuntimeCapabilityRollbackPayload::ManagedSkillBundle {
        previous_files: rollback,
    };
    rollback_managed_skill_activation_state(
        resolved_config_path.as_path(),
        config,
        artifact_id.as_str(),
        target_path_buf.as_path(),
        rollback_payload.clone(),
    )?;
    let verification = verify_managed_skill_rollback_state(
        artifact_id.as_str(),
        target_path_buf.as_path(),
        match rollback_payload {
            RuntimeCapabilityRollbackPayload::ManagedSkillBundle { ref previous_files } => {
                previous_files.as_ref()
            }
            RuntimeCapabilityRollbackPayload::ProfileNoteAddendum { .. } => None,
        },
    )?;
    Ok(RuntimeCapabilityRollbackReport {
        generated_at: now_rfc3339()?,
        record_path,
        config_path: resolved_config_path.display().to_string(),
        artifact_id,
        target,
        activation_surface,
        target_path,
        apply_requested: true,
        outcome: RuntimeCapabilityRollbackOutcome::RolledBack,
        notes: vec!["managed skill rollback restored the recorded pre-activation state".to_owned()],
        verification,
    })
}

fn execute_runtime_capability_rollback_profile_note_addendum(
    options: RuntimeCapabilityRollbackCommandOptions,
    record_path: String,
    record: RuntimeCapabilityActivationRecordDocument,
) -> CliResult<RuntimeCapabilityRollbackReport> {
    let RuntimeCapabilityActivationRecordDocument {
        config_path,
        artifact_id,
        target,
        activation_surface,
        target_path,
        rollback,
        ..
    } = record;
    let rollback = match rollback {
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
            previous_profile,
            previous_profile_note,
        } => (previous_profile, previous_profile_note),
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { .. } => {
            return Err(
                "runtime capability rollback expected a profile note activation record".to_owned(),
            );
        }
    };
    let config_override = options.config.unwrap_or(config_path);
    let dry_run_verification = build_profile_note_rollback_verification_hints(
        Path::new(config_override.as_str()),
        rollback.0,
        rollback.1.as_deref(),
    );

    if !options.apply {
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path: config_override,
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: false,
            outcome: RuntimeCapabilityRollbackOutcome::DryRun,
            notes: vec![
                "profile note rollback would restore the recorded pre-activation memory state"
                    .to_owned(),
            ],
            verification: dry_run_verification,
        });
    }

    let (resolved_config_path, _) = mvp::config::load(Some(config_override.as_str()))?;
    let already_rolled_back = profile_note_state_matches(
        resolved_config_path.as_path(),
        rollback.0,
        rollback.1.as_deref(),
    )?;
    if already_rolled_back {
        let verification = verify_profile_note_rollback_state(
            resolved_config_path.as_path(),
            rollback.0,
            rollback.1.as_deref(),
        )?;
        return Ok(RuntimeCapabilityRollbackReport {
            generated_at: now_rfc3339()?,
            record_path,
            config_path: resolved_config_path.display().to_string(),
            artifact_id,
            target,
            activation_surface,
            target_path,
            apply_requested: true,
            outcome: RuntimeCapabilityRollbackOutcome::AlreadyRolledBack,
            notes: vec![
                "profile note already matches the recorded pre-activation memory state".to_owned(),
            ],
            verification,
        });
    }

    let rollback_payload = RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
        previous_profile: rollback.0,
        previous_profile_note: rollback.1.clone(),
    };
    rollback_profile_note_addendum_activation_state(
        resolved_config_path.as_path(),
        rollback.0,
        rollback_payload,
    )?;
    let verification = verify_profile_note_rollback_state(
        resolved_config_path.as_path(),
        rollback.0,
        rollback.1.as_deref(),
    )?;
    Ok(RuntimeCapabilityRollbackReport {
        generated_at: now_rfc3339()?,
        record_path,
        config_path: resolved_config_path.display().to_string(),
        artifact_id,
        target,
        activation_surface,
        target_path,
        apply_requested: true,
        outcome: RuntimeCapabilityRollbackOutcome::RolledBack,
        notes: vec![
            "profile note rollback restored the recorded pre-activation memory state".to_owned(),
        ],
        verification,
    })
}

fn rollback_managed_skill_activation_state(
    resolved_config_path: &Path,
    mut config: mvp::config::LoongConfig,
    artifact_id: &str,
    target_path: &Path,
    rollback: RuntimeCapabilityRollbackPayload,
) -> CliResult<()> {
    let previous_files = match rollback {
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { previous_files } => previous_files,
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum { .. } => {
            return Err(
                "runtime capability rollback expected a managed skill rollback payload".to_owned(),
            );
        }
    };
    config.external_skills.enabled = true;
    config.external_skills.install_root = target_path
        .parent()
        .map(|value| value.display().to_string());
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &config,
        Some(resolved_config_path),
    );

    match previous_files {
        Some(previous_files) => {
            let staging_base_root =
                resolve_runtime_capability_activation_staging_base_root(&tool_runtime)?;
            let staging_root = write_runtime_capability_draft_files_to_staging(
                &previous_files,
                staging_base_root.as_path(),
            )?;
            let staging_path = staging_root.display().to_string();
            let install_request = ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": staging_path,
                    "skill_id": artifact_id,
                    "replace": true,
                }),
            };
            let install_result =
                mvp::tools::execute_tool_core_with_config(install_request, &tool_runtime);
            let cleanup_result = fs::remove_dir_all(&staging_root);
            if let Err(error) = cleanup_result {
                return Err(format!(
                    "cleanup managed skill rollback staging root {} failed: {error}",
                    staging_root.display()
                ));
            }
            install_result.map_err(|error| {
                format!(
                    "restore previous managed skill `{artifact_id}` during rollback failed: {error}"
                )
            })?;
        }
        None => {
            let remove_request = ToolCoreRequest {
                tool_name: "external_skills.remove".to_owned(),
                payload: json!({
                    "skill_id": artifact_id,
                }),
            };
            mvp::tools::execute_tool_core_with_config(remove_request, &tool_runtime).map_err(
                |error| {
                    format!("remove managed skill `{artifact_id}` during rollback failed: {error}")
                },
            )?;
        }
    }
    Ok(())
}

fn rollback_profile_note_addendum_activation_state(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    rollback: RuntimeCapabilityRollbackPayload,
) -> CliResult<()> {
    let previous_profile_note = match rollback {
        RuntimeCapabilityRollbackPayload::ProfileNoteAddendum {
            previous_profile_note,
            ..
        } => previous_profile_note,
        RuntimeCapabilityRollbackPayload::ManagedSkillBundle { .. } => {
            return Err(
                "runtime capability rollback expected a profile note rollback payload".to_owned(),
            );
        }
    };
    let config_path_text = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_text.as_str()))?;
    let (_, mut config) = load_result;
    config.memory.profile = previous_profile;
    config.memory.profile_note = previous_profile_note;
    mvp::config::write(Some(config_path_text.as_str()), &config, true)?;
    Ok(())
}

fn build_managed_skill_rollback_verification_hints(
    target_path: &Path,
    previous_files: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
    let target_display = target_path.display().to_string();
    match previous_files {
        Some(previous_files) => {
            let file_count = previous_files.len();
            let verification = format!(
                "verify {target_display} matches the recorded pre-activation managed skill bundle with {file_count} file(s)"
            );
            vec![verification]
        }
        None => {
            let verification = format!(
                "verify {target_display} is absent after rollback removes the managed skill"
            );
            vec![verification]
        }
    }
}

fn verify_managed_skill_rollback_state(
    artifact_id: &str,
    target_path: &Path,
    previous_files: Option<&BTreeMap<String, String>>,
) -> CliResult<Vec<String>> {
    match previous_files {
        Some(previous_files) => {
            let matches_payload =
                managed_skill_payload_matches_install_root(previous_files, target_path)?;
            if !matches_payload {
                return Err(format!(
                    "runtime capability rollback did not restore managed skill `{artifact_id}` to the recorded pre-activation bundle at {}",
                    target_path.display()
                ));
            }
            let file_count = previous_files.len();
            let verification = format!(
                "verified {} matches the recorded pre-activation managed skill bundle with {file_count} file(s)",
                target_path.display()
            );
            Ok(vec![verification])
        }
        None => {
            if target_path.exists() {
                return Err(format!(
                    "runtime capability rollback expected managed skill `{artifact_id}` to be removed from {}",
                    target_path.display()
                ));
            }
            let verification = format!(
                "verified {} is absent after rollback removed the managed skill",
                target_path.display()
            );
            Ok(vec![verification])
        }
    }
}

fn build_profile_note_rollback_verification_hints(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<&str>,
) -> Vec<String> {
    let config_display = config_path.display().to_string();
    let profile_hint = format!(
        "verify {config_display} restores memory.profile={} during rollback",
        render_memory_profile(previous_profile)
    );
    let note_hint = match previous_profile_note {
        Some(previous_profile_note) => {
            let char_count = previous_profile_note.chars().count();
            format!(
                "verify {config_display} restores the {char_count}-character pre-activation memory.profile_note"
            )
        }
        None => format!("verify {config_display} clears memory.profile_note during rollback"),
    };
    vec![profile_hint, note_hint]
}

fn profile_note_state_matches(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<&str>,
) -> CliResult<bool> {
    let config_path_text = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_text.as_str()))?;
    let (_, config) = load_result;
    if config.memory.profile != previous_profile {
        return Ok(false);
    }
    let current_profile_note = config.memory.profile_note.as_deref();
    Ok(current_profile_note == previous_profile_note)
}

fn verify_profile_note_rollback_state(
    config_path: &Path,
    previous_profile: mvp::config::MemoryProfile,
    previous_profile_note: Option<&str>,
) -> CliResult<Vec<String>> {
    let matches = profile_note_state_matches(config_path, previous_profile, previous_profile_note)?;
    if !matches {
        return Err(format!(
            "runtime capability rollback expected {} to restore the recorded pre-activation memory state",
            config_path.display()
        ));
    }

    let config_display = config_path.display().to_string();
    let profile_verification = format!(
        "verified {config_display} restores memory.profile={}",
        render_memory_profile(previous_profile)
    );
    let note_verification = match previous_profile_note {
        Some(previous_profile_note) => {
            let char_count = previous_profile_note.chars().count();
            format!(
                "verified {config_display} restores the {char_count}-character pre-activation memory.profile_note"
            )
        }
        None => format!("verified {config_display} clears memory.profile_note during rollback"),
    };
    Ok(vec![profile_verification, note_verification])
}

pub fn render_runtime_capability_text(artifact: &RuntimeCapabilityArtifactDocument) -> String {
    let body = [
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
    .join("\n");
    wrap_runtime_capability_surface("capability candidate", body)
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

    wrap_runtime_capability_surface("capability index", lines.join("\n"))
}

pub fn render_runtime_capability_apply_text(report: &RuntimeCapabilityApplyReport) -> String {
    let artifact = &report.applied_artifact;
    let body = [
        format!("family_id={}", report.family_id),
        format!(
            "outcome={}",
            render_runtime_capability_apply_outcome(report.outcome)
        ),
        format!("artifact_kind={}", artifact.artifact_kind),
        format!("artifact_id={}", artifact.artifact_id),
        format!("delivery_surface={}", artifact.delivery_surface),
        format!("output_path={}", report.output_path),
        format!("target={}", render_target(artifact.target)),
        format!("target_summary={}", artifact.summary),
        format!("bounded_scope={}", artifact.bounded_scope),
        format!(
            "required_capabilities={}",
            render_string_values(&artifact.required_capabilities)
        ),
        format!("tags={}", render_string_values(&artifact.tags)),
        format!(
            "approval_checklist={}",
            render_string_values_with_separator(&artifact.approval_checklist, " | ")
        ),
        format!(
            "rollback_hints={}",
            render_string_values_with_separator(&artifact.rollback_hints, " | ")
        ),
        format!("delta_candidate_count={}", artifact.delta_candidate_count),
        format!(
            "changed_surfaces={}",
            render_string_values(&artifact.changed_surfaces)
        ),
        format!(
            "candidate_ids={}",
            render_string_values(&artifact.candidate_ids)
        ),
        format!(
            "source_run_ids={}",
            render_string_values(&artifact.source_run_ids)
        ),
        format!(
            "experiment_ids={}",
            render_string_values(&artifact.experiment_ids)
        ),
        format!(
            "payload={}",
            render_runtime_capability_draft_payload(&artifact.payload)
        ),
    ]
    .join("\n");
    wrap_runtime_capability_surface("capability apply", body)
}

pub fn render_runtime_capability_activate_text(report: &RuntimeCapabilityActivateReport) -> String {
    let body = [
        format!("artifact_path={}", report.artifact_path),
        format!("config_path={}", report.config_path),
        format!("artifact_id={}", report.artifact_id),
        format!("target={}", render_target(report.target)),
        format!("delivery_surface={}", report.delivery_surface),
        format!("activation_surface={}", report.activation_surface),
        format!("target_path={}", report.target_path),
        format!("apply_requested={}", report.apply_requested),
        format!("replace_requested={}", report.replace_requested),
        format!(
            "outcome={}",
            render_runtime_capability_activate_outcome(report.outcome)
        ),
        format!(
            "notes={}",
            render_string_values_with_separator(&report.notes, " | ")
        ),
        format!(
            "verification={}",
            render_string_values_with_separator(&report.verification, " | ")
        ),
        format!(
            "rollback_hints={}",
            render_string_values_with_separator(&report.rollback_hints, " | ")
        ),
        format!(
            "activation_record_path={}",
            report.activation_record_path.as_deref().unwrap_or("-")
        ),
    ]
    .join("\n");
    wrap_runtime_capability_surface("capability activate", body)
}

pub fn render_runtime_capability_rollback_text(report: &RuntimeCapabilityRollbackReport) -> String {
    let body = [
        format!("record_path={}", report.record_path),
        format!("config_path={}", report.config_path),
        format!("artifact_id={}", report.artifact_id),
        format!("target={}", render_target(report.target)),
        format!("activation_surface={}", report.activation_surface),
        format!("target_path={}", report.target_path),
        format!("apply_requested={}", report.apply_requested),
        format!(
            "outcome={}",
            render_runtime_capability_rollback_outcome(report.outcome)
        ),
        format!(
            "notes={}",
            render_string_values_with_separator(&report.notes, " | ")
        ),
        format!(
            "verification={}",
            render_string_values_with_separator(&report.verification, " | ")
        ),
    ]
    .join("\n");
    wrap_runtime_capability_surface("capability rollback", body)
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

fn normalized_path_text(value: &str) -> String {
    value.replace('\\', "/")
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
    }
}

fn render_memory_profile(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
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

fn render_runtime_capability_apply_outcome(outcome: RuntimeCapabilityApplyOutcome) -> &'static str {
    match outcome {
        RuntimeCapabilityApplyOutcome::Applied => "applied",
        RuntimeCapabilityApplyOutcome::AlreadyApplied => "already_applied",
    }
}

fn render_runtime_capability_activate_outcome(
    outcome: RuntimeCapabilityActivateOutcome,
) -> &'static str {
    match outcome {
        RuntimeCapabilityActivateOutcome::DryRun => "dry_run",
        RuntimeCapabilityActivateOutcome::Activated => "activated",
        RuntimeCapabilityActivateOutcome::AlreadyActivated => "already_activated",
    }
}

fn render_runtime_capability_rollback_outcome(
    outcome: RuntimeCapabilityRollbackOutcome,
) -> &'static str {
    match outcome {
        RuntimeCapabilityRollbackOutcome::DryRun => "dry_run",
        RuntimeCapabilityRollbackOutcome::RolledBack => "rolled_back",
        RuntimeCapabilityRollbackOutcome::AlreadyRolledBack => "already_rolled_back",
    }
}

fn render_runtime_capability_planned_payload(
    payload: &RuntimeCapabilityPromotionPlannedPayload,
) -> String {
    let accepted_candidate_ids = render_string_values(&payload.provenance.accepted_candidate_ids);
    let changed_surfaces = render_string_values(&payload.provenance.changed_surfaces);
    let draft_payload = render_runtime_capability_draft_payload(&payload.payload);
    format!(
        "target={} draft_id={} review_scope={} accepted_candidate_ids={} changed_surfaces={} payload={}",
        render_target(payload.target),
        payload.draft_id,
        payload.review_scope,
        accepted_candidate_ids,
        changed_surfaces,
        draft_payload
    )
}

fn render_runtime_capability_draft_payload(payload: &RuntimeCapabilityDraftPayload) -> String {
    match payload {
        RuntimeCapabilityDraftPayload::ManagedSkillBundle { files } => {
            let file_names = files.keys().cloned().collect::<Vec<_>>().join(",");
            format!("managed_skill_bundle files={file_names}")
        }
        RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { files } => {
            let file_names = files.keys().cloned().collect::<Vec<_>>().join(",");
            format!("programmatic_flow_spec files={file_names}")
        }
        RuntimeCapabilityDraftPayload::ProfileNoteAddendum { content } => {
            let content_chars = content.chars().count();
            format!("profile_note_addendum chars={content_chars}")
        }
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
    });
    checklist
}

fn build_runtime_capability_rollback_hints(
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
) -> Vec<String> {
    vec![
        format!(
            "capture the current `{}` state before applying artifact `{}`",
            planned_artifact.delivery_surface, planned_artifact.artifact_id
        ),
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

fn build_runtime_capability_promotion_planned_payload(
    family_id: &str,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
    artifacts: &[RuntimeCapabilityArtifactDocument],
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> CliResult<RuntimeCapabilityPromotionPlannedPayload> {
    let accepted_candidate_ids = artifacts
        .iter()
        .filter(|artifact| artifact.decision == RuntimeCapabilityDecision::Accepted)
        .map(|artifact| artifact.candidate_id.clone())
        .collect::<Vec<_>>();

    let payload = build_runtime_capability_draft_payload(family_id, planned_artifact, evidence)?;

    let planned_payload = RuntimeCapabilityPromotionPlannedPayload {
        artifact_kind: planned_artifact.artifact_kind.clone(),
        target: planned_artifact.target_kind,
        draft_id: planned_artifact.artifact_id.clone(),
        summary: planned_artifact.summary.clone(),
        review_scope: planned_artifact.bounded_scope.clone(),
        required_capabilities: planned_artifact.required_capabilities.clone(),
        tags: planned_artifact.tags.clone(),
        payload,
        provenance: RuntimeCapabilityPromotionPlannedPayloadProvenance {
            family_id: family_id.to_owned(),
            accepted_candidate_ids,
            changed_surfaces: evidence.changed_surfaces.clone(),
        },
    };
    Ok(planned_payload)
}

fn build_runtime_capability_draft_payload(
    family_id: &str,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> CliResult<RuntimeCapabilityDraftPayload> {
    match planned_artifact.target_kind {
        RuntimeCapabilityTarget::ManagedSkill => {
            let files = build_managed_skill_draft_files(family_id, planned_artifact, evidence);
            Ok(RuntimeCapabilityDraftPayload::ManagedSkillBundle { files })
        }
        RuntimeCapabilityTarget::ProgrammaticFlow => {
            let files = build_programmatic_flow_draft_files(family_id, planned_artifact, evidence)?;
            Ok(RuntimeCapabilityDraftPayload::ProgrammaticFlowSpec { files })
        }
        RuntimeCapabilityTarget::ProfileNoteAddendum => {
            let content = build_profile_note_addendum_draft(family_id, planned_artifact, evidence);
            Ok(RuntimeCapabilityDraftPayload::ProfileNoteAddendum { content })
        }
    }
}

fn build_managed_skill_draft_files(
    family_id: &str,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> BTreeMap<String, String> {
    let skill_name = planned_artifact.summary.as_str();
    let skill_description = planned_artifact.summary.as_str();
    let bounded_scope = planned_artifact.bounded_scope.as_str();
    let required_capabilities = render_string_values(&planned_artifact.required_capabilities);
    let tags = render_string_values(&planned_artifact.tags);
    let changed_surfaces = render_string_values(&evidence.changed_surfaces);
    let skill_markdown = format!(
        "---\nname: {skill_name}\ndescription: {skill_description}\n---\n\n# {skill_name}\n\n## Purpose\n\nThis draft managed skill was generated from runtime capability family `{family_id}`.\nReview and refine it before activation.\n\n## Scope\n\n- In: {bounded_scope}\n- Required capabilities: {required_capabilities}\n- Tags: {tags}\n- Changed surfaces: {changed_surfaces}\n"
    );
    let mut files = BTreeMap::new();
    files.insert("SKILL.md".to_owned(), skill_markdown);
    files
}

fn build_programmatic_flow_draft_files(
    family_id: &str,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> CliResult<BTreeMap<String, String>> {
    let draft_id = planned_artifact.artifact_id.as_str();
    let summary = planned_artifact.summary.as_str();
    let bounded_scope = planned_artifact.bounded_scope.as_str();
    let required_capabilities = &planned_artifact.required_capabilities;
    let tags = &planned_artifact.tags;
    let changed_surfaces = &evidence.changed_surfaces;
    let flow_value = json!({
        "id": draft_id,
        "summary": summary,
        "bounded_scope": bounded_scope,
        "required_capabilities": required_capabilities,
        "tags": tags,
        "changed_surfaces": changed_surfaces,
        "provenance": {
            "family_id": family_id,
        },
        "steps": [],
    });
    let flow_json = serde_json::to_string_pretty(&flow_value).map_err(|error| {
        format!("serialize runtime capability programmatic flow draft failed: {error}")
    })?;
    let mut files = BTreeMap::new();
    files.insert("flow.json".to_owned(), flow_json);
    Ok(files)
}

fn build_profile_note_addendum_draft(
    family_id: &str,
    planned_artifact: &RuntimeCapabilityPromotionArtifactPlan,
    evidence: &RuntimeCapabilityEvidenceDigest,
) -> String {
    let summary = planned_artifact.summary.as_str();
    let bounded_scope = planned_artifact.bounded_scope.as_str();
    let required_capabilities = render_string_values(&planned_artifact.required_capabilities);
    let tags = render_string_values(&planned_artifact.tags);
    let changed_surfaces = render_string_values(&evidence.changed_surfaces);
    format!(
        "## Runtime Capability Draft: {summary}\n- Family: {family_id}\n- Scope: {bounded_scope}\n- Required capabilities: {required_capabilities}\n- Tags: {tags}\n- Changed surfaces: {changed_surfaces}\n- Status: review before activation\n"
    )
}

pub fn render_runtime_capability_promotion_plan_text(
    report: &RuntimeCapabilityPromotionPlanReport,
) -> String {
    let body = [
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
        format!(
            "planned_payload={}",
            render_runtime_capability_planned_payload(&report.planned_payload)
        ),
    ]
    .join("\n");
    wrap_runtime_capability_surface("promotion plan", body)
}

fn wrap_runtime_capability_surface(title: &str, body: String) -> String {
    crate::render_operator_shell_surface_from_body(title, "runtime capability", body)
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
