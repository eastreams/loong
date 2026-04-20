use super::render::render_string_values;
use super::*;
use serde_json::json;

pub(super) fn build_runtime_capability_promotion_artifact(
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

pub(super) fn build_runtime_capability_approval_checklist(
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

pub(super) fn build_runtime_capability_rollback_hints(
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

pub(super) fn build_runtime_capability_promotion_provenance(
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

pub(super) fn build_runtime_capability_promotion_planned_payload(
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
