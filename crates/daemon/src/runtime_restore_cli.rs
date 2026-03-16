use crate::{
    RuntimeSnapshotArtifactDocument, RuntimeSnapshotRestoreManagedSkillSpec,
    collect_runtime_snapshot_cli_state,
};
use clap::Parser;
use kernel::ToolCoreRequest;
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRestoreCommandOptions {
    pub config: Option<String>,
    pub snapshot: String,
    pub json: bool,
    pub apply: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeRestoreExecution {
    pub resolved_config_path: String,
    pub snapshot_path: String,
    pub lineage: RuntimeRestoreLineageSummary,
    pub plan: RuntimeRestorePlan,
    pub applied: bool,
    pub verification: Option<RuntimeRestoreVerification>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeRestoreLineageSummary {
    pub snapshot_id: String,
    pub created_at: String,
    pub label: Option<String>,
    pub experiment_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeRestorePlan {
    pub can_apply: bool,
    pub changed_surfaces: Vec<String>,
    pub warnings: Vec<String>,
    pub managed_skill_actions: Vec<RuntimeRestoreManagedSkillAction>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeRestoreManagedSkillAction {
    pub action: String,
    pub skill_id: String,
    pub source_kind: String,
    pub source_path: String,
    pub current_sha256: Option<String>,
    pub target_sha256: Option<String>,
    use_target_config: bool,
    apply_install_root: Option<String>,
    current_source_kind: Option<String>,
    current_source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeRestoreVerification {
    pub restored_exactly: bool,
    pub verified_surfaces: Vec<String>,
    pub mismatches: Vec<String>,
    pub capability_snapshot_sha256: String,
}

#[derive(Debug, Clone)]
struct RuntimeRestoreArtifactInput {
    document: RuntimeSnapshotArtifactDocument,
}

#[derive(Debug, Clone)]
struct ManagedSkillInventorySnapshot {
    install_root: Option<String>,
    skills: BTreeMap<String, ManagedSkillInventoryEntry>,
}

#[derive(Debug, Clone)]
struct ManagedSkillInventoryEntry {
    source_kind: String,
    source_path: String,
    sha256: String,
}

pub fn run_runtime_restore_cli(options: RuntimeRestoreCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_runtime_restore_command(options)?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&execution)
            .map_err(|error| format!("serialize runtime restore output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_runtime_restore_text(&execution));
    Ok(())
}

pub fn execute_runtime_restore_command(
    options: RuntimeRestoreCommandOptions,
) -> CliResult<RuntimeRestoreExecution> {
    let (resolved_path, current_config) = mvp::config::load(options.config.as_deref())?;
    let artifact = load_runtime_restore_artifact(Path::new(&options.snapshot))?;
    let target_config = build_restored_config(&current_config, &artifact.document)?;
    let plan =
        build_runtime_restore_plan(&resolved_path, &current_config, &target_config, &artifact)?;

    if options.apply && !plan.can_apply {
        return Err("runtime restore plan cannot be safely applied".to_owned());
    }

    let verification = if options.apply {
        Some(apply_runtime_restore(
            &resolved_path,
            &current_config,
            &target_config,
            &plan,
            &artifact,
        )?)
    } else {
        None
    };
    let lineage = artifact.document.lineage;

    Ok(RuntimeRestoreExecution {
        resolved_config_path: resolved_path.display().to_string(),
        snapshot_path: options.snapshot,
        lineage: RuntimeRestoreLineageSummary {
            snapshot_id: lineage.snapshot_id,
            created_at: lineage.created_at,
            label: lineage.label,
            experiment_id: lineage.experiment_id,
            parent_snapshot_id: lineage.parent_snapshot_id,
        },
        plan,
        applied: options.apply,
        verification,
    })
}

fn load_runtime_restore_artifact(path: &Path) -> CliResult<RuntimeRestoreArtifactInput> {
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read runtime snapshot artifact {} failed: {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_str::<Value>(&raw).map_err(|error| {
        format!(
            "parse runtime snapshot artifact {} failed: {error}",
            path.display()
        )
    })?;
    let document =
        serde_json::from_value::<RuntimeSnapshotArtifactDocument>(value).map_err(|error| {
            format!(
                "decode runtime snapshot artifact {} failed: {error}",
                path.display()
            )
        })?;
    if document.schema.version != crate::RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION {
        return Err(format!(
            "runtime snapshot artifact {} uses unsupported schema version {}; expected {}",
            path.display(),
            document.schema.version,
            crate::RUNTIME_SNAPSHOT_ARTIFACT_JSON_SCHEMA_VERSION
        ));
    }
    Ok(RuntimeRestoreArtifactInput { document })
}

fn build_restored_config(
    current_config: &mvp::config::LoongClawConfig,
    artifact: &RuntimeSnapshotArtifactDocument,
) -> CliResult<mvp::config::LoongClawConfig> {
    let mut restored = current_config.clone();
    restored.conversation = artifact.restore_spec.conversation.clone();
    restored.memory = artifact.restore_spec.memory.clone();
    restored.acp = artifact.restore_spec.acp.clone();
    restored.tools = artifact.restore_spec.tools.clone();
    restored.external_skills = artifact.restore_spec.external_skills.clone();
    restored.providers = artifact.restore_spec.provider.profiles.clone();
    restored.active_provider = artifact.restore_spec.provider.active_provider.clone();
    restored.last_provider = artifact.restore_spec.provider.last_provider.clone();

    let active_profile_id = restored
        .active_provider
        .clone()
        .or_else(|| restored.providers.keys().next().cloned())
        .ok_or_else(|| "runtime restore artifact is missing a provider profile".to_owned())?;
    let active_profile = restored
        .providers
        .get(&active_profile_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "runtime restore artifact references unknown active provider `{active_profile_id}`"
            )
        })?;
    restored.provider = active_profile.provider;
    restored.active_provider = Some(active_profile_id);
    Ok(restored)
}

fn build_runtime_restore_plan(
    resolved_path: &Path,
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
    artifact: &RuntimeRestoreArtifactInput,
) -> CliResult<RuntimeRestorePlan> {
    let current_managed_skills = collect_managed_skill_inventory(
        resolved_path,
        current_config,
        current_config.external_skills.resolved_install_root(),
    )?;
    let target_install_root = target_config
        .external_skills
        .resolved_install_root()
        .map(|path| path.display().to_string());
    let managed_skill_actions = plan_managed_skill_actions(
        &current_managed_skills,
        &artifact.document.restore_spec.managed_skills.skills,
        target_install_root.clone(),
    );

    let mut changed_surfaces = Vec::new();
    if provider_runtime_changed(current_config, target_config) {
        changed_surfaces.push("provider".to_owned());
    }
    if current_config.conversation != target_config.conversation {
        changed_surfaces.push("conversation".to_owned());
    }
    if current_config.memory != target_config.memory {
        changed_surfaces.push("memory".to_owned());
    }
    if current_config.acp != target_config.acp {
        changed_surfaces.push("acp".to_owned());
    }
    if current_config.tools != target_config.tools {
        changed_surfaces.push("tools".to_owned());
    }
    if current_config.external_skills != target_config.external_skills {
        changed_surfaces.push("external_skills".to_owned());
    }
    if !managed_skill_actions.is_empty() {
        changed_surfaces.push("managed_skills".to_owned());
    }

    let mut warnings = artifact.document.restore_spec.warnings.clone();
    if current_managed_skills.install_root != target_install_root
        && !managed_skill_actions.is_empty()
    {
        warnings.push(format!(
            "runtime restore will switch managed external skill install root from {} to {}",
            current_managed_skills
                .install_root
                .as_deref()
                .unwrap_or("-"),
            target_install_root.as_deref().unwrap_or("-")
        ));
    }
    let can_apply = !runtime_restore_has_blocking_warnings(&warnings)
        && managed_skill_actions
            .iter()
            .all(|action| validate_managed_skill_action(action, &mut warnings));

    Ok(RuntimeRestorePlan {
        can_apply,
        changed_surfaces,
        warnings,
        managed_skill_actions,
    })
}

fn provider_runtime_changed(
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
) -> bool {
    current_config.provider != target_config.provider
        || current_config.providers != target_config.providers
        || current_config.active_provider != target_config.active_provider
        || current_config.last_provider != target_config.last_provider
}

fn collect_managed_skill_inventory(
    resolved_path: &Path,
    base_config: &mvp::config::LoongClawConfig,
    install_root: Option<PathBuf>,
) -> CliResult<ManagedSkillInventorySnapshot> {
    let mut inventory_config = base_config.clone();
    inventory_config.external_skills.enabled = true;
    if let Some(install_root) = install_root {
        inventory_config.external_skills.install_root = Some(install_root.display().to_string());
    }

    let resolved_install_root = inventory_config
        .external_skills
        .resolved_install_root()
        .map(|path| path.display().to_string());
    let tool_runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        &inventory_config,
        Some(resolved_path),
    );
    let outcome = mvp::tools::execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "external_skills.list".to_owned(),
            payload: json!({}),
        },
        &tool_runtime,
    )
    .map_err(|error| format!("list current managed external skills failed: {error}"))?;
    let skills = outcome
        .payload
        .get("skills")
        .and_then(Value::as_array)
        .ok_or_else(|| "managed external skill inventory payload missing `skills`".to_owned())?;

    Ok(ManagedSkillInventorySnapshot {
        install_root: resolved_install_root,
        skills: skills
            .iter()
            .filter(|skill| skill.get("scope").and_then(Value::as_str) == Some("managed"))
            .filter_map(|skill| {
                Some((
                    skill.get("skill_id").and_then(Value::as_str)?.to_owned(),
                    ManagedSkillInventoryEntry {
                        source_kind: skill.get("source_kind").and_then(Value::as_str)?.to_owned(),
                        source_path: skill.get("source_path").and_then(Value::as_str)?.to_owned(),
                        sha256: skill.get("sha256").and_then(Value::as_str)?.to_owned(),
                    },
                ))
            })
            .collect(),
    })
}

fn plan_managed_skill_actions(
    current: &ManagedSkillInventorySnapshot,
    target: &[RuntimeSnapshotRestoreManagedSkillSpec],
    target_install_root: Option<String>,
) -> Vec<RuntimeRestoreManagedSkillAction> {
    let target = target
        .iter()
        .map(|skill| (skill.skill_id.clone(), skill))
        .collect::<BTreeMap<_, _>>();

    let mut actions = Vec::new();
    let current_install_root = current.install_root.clone();
    let install_root_changed = current_install_root != target_install_root;
    for skill_id in current
        .skills
        .keys()
        .chain(target.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
    {
        match (current.skills.get(&skill_id), target.get(&skill_id)) {
            (None, Some(target_skill)) => actions.push(build_install_action(
                &skill_id,
                target_skill,
                target_install_root.clone(),
            )),
            (Some(current_skill), None) if !install_root_changed => {
                actions.push(build_remove_action(
                    &skill_id,
                    current_skill,
                    current_install_root.clone(),
                ));
            }
            (Some(current_skill), Some(target_skill))
                if current_skill.sha256 != target_skill.sha256 =>
            {
                actions.push(build_replace_action(
                    &skill_id,
                    current_skill,
                    target_skill,
                    target_install_root
                        .clone()
                        .or_else(|| current_install_root.clone()),
                ));
            }
            (Some(_current_skill), Some(target_skill)) if install_root_changed => actions.push(
                build_install_action(&skill_id, target_skill, target_install_root.clone()),
            ),
            _ => {}
        }
    }
    actions.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    actions
}

fn build_install_action(
    skill_id: &str,
    target_skill: &RuntimeSnapshotRestoreManagedSkillSpec,
    target_install_root: Option<String>,
) -> RuntimeRestoreManagedSkillAction {
    RuntimeRestoreManagedSkillAction {
        action: "install".to_owned(),
        skill_id: skill_id.to_owned(),
        source_kind: target_skill.source_kind.clone(),
        source_path: target_skill.source_path.clone(),
        current_sha256: None,
        target_sha256: Some(target_skill.sha256.clone()),
        use_target_config: true,
        apply_install_root: target_install_root,
        current_source_kind: None,
        current_source_path: None,
    }
}

fn build_remove_action(
    skill_id: &str,
    current_skill: &ManagedSkillInventoryEntry,
    current_install_root: Option<String>,
) -> RuntimeRestoreManagedSkillAction {
    RuntimeRestoreManagedSkillAction {
        action: "remove".to_owned(),
        skill_id: skill_id.to_owned(),
        source_kind: current_skill.source_kind.clone(),
        source_path: current_skill.source_path.clone(),
        current_sha256: Some(current_skill.sha256.clone()),
        target_sha256: None,
        use_target_config: false,
        apply_install_root: current_install_root,
        current_source_kind: Some(current_skill.source_kind.clone()),
        current_source_path: Some(current_skill.source_path.clone()),
    }
}

fn build_replace_action(
    skill_id: &str,
    current_skill: &ManagedSkillInventoryEntry,
    target_skill: &RuntimeSnapshotRestoreManagedSkillSpec,
    apply_install_root: Option<String>,
) -> RuntimeRestoreManagedSkillAction {
    RuntimeRestoreManagedSkillAction {
        action: "replace".to_owned(),
        skill_id: skill_id.to_owned(),
        source_kind: target_skill.source_kind.clone(),
        source_path: target_skill.source_path.clone(),
        current_sha256: Some(current_skill.sha256.clone()),
        target_sha256: Some(target_skill.sha256.clone()),
        use_target_config: true,
        apply_install_root,
        current_source_kind: Some(current_skill.source_kind.clone()),
        current_source_path: Some(current_skill.source_path.clone()),
    }
}

fn runtime_restore_has_blocking_warnings(warnings: &[String]) -> bool {
    warnings.iter().any(|warning| {
        warning.contains("redacted inline provider credential")
            || warning.contains("redacted inline provider header")
    })
}

fn validate_managed_skill_action(
    action: &RuntimeRestoreManagedSkillAction,
    warnings: &mut Vec<String>,
) -> bool {
    if action.action == "remove" {
        return true;
    }
    if action.source_kind == "bundled" {
        let is_valid = action.source_path.starts_with("bundled://");
        if !is_valid {
            warnings.push(format!(
                "restore action for bundled skill `{}` is missing a bundled source identifier",
                action.skill_id
            ));
        }
        return is_valid;
    }
    if Path::new(&action.source_path).exists() {
        return true;
    }
    warnings.push(format!(
        "restore action for managed skill `{}` cannot find source path {}",
        action.skill_id, action.source_path
    ));
    false
}

fn apply_runtime_restore(
    resolved_path: &Path,
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
    plan: &RuntimeRestorePlan,
    artifact: &RuntimeRestoreArtifactInput,
) -> CliResult<RuntimeRestoreVerification> {
    let path_string = resolved_path.to_string_lossy();
    let rollback_actions = apply_managed_skill_actions(
        resolved_path,
        current_config,
        target_config,
        &plan.managed_skill_actions,
    )?;
    if let Err(error) = mvp::config::write(Some(path_string.as_ref()), target_config, true) {
        if let Err(rollback_error) = rollback_managed_skill_actions(
            resolved_path,
            current_config,
            target_config,
            &rollback_actions,
        ) {
            return Err(format!(
                "persist runtime restore config {} failed: {error}; managed skill rollback also failed: {rollback_error}",
                resolved_path.display()
            ));
        }
        return Err(format!(
            "persist runtime restore config {} failed after reverting managed skill changes: {error}",
            resolved_path.display()
        ));
    }

    let post_snapshot = collect_runtime_snapshot_cli_state(Some(path_string.as_ref()))?;
    Ok(verify_runtime_restore(plan, artifact, &post_snapshot))
}

fn apply_managed_skill_actions(
    resolved_path: &Path,
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
    actions: &[RuntimeRestoreManagedSkillAction],
) -> CliResult<Vec<RuntimeRestoreManagedSkillAction>> {
    let mut rollback_actions = Vec::new();
    for action in actions {
        if let Err(error) =
            apply_single_managed_skill_action(resolved_path, current_config, target_config, action)
        {
            if let Err(rollback_error) = rollback_managed_skill_actions(
                resolved_path,
                current_config,
                target_config,
                &rollback_actions,
            ) {
                return Err(format!(
                    "{error}; managed skill rollback also failed: {rollback_error}"
                ));
            }
            return Err(error);
        }
        if let Some(rollback_action) = rollback_action_for_success(action) {
            rollback_actions.push(rollback_action);
        }
    }
    Ok(rollback_actions)
}

fn rollback_managed_skill_actions(
    resolved_path: &Path,
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
    rollback_actions: &[RuntimeRestoreManagedSkillAction],
) -> CliResult<()> {
    let mut rollback_errors = Vec::new();
    for action in rollback_actions.iter().rev() {
        if let Err(error) =
            apply_single_managed_skill_action(resolved_path, current_config, target_config, action)
        {
            rollback_errors.push(error);
        }
    }
    if rollback_errors.is_empty() {
        Ok(())
    } else {
        Err(rollback_errors.join("; "))
    }
}

fn rollback_action_for_success(
    action: &RuntimeRestoreManagedSkillAction,
) -> Option<RuntimeRestoreManagedSkillAction> {
    match action.action.as_str() {
        "install" => Some(RuntimeRestoreManagedSkillAction {
            action: "remove".to_owned(),
            skill_id: action.skill_id.clone(),
            source_kind: action.source_kind.clone(),
            source_path: action.source_path.clone(),
            current_sha256: action.target_sha256.clone(),
            target_sha256: None,
            use_target_config: true,
            apply_install_root: action.apply_install_root.clone(),
            current_source_kind: None,
            current_source_path: None,
        }),
        "remove" | "replace" => Some(RuntimeRestoreManagedSkillAction {
            action: if action.action == "replace" {
                "replace".to_owned()
            } else {
                "install".to_owned()
            },
            skill_id: action.skill_id.clone(),
            source_kind: action.current_source_kind.clone()?,
            source_path: action.current_source_path.clone()?,
            current_sha256: None,
            target_sha256: action.current_sha256.clone(),
            use_target_config: false,
            apply_install_root: action.apply_install_root.clone(),
            current_source_kind: None,
            current_source_path: None,
        }),
        _ => None,
    }
}

fn apply_single_managed_skill_action(
    resolved_path: &Path,
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
    action: &RuntimeRestoreManagedSkillAction,
) -> CliResult<()> {
    let tool_runtime =
        build_action_tool_runtime(resolved_path, current_config, target_config, action);
    match action.action.as_str() {
        "install" | "replace" => {
            let payload = if action.source_kind == "bundled" {
                json!({
                    "bundled_skill_id": bundled_skill_id_for_action(action)?,
                    "replace": action.action == "replace",
                })
            } else {
                json!({
                    "path": action.source_path,
                    "replace": action.action == "replace",
                })
            };
            mvp::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload,
                },
                &tool_runtime,
            )
            .map_err(|error| {
                format!(
                    "{} managed external skill `{}` failed: {error}",
                    action.action, action.skill_id
                )
            })?;
            Ok(())
        }
        "remove" => {
            mvp::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.remove".to_owned(),
                    payload: json!({
                        "skill_id": action.skill_id,
                    }),
                },
                &tool_runtime,
            )
            .map_err(|error| {
                format!(
                    "remove managed external skill `{}` failed: {error}",
                    action.skill_id
                )
            })?;
            Ok(())
        }
        other => Err(format!("unknown managed skill restore action `{other}`")),
    }
}

