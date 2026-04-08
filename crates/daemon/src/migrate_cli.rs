use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
};

use clap::ValueEnum;
use kernel::{ToolCoreOutcome, ToolCoreRequest};
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum MigrateMode {
    Apply,
    Plan,
    Discover,
    PlanMany,
    RecommendPrimary,
    MergeProfiles,
    MapExternalSkills,
    ApplySelected,
    RollbackLastApply,
}

impl MigrateMode {
    fn as_id(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Plan => "plan",
            Self::Discover => "discover",
            Self::PlanMany => "plan_many",
            Self::RecommendPrimary => "recommend_primary",
            Self::MergeProfiles => "merge_profiles",
            Self::MapExternalSkills => "map_external_skills",
            Self::ApplySelected => "apply_selected",
            Self::RollbackLastApply => "rollback_last_apply",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MigrateCommandOptions {
    pub input: Option<String>,
    pub output: Option<String>,
    pub source: Option<String>,
    pub mode: MigrateMode,
    pub json: bool,
    pub source_id: Option<String>,
    pub safe_profile_merge: bool,
    pub primary_source_id: Option<String>,
    pub apply_external_skills_plan: bool,
    pub force: bool,
}

pub fn parse_legacy_claw_source(raw: &str) -> Option<mvp::migration::LegacyClawSource> {
    mvp::migration::LegacyClawSource::from_id(raw)
}
pub fn run_migrate_cli(options: MigrateCommandOptions) -> CliResult<()> {
    block_on_migrate_cli(run_migrate_cli_async(options))
}

async fn run_migrate_cli_async(options: MigrateCommandOptions) -> CliResult<()> {
    validate_migrate_cli_required_flags(&options)?;
    let config = load_migrate_cli_runtime_config(&options)?;
    let kernel_ctx = mvp::context::bootstrap_kernel_context_with_config(
        "daemon-migrate-cli",
        mvp::context::DEFAULT_TOKEN_TTL_S,
        &config,
    )?;
    let outcome = mvp::tools::execute_tool(
        ToolCoreRequest {
            tool_name: "claw.migrate".to_owned(),
            payload: build_migrate_tool_payload(&options),
        },
        &kernel_ctx,
    )
    .await
    .map_err(|error| translate_migrate_cli_error(&options, error))?;

    render_migrate_tool_outcome(&options, outcome)
}

fn validate_migrate_cli_required_flags(options: &MigrateCommandOptions) -> CliResult<()> {
    let mode_id = options.mode.as_id();
    let requires_output = matches!(
        options.mode,
        MigrateMode::Apply | MigrateMode::ApplySelected | MigrateMode::RollbackLastApply
    );
    let requires_input = !matches!(options.mode, MigrateMode::RollbackLastApply);

    let output_is_present = required_cli_flag_is_present(options.output.as_deref());
    let input_is_present = required_cli_flag_is_present(options.input.as_deref());

    if requires_output && !output_is_present {
        let error = format!("`--output` is required for `loongclaw migrate --mode {mode_id}`");
        return Err(error);
    }

    if requires_input && !input_is_present {
        let error = format!("`--input` is required for `loongclaw migrate --mode {mode_id}`");
        return Err(error);
    }

    Ok(())
}

fn required_cli_flag_is_present(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let normalized = raw.trim();
    !normalized.is_empty()
}

fn block_on_migrate_cli<F>(future: F) -> CliResult<()>
where
    F: Future<Output = CliResult<()>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to create migrate CLI runtime: {error}"))?
        .block_on(future)
}

fn load_migrate_cli_runtime_config(
    options: &MigrateCommandOptions,
) -> CliResult<mvp::config::LoongClawConfig> {
    let config_path = mvp::config::default_config_path();
    let mut config = if config_path.exists() {
        let config_path_string = config_path.display().to_string();
        let (_, config) = mvp::config::load(Some(&config_path_string))?;
        config
    } else {
        mvp::config::LoongClawConfig::default()
    };

    if config.tools.file_root.is_none()
        && let Some(file_root) = derive_migrate_cli_file_root(options)
    {
        config.tools.file_root = Some(file_root.display().to_string());
    }

    Ok(config)
}

fn derive_migrate_cli_file_root(options: &MigrateCommandOptions) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(input) = options.input.as_deref() {
        candidates.push(policy_root_candidate(
            normalize_migrate_cli_path(input, true),
            true,
        ));
    }
    if let Some(output) = options.output.as_deref() {
        candidates.push(policy_root_candidate(
            normalize_migrate_cli_path(output, false),
            false,
        ));
    }

