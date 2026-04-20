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

mod artifact;
mod emit;
mod family;
mod index;
mod io;
mod lifecycle;
mod promotion;
mod render;
mod schema;
mod source_run;

use self::artifact::*;
use self::emit::*;
use self::family::*;
use self::index::*;
use self::io::*;
use self::lifecycle::*;
use self::promotion::*;
use self::render::{
    normalized_path_text, render_family_readiness_checks, render_family_readiness_status,
    render_memory_profile, render_target,
};
pub use self::render::{
    render_runtime_capability_activate_text, render_runtime_capability_apply_text,
    render_runtime_capability_index_text, render_runtime_capability_promotion_plan_text,
    render_runtime_capability_rollback_text, render_runtime_capability_text,
};
use self::schema::*;
use self::source_run::*;

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