fn build_action_tool_runtime(
    resolved_path: &Path,
    current_config: &mvp::config::LoongClawConfig,
    target_config: &mvp::config::LoongClawConfig,
    action: &RuntimeRestoreManagedSkillAction,
) -> mvp::tools::runtime_config::ToolRuntimeConfig {
    let mut config = if action.use_target_config {
        target_config.clone()
    } else {
        current_config.clone()
    };
    config.external_skills.enabled = true;
    if let Some(install_root) = action.apply_install_root.as_ref() {
        config.external_skills.install_root = Some(install_root.clone());
    }
    mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        &config,
        Some(resolved_path),
    )
}

fn bundled_skill_id_for_action(action: &RuntimeRestoreManagedSkillAction) -> CliResult<String> {
    action
        .source_path
        .strip_prefix("bundled://")
        .map(str::to_owned)
        .or_else(|| (!action.skill_id.trim().is_empty()).then_some(action.skill_id.clone()))
        .ok_or_else(|| {
            format!(
                "restore action for bundled skill `{}` is missing a bundled source identifier",
                action.skill_id
            )
        })
}

fn verify_runtime_restore(
    plan: &RuntimeRestorePlan,
    artifact: &RuntimeRestoreArtifactInput,
    post_snapshot: &crate::RuntimeSnapshotCliState,
) -> RuntimeRestoreVerification {
    let mut mismatches = Vec::new();
    if post_snapshot.restore_spec.provider != artifact.document.restore_spec.provider {
        mismatches.push("provider".to_owned());
    }
    if post_snapshot.restore_spec.conversation != artifact.document.restore_spec.conversation {
        mismatches.push("conversation".to_owned());
    }
    if post_snapshot.restore_spec.memory != artifact.document.restore_spec.memory {
        mismatches.push("memory".to_owned());
    }
    if post_snapshot.restore_spec.acp != artifact.document.restore_spec.acp {
        mismatches.push("acp".to_owned());
    }
    if post_snapshot.restore_spec.tools != artifact.document.restore_spec.tools {
        mismatches.push("tools".to_owned());
    }
    if post_snapshot.restore_spec.external_skills != artifact.document.restore_spec.external_skills
    {
        mismatches.push("external_skills".to_owned());
    }
    if post_snapshot.restore_spec.managed_skills != artifact.document.restore_spec.managed_skills {
        mismatches.push("managed_skills".to_owned());
    }
    if expected_capability_snapshot_sha256(&artifact.document)
        .is_some_and(|digest| digest != post_snapshot.capability_snapshot_sha256)
    {
        mismatches.push("capability_digest".to_owned());
    }

    let verified_surfaces = plan
        .changed_surfaces
        .iter()
        .filter(|surface| !mismatches.iter().any(|mismatch| mismatch == *surface))
        .cloned()
        .collect::<Vec<_>>();

    RuntimeRestoreVerification {
        restored_exactly: mismatches.is_empty(),
        verified_surfaces,
        mismatches,
        capability_snapshot_sha256: post_snapshot.capability_snapshot_sha256.clone(),
    }
}