    common_ancestor_path(&candidates)
}

fn policy_root_candidate(path: PathBuf, keep_dir: bool) -> PathBuf {
    if keep_dir && path.is_dir() {
        return path;
    }

    path.parent().map(Path::to_path_buf).unwrap_or(path)
}

fn normalize_migrate_cli_path(raw: &str, keep_dir: bool) -> PathBuf {
    normalize_migrate_cli_pathbuf(mvp::config::expand_path(raw), keep_dir)
}

fn normalize_migrate_cli_pathbuf(path: PathBuf, keep_dir: bool) -> PathBuf {
    if path.exists() {
        return fs::canonicalize(&path).unwrap_or(path);
    }

    if !keep_dir
        && let (Some(parent), Some(file_name)) = (path.parent(), path.file_name())
        && parent.exists()
        && let Ok(parent) = fs::canonicalize(parent)
    {
        return parent.join(file_name);
    }

    path
}

fn common_ancestor_path(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut iter = paths.iter();
    let mut current = iter.next()?.clone();

    for path in iter {
        let mut shared = PathBuf::new();
        let mut matched = false;
        for (left, right) in current.components().zip(path.components()) {
            if left != right {
                break;
            }
            shared.push(left.as_os_str());
            matched = true;
        }
        if !matched {
            return None;
        }
        current = shared;
    }

    Some(current)
}

fn build_migrate_tool_payload(options: &MigrateCommandOptions) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("mode".to_owned(), json!(options.mode.as_id()));
    if let Some(input) = options.input.as_deref() {
        let input = normalize_migrate_cli_path(input, true);
        payload.insert("input_path".to_owned(), json!(input.display().to_string()));
    }
    if let Some(output) = options.output.as_deref() {
        let output = normalize_migrate_cli_path(output, false);
        payload.insert(
            "output_path".to_owned(),
            json!(output.display().to_string()),
        );
    }
    if let Some(source) = options.source.as_deref() {
        payload.insert("source".to_owned(), json!(source));
    }
    if let Some(source_id) = options.source_id.as_deref() {
        payload.insert("source_id".to_owned(), json!(source_id));
    }
    if options.safe_profile_merge {
        payload.insert("safe_profile_merge".to_owned(), json!(true));
    }
    if let Some(primary_source_id) = options.primary_source_id.as_deref() {
        payload.insert("primary_source_id".to_owned(), json!(primary_source_id));
    }
    if options.apply_external_skills_plan {
        payload.insert("apply_external_skills_plan".to_owned(), json!(true));
    }
    if options.force {
        payload.insert("force".to_owned(), json!(true));
    }
    Value::Object(payload)
}

fn translate_migrate_cli_error(options: &MigrateCommandOptions, error: String) -> String {
    let leaf = error
        .strip_prefix("tool execution failed: ")
        .unwrap_or(&error);
    if leaf == "claw.migrate requires payload.input_path" {
        return format!(
            "`--input` is required for `loongclaw migrate --mode {}`",
            options.mode.as_id()
        );
    }

    if leaf
        == format!(
            "claw.migrate {} mode requires payload.output_path",
            options.mode.as_id()
        )
    {
        return format!(
            "`--output` is required for `loongclaw migrate --mode {}`",
            options.mode.as_id()
        );
    }

    format!("migrate tool execution failed: {error}")
}

