use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

use crate::{
    config::{self, LoongClawConfig, MemoryProfile},
    migration::{self, LegacyClawSource},
};

const DEFAULT_MODE: &str = "plan";
const SUPPORTED_SOURCES: &str = "auto, nanobot, openclaw, picoclaw, zeroclaw, nanoclaw";

pub(super) fn execute_claw_import_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "claw.import payload must be an object".to_owned())?;
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_MODE);
    if !matches!(mode, "plan" | "apply") {
        return Err(format!(
            "claw.import payload.mode must be `plan` or `apply`, got `{mode}`"
        ));
    }

    let input_path = payload
        .get("input_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "claw.import requires payload.input_path".to_owned())?;
    let input_path = resolve_safe_path_with_config(input_path, config)?;

    let output_path = payload
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_safe_path_with_config(value, config))
        .transpose()?;

    if mode == "apply" && output_path.is_none() {
        return Err("claw.import apply mode requires payload.output_path".to_owned());
    }

    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let hint = payload
        .get("source")
        .and_then(Value::as_str)
        .map(parse_source_hint)
        .transpose()?
        .flatten();
    let plan = migration::plan_import_from_path(&input_path, hint)?;

    let mut merged_config = load_or_default_config(output_path.as_deref())?;
    migration::apply_import_plan(&mut merged_config, &plan);
    let config_toml = config::render(&merged_config)?;

    let written_output_path = if mode == "apply" {
        let output_path = output_path
            .clone()
            .expect("output path already required in apply mode");
        let output_string = output_path.display().to_string();
        Some(config::write(Some(&output_string), &merged_config, force)?)
    } else {
        None
    };
    let response_output_path = written_output_path
        .as_ref()
        .or(output_path.as_ref())
        .map(|path| path.display().to_string());

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "mode": mode,
            "source": plan.source.as_id(),
            "input_path": input_path.display().to_string(),
            "output_path": response_output_path,
            "config_written": mode == "apply",
            "warnings": plan.warnings,
            "config_preview": config_preview_payload(&merged_config),
            "config_toml": config_toml,
            "next_step": written_output_path
                .as_ref()
                .map(|path| format!("loongclawd chat --config {}", path.display())),
        }),
    })
}

fn parse_source_hint(raw: &str) -> Result<Option<LegacyClawSource>, String> {
    let parsed = LegacyClawSource::from_id(raw).ok_or_else(|| {
        format!("unsupported claw.import payload.source `{raw}`. supported: {SUPPORTED_SOURCES}")
    })?;
    if matches!(parsed, LegacyClawSource::Unknown) {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

fn load_or_default_config(path: Option<&Path>) -> Result<LoongClawConfig, String> {
    let Some(path) = path else {
        return Ok(LoongClawConfig::default());
    };
    if !path.exists() {
        return Ok(LoongClawConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = config::load(Some(&path_string))?;
    Ok(config)
}

fn config_preview_payload(config: &LoongClawConfig) -> Value {
    json!({
        "prompt_pack_id": config
            .cli
            .prompt_pack_id()
            .unwrap_or(crate::prompt::DEFAULT_PROMPT_PACK_ID),
        "memory_profile": memory_profile_id(config.memory.profile),
        "system_prompt_addendum": config.cli.system_prompt_addendum.clone(),
        "profile_note": config.memory.profile_note.clone(),
    })
}

fn memory_profile_id(profile: MemoryProfile) -> &'static str {
    match profile {
        MemoryProfile::WindowOnly => "window_only",
        MemoryProfile::WindowPlusSummary => "window_plus_summary",
        MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

fn resolve_safe_path_with_config(
    raw: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    if config.file_root.is_none() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let candidate = Path::new(raw);
        let combined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        return canonicalize_or_fallback(combined);
    }

    let root = config
        .file_root
        .clone()
        .expect("file_root already checked above");
    let root = canonicalize_or_fallback(root)?;

    let candidate = Path::new(raw);
    let combined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = normalize_without_fs_access(&combined);
    resolve_path_within_root(&root, &normalized)
}

fn canonicalize_or_fallback(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
    }
    Ok(normalize_without_fs_access(&path))
}

fn resolve_path_within_root(root: &Path, normalized: &Path) -> Result<PathBuf, String> {
    ensure_path_within_root(root, normalized)?;

    if normalized.exists() {
        let canonical = fs::canonicalize(normalized).map_err(|error| {
            format!(
                "failed to canonicalize target path {}: {error}",
                normalized.display()
            )
        })?;
        ensure_path_within_root(root, &canonical)?;
        return Ok(canonical);
    }

    let (ancestor, suffix) = split_existing_ancestor(normalized)?;
    let canonical_ancestor = fs::canonicalize(&ancestor).map_err(|error| {
        format!(
            "failed to canonicalize ancestor {}: {error}",
            ancestor.display()
        )
    })?;
    ensure_path_within_root(root, &canonical_ancestor)?;

    let mut reconstructed = canonical_ancestor;
    for component in suffix {
        reconstructed.push(component);
    }
    ensure_path_within_root(root, &reconstructed)?;
    Ok(reconstructed)
}

fn ensure_path_within_root(root: &Path, path: &Path) -> Result<(), String> {
    if path.starts_with(root) {
        return Ok(());
    }
    Err(format!(
        "migration path {} escapes configured file root {}",
        path.display(),
        root.display()
    ))
}

fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), String> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();

    loop {
        if cursor.exists() {
            suffix.reverse();
            return Ok((cursor, suffix));
        }

        let Some(name) = cursor.file_name().map(|value| value.to_owned()) else {
            return Err(format!(
                "cannot resolve existing ancestor for {}",
                path.display()
            ));
        };
        suffix.push(name);
        let Some(parent) = cursor.parent() else {
            return Err(format!(
                "cannot resolve existing ancestor for {}",
                path.display()
            ));
        };
        cursor = parent.to_path_buf();
    }
}

fn normalize_without_fs_access(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut parts: Vec<OsString> = Vec::new();
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_owned()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = parts.last() {
                    if last != ".." {
                        let _ = parts.pop();
                    } else if !has_root {
                        parts.push(OsString::from(".."));
                    }
                } else if !has_root {
                    parts.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => parts.push(value.to_owned()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for part in parts {
        normalized.push(part);
    }
    if normalized.as_os_str().is_empty() {
        if has_root {
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}