fn expected_capability_snapshot_sha256(artifact: &RuntimeSnapshotArtifactDocument) -> Option<&str> {
    artifact
        .tools
        .get("capability_snapshot_sha256")
        .and_then(Value::as_str)
}

fn render_runtime_restore_text(execution: &RuntimeRestoreExecution) -> String {
    let mut lines = vec![
        format!("config={}", execution.resolved_config_path),
        format!("snapshot={}", execution.snapshot_path),
        format!("snapshot_id={}", execution.lineage.snapshot_id),
        format!("created_at={}", execution.lineage.created_at),
        format!("apply_requested={}", execution.applied),
        format!("can_apply={}", execution.plan.can_apply),
        format!(
            "changed_surfaces={}",
            render_string_list(execution.plan.changed_surfaces.iter().map(String::as_str))
        ),
    ];

    if !execution.plan.warnings.is_empty() {
        lines.push("warnings:".to_owned());
        for warning in &execution.plan.warnings {
            lines.push(format!("- {warning}"));
        }
    }

    if !execution.plan.managed_skill_actions.is_empty() {
        lines.push("managed_skill_actions:".to_owned());
        for action in &execution.plan.managed_skill_actions {
            lines.push(format!(
                "- {} {} source_kind={} source_path={}",
                action.action, action.skill_id, action.source_kind, action.source_path
            ));
        }
    }

    if let Some(verification) = &execution.verification {
        lines.push(format!(
            "restored_exactly={}",
            verification.restored_exactly
        ));
        if !verification.mismatches.is_empty() {
            lines.push(format!(
                "mismatches={}",
                render_string_list(verification.mismatches.iter().map(String::as_str))
            ));
        }
    }

    lines.join("\n")
}

fn render_string_list<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let rendered = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        "-".to_owned()
    } else {
        rendered.join(",")
    }
}