fn render_migrate_tool_outcome(
    options: &MigrateCommandOptions,
    outcome: ToolCoreOutcome,
) -> CliResult<()> {
    let mut payload = outcome.payload;
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| "migrate tool payload missing `mode`".to_owned())?
        .to_owned();

    let sqlite_memory_path = ensure_memory_ready_for_tool_payload(mode.as_str(), &payload)?;
    if let Some(memory_path) = sqlite_memory_path.as_ref()
        && let Some(object) = payload.as_object_mut()
    {
        let object: &mut serde_json::Map<String, Value> = object;
        object.insert(
            "sqlite_memory_path".to_owned(),
            json!(memory_path.display().to_string()),
        );
    }

    if options.json {
        return print_json_payload(payload);
    }

    match mode.as_str() {
        "discover" => render_discover_outcome(&payload),
        "plan_many" | "recommend_primary" => render_plan_many_outcome(&payload),
        "merge_profiles" => render_merge_profiles_outcome(&payload),
        "map_external_skills" => render_external_skill_mapping_outcome(&payload, options),
        "apply_selected" => render_apply_selected_outcome(&payload, options, sqlite_memory_path),
        "apply" | "plan" => {
            render_apply_or_plan_outcome(mode.as_str(), &payload, sqlite_memory_path, options)
        }
        "rollback_last_apply" => render_rollback_outcome(&payload),
        other => Err(format!("unsupported migrate tool mode `{other}`")),
    }
}

#[cfg(feature = "memory-sqlite")]
fn ensure_memory_ready_for_tool_payload(mode: &str, payload: &Value) -> CliResult<Option<PathBuf>> {
    let output_path = match mode {
        "apply" => payload.get("output_path").and_then(Value::as_str),
        "apply_selected" => payload
            .get("result")
            .and_then(|value| value.get("output_path"))
            .and_then(Value::as_str),
        _ => None,
    };

    match output_path {
        Some(path) => ensure_memory_ready_from_path(Path::new(path)).map(Some),
        None => Ok(None),
    }
}

#[cfg(not(feature = "memory-sqlite"))]
fn ensure_memory_ready_for_tool_payload(
    _mode: &str,
    _payload: &Value,
) -> CliResult<Option<PathBuf>> {
    Ok(None)
}

