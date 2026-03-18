use crate::Capability;
use crate::runtime_experiment_cli::{
    RuntimeExperimentArtifactDocument, RuntimeExperimentDecision,
    RuntimeExperimentShowCommandOptions, RuntimeExperimentStatus,
    execute_runtime_experiment_show_command,
};
use crate::sha2::{self, Digest};
use clap::{Args, Subcommand, ValueEnum};
use loongclaw_spec::CliResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const RUNTIME_CAPABILITY_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCapabilityCommands {
    /// Create one capability-candidate artifact from one finished experiment run
    Propose(RuntimeCapabilityProposeCommandOptions),
    /// Record one explicit operator review decision for a capability candidate
    Review(RuntimeCapabilityReviewCommandOptions),
    /// Load and render one persisted capability-candidate artifact
    Show(RuntimeCapabilityShowCommandOptions),
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
            surface: "runtime_capability".to_owned(),
            purpose: "promotion_candidate_record".to_owned(),
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
    Ok(artifact)
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
    fs::canonicalize(path)
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

fn render_target(target: RuntimeCapabilityTarget) -> &'static str {
    match target {
        RuntimeCapabilityTarget::ManagedSkill => "managed_skill",
        RuntimeCapabilityTarget::ProgrammaticFlow => "programmatic_flow",
        RuntimeCapabilityTarget::ProfileNoteAddendum => "profile_note_addendum",
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