fn render_discover_outcome(payload: &Value) -> CliResult<()> {
    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "discover payload missing `input_path`".to_owned())?;
    let sources = payload
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| "discover payload missing `sources`".to_owned())?;

    println!("migration discovery complete");
    println!("- input: {input_path}");
    println!("- discovered sources: {}", sources.len());
    for source in sources {
        println!(
            "- [{}] kind={} confidence={} path={}",
            source
                .get("source_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            source
                .get("source_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            source
                .get("confidence_score")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            source
                .get("input_path")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        );
    }
    Ok(())
}

fn render_plan_many_outcome(payload: &Value) -> CliResult<()> {
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| "planning payload missing `mode`".to_owned())?;
    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "planning payload missing `input_path`".to_owned())?;
    let plans = payload
        .get("plans")
        .and_then(Value::as_array)
        .ok_or_else(|| "planning payload missing `plans`".to_owned())?;

    println!("migration planning complete");
    println!("- mode: {mode}");
    println!("- input: {input_path}");
    println!("- planned sources: {}", plans.len());
    if let Some(recommendation) = payload.get("recommendation")
        && let Some(source_id) = recommendation.get("source_id").and_then(Value::as_str)
    {
        let source_kind = recommendation
            .get("source_kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        println!("- recommended source: {source_id} ({source_kind})");
    }
    for plan in plans {
        println!(
            "- [{}] kind={} confidence={} prompt={} profile={} warnings={} path={}",
            plan.get("source_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            plan.get("source_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            plan.get("confidence_score")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            yes_no(
                plan.get("prompt_addendum_present")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ),
            yes_no(
                plan.get("profile_note_present")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ),
            plan.get("warning_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            plan.get("input_path")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        );
    }
    Ok(())
}

fn render_merge_profiles_outcome(payload: &Value) -> CliResult<()> {
    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "merge_profiles payload missing `input_path`".to_owned())?;
    let plans = payload
        .get("plans")
        .and_then(Value::as_array)
        .ok_or_else(|| "merge_profiles payload missing `plans`".to_owned())?;
    let result = payload
        .get("result")
        .ok_or_else(|| "merge_profiles payload missing `result`".to_owned())?;

    println!("profile merge preview complete");
    println!("- input: {input_path}");
    println!("- source count: {}", plans.len());
    if let Some(recommendation) = payload.get("recommendation")
        && let Some(source_id) = recommendation.get("source_id").and_then(Value::as_str)
    {
        println!("- recommended prompt owner: {source_id}");
    }
    println!(
        "- auto apply allowed: {}",
        yes_no(
            result
                .get("auto_apply_allowed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        )
    );
    println!(
        "- unresolved conflicts: {}",
        result
            .get("unresolved_conflicts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    println!(
        "- kept entries: {}",
        result
            .get("kept_entries")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    println!(
        "- dropped duplicates: {}",
        result
            .get("dropped_duplicates")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    Ok(())
}

fn render_external_skill_mapping_outcome(
    payload: &Value,
    options: &MigrateCommandOptions,
) -> CliResult<()> {
    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "map_external_skills payload missing `input_path`".to_owned())?;
    let result = payload
        .get("result")
        .ok_or_else(|| "map_external_skills payload missing `result`".to_owned())?;

    println!("external skills mapping plan ready");
    println!("- input: {input_path}");
    println!(
        "- detected artifacts: {}",
        result
            .get("artifact_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    println!(
        "- declared skills: {}",
        result
            .get("declared_skills")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    println!(
        "- locked skills: {}",
        result
            .get("locked_skills")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    println!(
        "- resolved skills: {}",
        result
            .get("resolved_skills")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    println!(
        "- profile addendum generated: {}",
        yes_no(result.get("profile_note_addendum").is_some())
    );
    if let Some(artifacts) = result.get("artifacts").and_then(Value::as_array) {
        for artifact in artifacts {
            println!(
                "- artifact: kind={} path={}",
                artifact
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                artifact
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            );
        }
    }
    if let Some(warnings) = result.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            println!("- warning: {warning}");
        }
    }
    if let Some(output) = options.output.as_deref() {
        println!(
            "next step: loongclaw migrate --mode apply_selected --input {} --output {} --apply-external-skills-plan --force",
            input_path, output
        );
    }
    Ok(())
}

fn render_apply_selected_outcome(
    payload: &Value,
    options: &MigrateCommandOptions,
    sqlite_memory_path: Option<PathBuf>,
) -> CliResult<()> {
    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "apply_selected payload missing `input_path`".to_owned())?;
    let output_path = payload
        .get("output_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "apply_selected payload missing `output_path`".to_owned())?;
    let result = payload
        .get("result")
        .ok_or_else(|| "apply_selected payload missing `result`".to_owned())?;

    println!("migration selection applied");
    println!("- mode: apply_selected");
    println!("- input: {input_path}");
    println!("- output: {output_path}");
    let selection_mode = if options.safe_profile_merge {
        "safe_profile_merge"
    } else if options.source_id.is_some() {
        "selected_single_source"
    } else {
        "recommended_single_source"
    };
    println!("- selection mode: {selection_mode}");
    if let Some(source_id) = result
        .get("selected_primary_source_id")
        .and_then(Value::as_str)
    {
        println!("- selected primary source: {source_id}");
    }
    if let Some(merged_ids) = result.get("merged_source_ids").and_then(Value::as_array) {
        let merged = merged_ids
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        println!("- merged source ids: {merged}");
    }
    println!(
        "- unresolved conflicts: {}",
        result
            .get("unresolved_conflicts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    );
    println!(
        "- external skill artifacts: {}",
        result
            .get("external_skill_artifact_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    println!(
        "- external skill entries applied: {}",
        result
            .get("external_skill_entries_applied")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    println!(
        "- managed external skills bridged: {}",
        result
            .get("external_skill_managed_install_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    );
    if let Some(bridged_skill_ids) = result
        .get("external_skill_managed_skill_ids")
        .and_then(Value::as_array)
    {
        let bridged = bridged_skill_ids
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !bridged.is_empty() {
            println!("- bridged skill ids: {}", bridged.join(", "));
        }
    }
    if let Some(manifest_path) = result
        .get("external_skills_manifest_path")
        .and_then(Value::as_str)
    {
        println!("- external skills manifest: {manifest_path}");
    }
    if let Some(memory_path) = sqlite_memory_path.as_ref() {
        println!("- sqlite memory: {}", memory_path.display());
    }
    if let Some(warnings) = result.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            println!("- warning: {warning}");
        }
    }
    println!(
        "next step: loongclaw ask --config '{}' --message 'Summarize this repository and suggest the best next step.'",
        output_path
    );
    Ok(())
}

fn render_apply_or_plan_outcome(
    mode: &str,
    payload: &Value,
    sqlite_memory_path: Option<PathBuf>,
    options: &MigrateCommandOptions,
) -> CliResult<()> {
    let source = payload
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{mode} payload missing `source`"))?;
    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{mode} payload missing `input_path`"))?;
    let output_path = payload.get("output_path").and_then(Value::as_str);
    let config_preview = payload
        .get("config_preview")
        .ok_or_else(|| format!("{mode} payload missing `config_preview`"))?;
    let prompt_pack_id = config_preview
        .get("prompt_pack_id")
        .and_then(Value::as_str)
        .unwrap_or(mvp::prompt::DEFAULT_PROMPT_PACK_ID);
    let memory_profile = config_preview
        .get("memory_profile")
        .and_then(Value::as_str)
        .unwrap_or("profile_plus_window");
    let prompt_addendum_present = config_preview
        .get("system_prompt_addendum")
        .and_then(Value::as_str)
        .is_some();
    let profile_note_present = config_preview
        .get("profile_note")
        .and_then(Value::as_str)
        .is_some();

    if mode == "plan" {
        println!("migration plan ready");
        println!("- source: {source}");
        println!("- input: {input_path}");
        if let Some(output_path) = output_path.or(options.output.as_deref()) {
            println!("- output target: {output_path}");
        }
    } else {
        println!("migration complete");
        println!("- source: {source}");
        println!("- input: {input_path}");
        if let Some(output_path) = output_path {
            println!("- config: {output_path}");
        }
    }
    println!("- prompt pack: {prompt_pack_id}");
    println!("- memory profile: {memory_profile}");
    println!(
        "- migrated prompt addendum: {}",
        yes_no(prompt_addendum_present)
    );
    println!("- migrated profile note: {}", yes_no(profile_note_present));
    if let Some(memory_path) = sqlite_memory_path.as_ref() {
        println!("- sqlite memory: {}", memory_path.display());
    }
    if let Some(warnings) = payload.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            println!("- warning: {warning}");
        }
    }
    if let Some(next_step) = payload.get("next_step").and_then(Value::as_str) {
        println!("next step: {next_step}");
    } else if mode == "plan"
        && let Some(output_path) = output_path.or(options.output.as_deref())
    {
        println!(
            "next step: loongclaw migrate --mode apply --input {} --output {} --force",
            input_path, output_path
        );
    }
    Ok(())
}

fn render_rollback_outcome(payload: &Value) -> CliResult<()> {
    let output_path = payload
        .get("output_path")
        .and_then(Value::as_str)
        .ok_or_else(|| "rollback payload missing `output_path`".to_owned())?;
    println!("migration rollback complete");
    println!("- output: {output_path}");
    println!("- rolled_back: yes");
    Ok(())
}

fn print_json_payload(payload: Value) -> CliResult<()> {
    let encoded = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("json serialization failed: {error}"))?;
    println!("{encoded}");
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn ensure_memory_ready_from_path(path: &Path) -> CliResult<PathBuf> {
    let output = path.display().to_string();
    let (_, config) = mvp::config::load(Some(&output))?;
    let runtime =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    mvp::memory::ensure_memory_db_ready(Some(config.memory.resolved_sqlite_path()), &runtime)
        .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
