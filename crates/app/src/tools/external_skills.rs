use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flate2::read::GzDecoder;
use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tar::Archive;

const DEFAULT_DOWNLOAD_DIR_NAME: &str = "external-skills-downloads";
const DEFAULT_INSTALL_DIR_NAME: &str = "external-skills-installed";
const DEFAULT_SKILL_FILENAME: &str = "SKILL.md";
const DEFAULT_INDEX_FILENAME: &str = "index.json";
const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;
const HARD_MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InstalledSkillEntry {
    skill_id: String,
    display_name: String,
    summary: String,
    source_kind: String,
    source_path: String,
    install_path: String,
    skill_md_path: String,
    sha256: String,
    installed_at_unix: u64,
    active: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct InstalledSkillIndex {
    skills: Vec<InstalledSkillEntry>,
}

#[derive(Debug, Clone, Default)]
struct ExternalSkillsPolicyOverride {
    enabled: Option<bool>,
    require_download_approval: Option<bool>,
    allowed_domains: Option<BTreeSet<String>>,
    blocked_domains: Option<BTreeSet<String>>,
}

static EXTERNAL_SKILLS_POLICY_OVERRIDE: OnceLock<RwLock<ExternalSkillsPolicyOverride>> =
    OnceLock::new();

pub(super) fn execute_external_skills_policy_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "external_skills.policy payload must be an object".to_owned())?;
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("get")
        .to_ascii_lowercase();

    if !matches!(action.as_str(), "get" | "set" | "reset") {
        return Err(format!(
            "external_skills.policy payload.action must be `get`, `set`, or `reset`, got `{action}`"
        ));
    }

    match action.as_str() {
        "get" => {
            let effective_policy = resolve_effective_policy(config)?;
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "core-tools",
                    "tool_name": request.tool_name,
                    "action": "get",
                    "policy": policy_payload(&effective_policy),
                    "override_active": policy_override_is_active()?,
                }),
            })
        }
        "set" => {
            let policy_update_approved = payload
                .get("policy_update_approved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !policy_update_approved {
                return Err(
                    "external skills policy update requires explicit authorization; set payload.policy_update_approved=true after user approval"
                        .to_owned(),
                );
            }

            let enabled = parse_optional_bool(payload, "enabled")?;
            let require_download_approval =
                parse_optional_bool(payload, "require_download_approval")?;
            let allowed_domains = parse_optional_domain_list(payload, "allowed_domains")?;
            let blocked_domains = parse_optional_domain_list(payload, "blocked_domains")?;

            let override_store = policy_override_store();
            let mut override_state = override_store
                .write()
                .map_err(|error| format!("external skills policy lock poisoned: {error}"))?;

            if let Some(value) = enabled {
                override_state.enabled = Some(value);
            }
            if let Some(value) = require_download_approval {
                override_state.require_download_approval = Some(value);
            }
            if let Some(value) = allowed_domains {
                override_state.allowed_domains = Some(value);
            }
            if let Some(value) = blocked_domains {
                override_state.blocked_domains = Some(value);
            }

            let effective_policy = build_effective_policy(config, &override_state);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "core-tools",
                    "tool_name": request.tool_name,
                    "action": "set",
                    "policy_update_approved": policy_update_approved,
                    "policy": policy_payload(&effective_policy),
                    "override_active": override_state.has_values(),
                }),
            })
        }
        "reset" => {
            let policy_update_approved = payload
                .get("policy_update_approved")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !policy_update_approved {
                return Err(
                    "external skills policy update requires explicit authorization; set payload.policy_update_approved=true after user approval"
                        .to_owned(),
                );
            }

            let override_store = policy_override_store();
            let mut override_state = override_store
                .write()
                .map_err(|error| format!("external skills policy lock poisoned: {error}"))?;
            *override_state = ExternalSkillsPolicyOverride::default();

            let effective_policy = build_effective_policy(config, &override_state);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "core-tools",
                    "tool_name": request.tool_name,
                    "action": "reset",
                    "policy_update_approved": policy_update_approved,
                    "policy": policy_payload(&effective_policy),
                    "override_active": false,
                }),
            })
        }
        _ => Err("unreachable external skills policy action".to_owned()),
    }
}

pub(super) fn execute_external_skills_fetch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "external_skills.fetch payload must be an object".to_owned())?;

    let url = payload
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "external_skills.fetch requires payload.url".to_owned())?;

    let parsed_url = reqwest::Url::parse(url)
        .map_err(|error| format!("invalid external skills url `{url}`: {error}"))?;
    let host = parsed_url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("external skills url `{url}` has no host"))?;
    if parsed_url.scheme() != "https" {
        return Err(format!(
            "external skills download requires https url, got scheme `{}`",
            parsed_url.scheme()
        ));
    }

    let policy = require_enabled_runtime_policy(config)?;

    if let Some(rule) = first_matching_domain_rule(&host, &policy.blocked_domains) {
        return Err(format!(
            "external skills download blocked: host `{host}` matches blocked domain rule `{rule}`"
        ));
    }

    if !policy.allowed_domains.is_empty()
        && first_matching_domain_rule(&host, &policy.allowed_domains).is_none()
    {
        return Err(format!(
            "external skills download denied: host `{host}` is not in allowed_domains"
        ));
    }

    let approval_granted = payload
        .get("approval_granted")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if policy.require_download_approval && !approval_granted {
        return Err(
            "external skills download requires explicit authorization; set payload.approval_granted=true after user approval"
                .to_owned(),
        );
    }

    let max_bytes = parse_max_download_bytes(payload)?;
    let save_as = parse_optional_string(payload, "save_as")?;

    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| {
            format!("failed to build HTTP client for external skills download: {error}")
        })?;

    let response = client
        .get(parsed_url.clone())
        .send()
        .map_err(|error| format!("external skills download request failed: {error}"))?;

    if response.status().is_redirection() {
        return Err(format!(
            "external skills download rejected redirect response {} for `{url}`",
            response.status()
        ));
    }

    if !response.status().is_success() {
        return Err(format!(
            "external skills download returned non-success status {} for `{url}`",
            response.status()
        ));
    }

    let mut body = Vec::new();
    let mut limited_reader = response.take((max_bytes as u64).saturating_add(1));
    limited_reader
        .read_to_end(&mut body)
        .map_err(|error| format!("failed to read external skills download body: {error}"))?;

    if body.len() > max_bytes {
        return Err(format!(
            "external skills download exceeded max_bytes limit ({max_bytes} bytes)"
        ));
    }

    let output_dir = resolve_download_dir(config);
    fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create external skills download directory {}: {error}",
            output_dir.display()
        )
    })?;

    let requested_name = save_as
        .as_deref()
        .map(sanitize_filename)
        .filter(|value| !value.is_empty());
    let derived_name = requested_name.unwrap_or_else(|| derive_filename_from_url(&parsed_url));
    let output_path = unique_output_path(&output_dir, &derived_name);

    fs::write(&output_path, &body).map_err(|error| {
        format!(
            "failed to write downloaded external skill artifact {}: {error}",
            output_path.display()
        )
    })?;

    let sha256 = format!("{:x}", Sha256::digest(&body));

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "url": url,
            "host": host,
            "saved_path": output_path.display().to_string(),
            "bytes_downloaded": body.len(),
            "sha256": sha256,
            "approval_required": policy.require_download_approval,
            "approval_granted": approval_granted,
            "max_bytes": max_bytes,
            "policy": policy_payload(&policy),
        }),
    })
}

pub(super) fn execute_external_skills_install_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "external_skills.install payload must be an object".to_owned())?;
    let raw_path = payload
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "external_skills.install requires payload.path".to_owned())?;
    let replace = payload
        .get("replace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let explicit_skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim);

    require_enabled_runtime_policy(config)?;

    let source_path = super::file::resolve_safe_file_path_with_config(raw_path, config)?;
    let install_root = resolve_install_root(config);
    fs::create_dir_all(&install_root).map_err(|error| {
        format!(
            "failed to create external skills install root {}: {error}",
            install_root.display()
        )
    })?;

    let (skill_root, source_kind) = if source_path.is_dir() {
        let skill_root = resolve_skill_root(&source_path)?;
        (skill_root, "directory")
    } else {
        let skill_root = extract_archive_to_staging(&source_path, &install_root)?;
        (skill_root, "archive")
    };

    let skill_md_path = skill_root.join(DEFAULT_SKILL_FILENAME);
    let skill_markdown = fs::read_to_string(&skill_md_path).map_err(|error| {
        format!(
            "failed to read installed skill source {}: {error}",
            skill_md_path.display()
        )
    })?;
    let skill_id = explicit_skill_id
        .and_then(|value| (!value.is_empty()).then_some(value))
        .map(normalize_skill_id)
        .transpose()?
        .unwrap_or_else(|| derive_skill_id(&skill_root));
    let display_name = derive_skill_display_name(skill_markdown.as_str(), skill_id.as_str());
    let summary = derive_skill_summary(skill_markdown.as_str());

    let mut index = load_installed_skill_index(&install_root)?;
    if !replace && index.skills.iter().any(|entry| entry.skill_id == skill_id) {
        return Err(format!(
            "external skill `{skill_id}` is already installed; pass payload.replace=true to replace it"
        ));
    }

    let destination_root = install_root.join(skill_id.as_str());
    if destination_root.exists() {
        fs::remove_dir_all(&destination_root).map_err(|error| {
            format!(
                "failed to remove existing installed skill {}: {error}",
                destination_root.display()
            )
        })?;
    }
    copy_dir_recursive(&skill_root, &destination_root)?;

    let installed_skill_md_path = destination_root.join(DEFAULT_SKILL_FILENAME);
    let installed_skill_markdown =
        fs::read_to_string(&installed_skill_md_path).map_err(|error| {
            format!(
                "failed to verify installed skill {}: {error}",
                installed_skill_md_path.display()
            )
        })?;
    let digest = format!("{:x}", Sha256::digest(installed_skill_markdown.as_bytes()));
    let installed_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    index.skills.retain(|entry| entry.skill_id != skill_id);
    index.skills.push(InstalledSkillEntry {
        skill_id: skill_id.clone(),
        display_name,
        summary,
        source_kind: source_kind.to_owned(),
        source_path: source_path.display().to_string(),
        install_path: destination_root.display().to_string(),
        skill_md_path: installed_skill_md_path.display().to_string(),
        sha256: digest.clone(),
        installed_at_unix,
        active: true,
    });
    persist_installed_skill_index(&install_root, &mut index)?;

    if source_kind == "archive" {
        fs::remove_dir_all(&skill_root).ok();
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skill_id": skill_id,
            "source_kind": source_kind,
            "source_path": source_path.display().to_string(),
            "install_path": destination_root.display().to_string(),
            "skill_md_path": installed_skill_md_path.display().to_string(),
            "sha256": digest,
            "replaced": replace,
        }),
    })
}

pub(super) fn execute_external_skills_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let install_root = resolve_install_root(config);
    let index = load_installed_skill_index(&install_root)?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skills": index.skills,
        }),
    })
}

pub(super) fn execute_external_skills_inspect_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "external_skills.inspect payload must be an object".to_owned())?;
    let skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "external_skills.inspect requires payload.skill_id".to_owned())?;
    let install_root = resolve_install_root(config);
    let entry = installed_skill_by_id(&load_installed_skill_index(&install_root)?, skill_id)?;
    let instructions = fs::read_to_string(&entry.skill_md_path).map_err(|error| {
        format!(
            "failed to read installed skill {}: {error}",
            entry.skill_md_path
        )
    })?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skill": entry,
            "instructions_preview": build_preview(instructions.as_str(), 240),
        }),
    })
}

pub(super) fn execute_external_skills_invoke_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "external_skills.invoke payload must be an object".to_owned())?;
    let skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "external_skills.invoke requires payload.skill_id".to_owned())?;

    require_enabled_runtime_policy(config)?;

    let install_root = resolve_install_root(config);
    let entry = installed_skill_by_id(&load_installed_skill_index(&install_root)?, skill_id)?;
    if !entry.active {
        return Err(format!(
            "external skill `{skill_id}` is installed but inactive"
        ));
    }
    let instructions = fs::read_to_string(&entry.skill_md_path).map_err(|error| {
        format!(
            "failed to read installed skill {}: {error}",
            entry.skill_md_path
        )
    })?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skill_id": entry.skill_id,
            "display_name": entry.display_name,
            "summary": entry.summary,
            "install_path": entry.install_path,
            "skill_md_path": entry.skill_md_path,
            "instructions": instructions,
            "invocation_summary": format!(
                "Loaded external skill `{}`. Apply the instructions in `SKILL.md` before continuing the task.",
                skill_id
            ),
        }),
    })
}

pub(super) fn execute_external_skills_remove_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "external_skills.remove payload must be an object".to_owned())?;
    let skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "external_skills.remove requires payload.skill_id".to_owned())?;

    let install_root = resolve_install_root(config);
    let mut index = load_installed_skill_index(&install_root)?;
    let Some(position) = index
        .skills
        .iter()
        .position(|entry| entry.skill_id == skill_id)
    else {
        return Err(format!("external skill `{skill_id}` is not installed"));
    };
    let entry = index.skills.remove(position);
    let install_path = PathBuf::from(entry.install_path.clone());
    if install_path.exists() {
        fs::remove_dir_all(&install_path).map_err(|error| {
            format!(
                "failed to remove installed skill {}: {error}",
                install_path.display()
            )
        })?;
    }
    persist_installed_skill_index(&install_root, &mut index)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skill_id": skill_id,
            "removed": true,
        }),
    })
}

fn policy_override_store() -> &'static RwLock<ExternalSkillsPolicyOverride> {
    EXTERNAL_SKILLS_POLICY_OVERRIDE
        .get_or_init(|| RwLock::new(ExternalSkillsPolicyOverride::default()))
}

fn policy_override_is_active() -> Result<bool, String> {
    let guard = policy_override_store()
        .read()
        .map_err(|error| format!("external skills policy lock poisoned: {error}"))?;
    Ok(guard.has_values())
}

fn resolve_effective_policy(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<super::runtime_config::ExternalSkillsRuntimePolicy, String> {
    let override_state = policy_override_store()
        .read()
        .map_err(|error| format!("external skills policy lock poisoned: {error}"))?;
    Ok(build_effective_policy(config, &override_state))
}

fn require_enabled_runtime_policy(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<super::runtime_config::ExternalSkillsRuntimePolicy, String> {
    let policy = resolve_effective_policy(config)?;
    if !policy.enabled {
        return Err(
            "external skills runtime is disabled; enable `external_skills.enabled = true` first"
                .to_owned(),
        );
    }
    Ok(policy)
}

fn build_effective_policy(
    config: &super::runtime_config::ToolRuntimeConfig,
    override_state: &ExternalSkillsPolicyOverride,
) -> super::runtime_config::ExternalSkillsRuntimePolicy {
    let mut effective = config.external_skills.clone();
    if let Some(value) = override_state.enabled {
        effective.enabled = value;
    }
    if let Some(value) = override_state.require_download_approval {
        effective.require_download_approval = value;
    }
    if let Some(value) = override_state.allowed_domains.as_ref() {
        effective.allowed_domains = value.clone();
    }
    if let Some(value) = override_state.blocked_domains.as_ref() {
        effective.blocked_domains = value.clone();
    }
    effective
}

impl ExternalSkillsPolicyOverride {
    fn has_values(&self) -> bool {
        self.enabled.is_some()
            || self.require_download_approval.is_some()
            || self.allowed_domains.is_some()
            || self.blocked_domains.is_some()
    }
}

fn parse_optional_bool(payload: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    let parsed = value
        .as_bool()
        .ok_or_else(|| format!("external_skills.policy payload.{key} must be a boolean"))?;
    Ok(Some(parsed))
}

fn parse_optional_string(
    payload: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    let parsed = value
        .as_str()
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .ok_or_else(|| format!("external_skills.fetch payload.{key} must be a non-empty string"))?;
    Ok(Some(parsed.to_owned()))
}

fn parse_optional_domain_list(
    payload: &Map<String, Value>,
    key: &str,
) -> Result<Option<BTreeSet<String>>, String> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };

    let items = value.as_array().ok_or_else(|| {
        format!("external_skills.policy payload.{key} must be an array of strings")
    })?;

    let mut normalized = BTreeSet::new();
    for item in items {
        let raw = item.as_str().ok_or_else(|| {
            format!("external_skills.policy payload.{key} must contain only strings")
        })?;
        let rule = normalize_domain_rule(raw)
            .map_err(|error| format!("invalid domain rule in payload.{key}: {error}"))?;
        normalized.insert(rule);
    }

    Ok(Some(normalized))
}

fn normalize_domain_rule(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("domain rule cannot be empty".to_owned());
    }

    let mut wildcard = false;
    let lowered = trimmed.to_ascii_lowercase();
    let mut candidate = if let Some(rest) = lowered.strip_prefix("*.") {
        wildcard = true;
        rest.to_owned()
    } else {
        lowered
    };

    if candidate.contains("://") {
        let parsed = reqwest::Url::parse(trimmed)
            .map_err(|error| format!("invalid domain/url `{trimmed}`: {error}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| format!("domain/url `{trimmed}` has no host"))?;
        candidate = host.to_ascii_lowercase();
        wildcard = false;
    }

    let candidate = candidate.trim_end_matches('.').to_owned();
    if candidate.is_empty() {
        return Err("domain rule cannot be empty".to_owned());
    }

    if candidate.starts_with('.') || candidate.ends_with('.') || candidate.contains("..") {
        return Err(format!("invalid domain `{candidate}`"));
    }

    let valid_chars = candidate
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.'));
    if !valid_chars {
        return Err(format!("invalid domain `{candidate}`"));
    }

    if candidate != "localhost" && !candidate.contains('.') {
        return Err(format!(
            "domain `{candidate}` must contain a dot or be localhost"
        ));
    }

    if wildcard {
        Ok(format!("*.{candidate}"))
    } else {
        Ok(candidate)
    }
}

fn first_matching_domain_rule<'a>(host: &str, rules: &'a BTreeSet<String>) -> Option<&'a str> {
    for rule in rules {
        if domain_rule_matches(host, rule) {
            return Some(rule.as_str());
        }
    }
    None
}

fn domain_rule_matches(host: &str, rule: &str) -> bool {
    if let Some(suffix) = rule.strip_prefix("*.") {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }
    host == rule
}

fn parse_max_download_bytes(payload: &Map<String, Value>) -> Result<usize, String> {
    let Some(value) = payload.get("max_bytes") else {
        return Ok(DEFAULT_MAX_DOWNLOAD_BYTES);
    };
    let parsed = value
        .as_u64()
        .ok_or_else(|| "external_skills.fetch payload.max_bytes must be an integer".to_owned())?;
    if parsed == 0 {
        return Err("external_skills.fetch payload.max_bytes must be >= 1".to_owned());
    }
    let capped = parsed.min(HARD_MAX_DOWNLOAD_BYTES as u64);
    usize::try_from(capped).map_err(|error| format!("invalid max_bytes `{parsed}`: {error}"))
}

fn resolve_download_dir(config: &super::runtime_config::ToolRuntimeConfig) -> PathBuf {
    let root = config
        .file_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    root.join(DEFAULT_DOWNLOAD_DIR_NAME)
}

fn derive_filename_from_url(url: &reqwest::Url) -> String {
    let from_path = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("skill-package.bin");
    let sanitized = sanitize_filename(from_path);
    if sanitized.is_empty() {
        "skill-package.bin".to_owned()
    } else {
        sanitized
    }
}

fn sanitize_filename(raw: &str) -> String {
    let mut normalized = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            normalized.push(ch);
        } else {
            normalized.push('_');
        }
    }
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "skill-package.bin".to_owned()
    } else {
        normalized.to_owned()
    }
}

fn unique_output_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }

    let (stem, ext) = split_stem_and_ext(filename);
    for index in 1..=9_999usize {
        let name = if ext.is_empty() {
            format!("{stem}-{index}")
        } else {
            format!("{stem}-{index}.{ext}")
        };
        let next = dir.join(name);
        if !next.exists() {
            return next;
        }
    }

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if ext.is_empty() {
        dir.join(format!("{stem}-{suffix}"))
    } else {
        dir.join(format!("{stem}-{suffix}.{ext}"))
    }
}

fn split_stem_and_ext(filename: &str) -> (&str, &str) {
    if let Some((stem, ext)) = filename.rsplit_once('.')
        && !stem.is_empty()
        && !ext.is_empty()
    {
        return (stem, ext);
    }
    (filename, "")
}

fn resolve_install_root(config: &super::runtime_config::ToolRuntimeConfig) -> PathBuf {
    if let Some(path) = config.external_skills.install_root.clone() {
        return path;
    }
    let root = config
        .file_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    root.join(DEFAULT_INSTALL_DIR_NAME)
}

fn resolve_skill_root(root: &Path) -> Result<PathBuf, String> {
    if root.join(DEFAULT_SKILL_FILENAME).is_file() {
        return Ok(root.to_path_buf());
    }
    let candidates = find_skill_roots(root)?;
    match candidates.as_slice() {
        [] => Err(format!(
            "external skill source {} does not contain `{DEFAULT_SKILL_FILENAME}`",
            root.display()
        )),
        [single] => Ok(single.clone()),
        _ => Err(format!(
            "external skill source {} contains multiple `{DEFAULT_SKILL_FILENAME}` roots; provide a more specific path",
            root.display()
        )),
    }
}

fn extract_archive_to_staging(archive_path: &Path, install_root: &Path) -> Result<PathBuf, String> {
    let filename = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !(filename.ends_with(".tgz") || filename.ends_with(".tar.gz")) {
        return Err(format!(
            "external skill archive {} must end with .tgz or .tar.gz",
            archive_path.display()
        ));
    }

    let staging_root = install_root.join(format!(
        ".staging-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&staging_root).map_err(|error| {
        format!(
            "failed to create external skill staging directory {}: {error}",
            staging_root.display()
        )
    })?;
    let file = fs::File::open(archive_path).map_err(|error| {
        format!(
            "failed to open external skill archive {}: {error}",
            archive_path.display()
        )
    })?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    for entry in archive.entries().map_err(|error| {
        format!(
            "failed to read external skill archive {}: {error}",
            archive_path.display()
        )
    })? {
        let mut entry = entry.map_err(|error| {
            format!(
                "failed to inspect external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
        entry.unpack_in(&staging_root).map_err(|error| {
            format!(
                "failed to extract external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
    }
    resolve_skill_root(&staging_root)
}

fn find_skill_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    visit_skill_roots(root, &mut roots)?;
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn visit_skill_roots(root: &Path, roots: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata = fs::metadata(root).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Ok(());
    }
    if root.join(DEFAULT_SKILL_FILENAME).is_file() {
        roots.push(root.to_path_buf());
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|error| {
        format!(
            "failed to read external skill source {}: {error}",
            root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to traverse external skill source {}: {error}",
                root.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            visit_skill_roots(&path, roots)?;
        }
    }
    Ok(())
}

fn derive_skill_id(root: &Path) -> String {
    let fallback = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("external-skill");
    normalize_skill_id(fallback).unwrap_or_else(|_| "external-skill".to_owned())
}

fn normalize_skill_id(raw: &str) -> Result<String, String> {
    let mut normalized = String::new();
    let mut last_dash = false;
    for ch in raw.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_' | ' ' | '.') {
            Some('-')
        } else {
            None
        };
        if let Some(value) = mapped {
            if value == '-' {
                if !last_dash {
                    normalized.push(value);
                }
                last_dash = true;
            } else {
                normalized.push(value);
                last_dash = false;
            }
        }
    }
    let normalized = normalized.trim_matches('-').to_owned();
    if normalized.is_empty() {
        return Err(format!("invalid external skill id `{raw}`"));
    }
    Ok(normalized)
}

fn derive_skill_display_name(skill_markdown: &str, fallback: &str) -> String {
    for line in skill_markdown.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return title.to_owned();
            }
        }
    }
    fallback.to_owned()
}

fn derive_skill_summary(skill_markdown: &str) -> String {
    for line in skill_markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return build_preview(trimmed, 120);
    }
    "No summary provided.".to_owned()
}

fn build_preview(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_owned();
    }
    let mut out = String::new();
    for ch in content.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create external skill destination {}: {error}",
            destination.display()
        )
    })?;
    for entry in fs::read_dir(source).map_err(|error| {
        format!(
            "failed to read external skill source {}: {error}",
            source.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to traverse external skill source {}: {error}",
                source.display()
            )
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "failed to copy external skill file {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn load_installed_skill_index(root: &Path) -> Result<InstalledSkillIndex, String> {
    let index_path = root.join(DEFAULT_INDEX_FILENAME);
    if !index_path.exists() {
        return Ok(InstalledSkillIndex::default());
    }
    let raw = fs::read_to_string(&index_path).map_err(|error| {
        format!(
            "failed to read external skills index {}: {error}",
            index_path.display()
        )
    })?;
    let mut index: InstalledSkillIndex = serde_json::from_str(raw.as_str()).map_err(|error| {
        format!(
            "failed to parse external skills index {}: {error}",
            index_path.display()
        )
    })?;
    index
        .skills
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    Ok(index)
}

fn persist_installed_skill_index(
    root: &Path,
    index: &mut InstalledSkillIndex,
) -> Result<(), String> {
    index
        .skills
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    fs::create_dir_all(root).map_err(|error| {
        format!(
            "failed to create external skills install root {}: {error}",
            root.display()
        )
    })?;
    let index_path = root.join(DEFAULT_INDEX_FILENAME);
    let encoded = serde_json::to_string_pretty(index)
        .map_err(|error| format!("failed to encode external skills index: {error}"))?;
    fs::write(&index_path, encoded).map_err(|error| {
        format!(
            "failed to write external skills index {}: {error}",
            index_path.display()
        )
    })
}

fn installed_skill_by_id(
    index: &InstalledSkillIndex,
    skill_id: &str,
) -> Result<InstalledSkillEntry, String> {
    index
        .skills
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned()
        .ok_or_else(|| format!("external skill `{skill_id}` is not installed"))
}

pub(super) fn installed_skill_snapshot_lines_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<String>, String> {
    let policy = resolve_effective_policy(config)?;
    if !policy.enabled || !policy.auto_expose_installed {
        return Ok(Vec::new());
    }
    let install_root = resolve_install_root(config);
    let index = load_installed_skill_index(&install_root)?;
    Ok(index
        .skills
        .into_iter()
        .filter(|entry| entry.active)
        .map(|entry| format!("- {}: {}", entry.skill_id, entry.summary))
        .collect())
}

fn policy_payload(policy: &super::runtime_config::ExternalSkillsRuntimePolicy) -> Value {
    json!({
        "enabled": policy.enabled,
        "require_download_approval": policy.require_download_approval,
        "allowed_domains": policy.allowed_domains.iter().cloned().collect::<Vec<_>>(),
        "blocked_domains": policy.blocked_domains.iter().cloned().collect::<Vec<_>>(),
        "install_root": policy.install_root.as_ref().map(|path| path.display().to_string()),
        "auto_expose_installed": policy.auto_expose_installed,
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;
    use crate::tools::runtime_config::{ExternalSkillsRuntimePolicy, ToolRuntimeConfig};

    static POLICY_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_policy_test_lock<T>(f: impl FnOnce() -> T) -> T {
        let lock = POLICY_TEST_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().expect("policy test lock");
        f()
    }

    fn reset_policy_override_for_test() {
        if let Some(store) = EXTERNAL_SKILLS_POLICY_OVERRIDE.get()
            && let Ok(mut guard) = store.write()
        {
            *guard = ExternalSkillsPolicyOverride::default();
        }
    }

    fn base_runtime_config() -> ToolRuntimeConfig {
        ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(std::env::temp_dir().join("loongclaw-ext-skills-tests")),
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: false,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: true,
            },
        }
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    fn managed_runtime_config(root: &Path) -> ToolRuntimeConfig {
        ToolRuntimeConfig {
            shell_allowlist: BTreeSet::new(),
            file_root: Some(root.to_path_buf()),
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: true,
            },
        }
    }

    #[test]
    fn normalize_domain_rule_accepts_exact_and_wildcard_domains() {
        assert_eq!(
            normalize_domain_rule("skills.sh").expect("normalize"),
            "skills.sh"
        );
        assert_eq!(
            normalize_domain_rule("*.clawhub.io").expect("normalize wildcard"),
            "*.clawhub.io"
        );
        assert!(normalize_domain_rule("not-a-domain").is_err());
    }

    #[test]
    fn domain_rule_matching_supports_subdomains() {
        assert!(domain_rule_matches("api.skills.sh", "*.skills.sh"));
        assert!(domain_rule_matches("skills.sh", "*.skills.sh"));
        assert!(!domain_rule_matches("skills.sh", "*.clawhub.io"));
        assert!(domain_rule_matches("skills.sh", "skills.sh"));
    }

    #[test]
    fn policy_tool_set_and_reset_override_runtime_policy() {
        with_policy_test_lock(|| {
            reset_policy_override_for_test();
            let config = base_runtime_config();

            let set_outcome = execute_external_skills_policy_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.policy".to_owned(),
                    payload: json!({
                        "action": "set",
                        "policy_update_approved": true,
                        "enabled": true,
                        "allowed_domains": ["skills.sh"],
                        "blocked_domains": ["*.evil.example"]
                    }),
                },
                &config,
            )
            .expect("set policy should succeed");

            assert_eq!(set_outcome.status, "ok");
            assert_eq!(set_outcome.payload["policy"]["enabled"], json!(true));
            assert_eq!(
                set_outcome.payload["policy"]["allowed_domains"],
                json!(["skills.sh"])
            );
            assert_eq!(set_outcome.payload["override_active"], json!(true));

            let reset_outcome = execute_external_skills_policy_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.policy".to_owned(),
                    payload: json!({
                        "action": "reset",
                        "policy_update_approved": true
                    }),
                },
                &config,
            )
            .expect("reset policy should succeed");
            assert_eq!(reset_outcome.status, "ok");
            assert_eq!(reset_outcome.payload["policy"]["enabled"], json!(false));
            assert_eq!(reset_outcome.payload["override_active"], json!(false));
        });
    }

    #[test]
    fn policy_tool_set_requires_explicit_authorization() {
        with_policy_test_lock(|| {
            reset_policy_override_for_test();
            let config = base_runtime_config();

            let error = execute_external_skills_policy_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.policy".to_owned(),
                    payload: json!({
                        "action": "set",
                        "enabled": true
                    }),
                },
                &config,
            )
            .expect_err("policy update should require explicit authorization");

            assert!(error.contains("policy update requires explicit authorization"));
        });
    }

    #[test]
    fn fetch_requires_enabled_runtime() {
        with_policy_test_lock(|| {
            reset_policy_override_for_test();
            let config = base_runtime_config();

            let error = execute_external_skills_fetch_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.fetch".to_owned(),
                    payload: json!({
                        "url": "https://skills.sh/demo.tgz",
                        "approval_granted": true
                    }),
                },
                &config,
            )
            .expect_err("disabled runtime must fail");

            assert!(error.contains("external skills runtime is disabled"));
        });
    }

    #[test]
    fn fetch_rejects_non_https_urls() {
        with_policy_test_lock(|| {
            reset_policy_override_for_test();
            let config = base_runtime_config();

            let error = execute_external_skills_fetch_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.fetch".to_owned(),
                    payload: json!({
                        "url": "http://skills.sh/demo.tgz",
                        "approval_granted": true
                    }),
                },
                &config,
            )
            .expect_err("non-https url must fail");

            assert!(error.contains("requires https url"));
        });
    }

    #[test]
    fn fetch_checks_domain_policy_and_approval_before_network() {
        with_policy_test_lock(|| {
            reset_policy_override_for_test();
            let config = base_runtime_config();

            execute_external_skills_policy_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.policy".to_owned(),
                    payload: json!({
                        "action": "set",
                        "policy_update_approved": true,
                        "enabled": true,
                        "require_download_approval": true,
                        "allowed_domains": ["skills.sh"],
                        "blocked_domains": ["*.evil.example"]
                    }),
                },
                &config,
            )
            .expect("set policy should succeed");

            let approval_error = execute_external_skills_fetch_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.fetch".to_owned(),
                    payload: json!({
                        "url": "https://skills.sh/demo.tgz"
                    }),
                },
                &config,
            )
            .expect_err("approval should be required");
            assert!(approval_error.contains("requires explicit authorization"));

            let deny_error = execute_external_skills_fetch_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.fetch".to_owned(),
                    payload: json!({
                        "url": "https://cdn.evil.example/demo.tgz",
                        "approval_granted": true
                    }),
                },
                &config,
            )
            .expect_err("blocked domains should be denied");
            assert!(deny_error.contains("matches blocked domain rule"));

            let allowlist_error = execute_external_skills_fetch_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.fetch".to_owned(),
                    payload: json!({
                        "url": "https://clawhub.io/demo.tgz",
                        "approval_granted": true
                    }),
                },
                &config,
            )
            .expect_err("non-allowlisted domain should be rejected");
            assert!(allowlist_error.contains("not in allowed_domains"));

            execute_external_skills_policy_tool_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.policy".to_owned(),
                    payload: json!({
                        "action": "reset",
                        "policy_update_approved": true
                    }),
                },
                &config,
            )
            .expect("reset policy should succeed");
        });
    }

    #[test]
    fn install_from_directory_writes_managed_index_and_copy() {
        let root = unique_temp_dir("loongclaw-ext-skill-install-dir");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nUse this skill when the task needs deployment discipline.\n",
        );
        let config = managed_runtime_config(&root);

        let outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &config,
        )
        .expect("install should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["skill_id"], "demo-skill");
        assert!(
            root.join("external-skills-installed")
                .join("index.json")
                .exists(),
            "managed external skill index should exist"
        );
        assert!(
            root.join("external-skills-installed")
                .join("demo-skill")
                .join("SKILL.md")
                .exists(),
            "managed external skill copy should exist"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_requires_enabled_runtime() {
        let root = unique_temp_dir("loongclaw-ext-skill-install-disabled");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nInstall should require enabled runtime.\n",
        );
        let mut config = managed_runtime_config(&root);
        config.external_skills.enabled = false;

        let error = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &config,
        )
        .expect_err("disabled runtime should block install");

        assert!(error.contains("external skills runtime is disabled"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_and_invoke_installed_skill_return_managed_metadata() {
        let root = unique_temp_dir("loongclaw-ext-skill-list-invoke");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nPrefer explicit verification before completion.\n",
        );
        let config = managed_runtime_config(&root);

        crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &config,
        )
        .expect("install should succeed");

        let list_outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.list".to_owned(),
                payload: json!({}),
            },
            &config,
        )
        .expect("list should succeed");
        assert_eq!(list_outcome.status, "ok");
        assert_eq!(list_outcome.payload["skills"][0]["skill_id"], "demo-skill");

        let invoke_outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.invoke".to_owned(),
                payload: json!({
                    "skill_id": "demo-skill"
                }),
            },
            &config,
        )
        .expect("invoke should succeed");
        assert_eq!(invoke_outcome.status, "ok");
        assert_eq!(invoke_outcome.payload["skill_id"], "demo-skill");
        assert!(
            invoke_outcome.payload["instructions"]
                .as_str()
                .expect("instructions should be text")
                .contains("Demo Skill")
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn invoke_requires_enabled_runtime() {
        let root = unique_temp_dir("loongclaw-ext-skill-invoke-disabled");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nInvoke should require enabled runtime.\n",
        );
        let enabled_config = managed_runtime_config(&root);

        crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &enabled_config,
        )
        .expect("install should succeed");

        let mut disabled_config = enabled_config.clone();
        disabled_config.external_skills.enabled = false;
        let error = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.invoke".to_owned(),
                payload: json!({
                    "skill_id": "demo-skill"
                }),
            },
            &disabled_config,
        )
        .expect_err("disabled runtime should block invoke");

        assert!(error.contains("external skills runtime is disabled"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remove_installed_skill_clears_managed_entry() {
        let root = unique_temp_dir("loongclaw-ext-skill-remove");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nKeep output concise.\n",
        );
        let config = managed_runtime_config(&root);

        crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &config,
        )
        .expect("install should succeed");

        let remove_outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.remove".to_owned(),
                payload: json!({
                    "skill_id": "demo-skill"
                }),
            },
            &config,
        )
        .expect("remove should succeed");
        assert_eq!(remove_outcome.status, "ok");
        assert_eq!(remove_outcome.payload["removed"], json!(true));

        let list_outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.list".to_owned(),
                payload: json!({}),
            },
            &config,
        )
        .expect("list should succeed after remove");
        assert_eq!(list_outcome.payload["skills"], json!([]));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_from_tar_gz_archive_extracts_wrapped_skill_root() {
        let root = unique_temp_dir("loongclaw-ext-skill-install-archive");
        fs::create_dir_all(&root).expect("create fixture root");
        let archive_source_root = root.join("archive-src");
        write_file(
            &archive_source_root,
            "bundle/demo-skill/SKILL.md",
            "# Demo Skill\n\nArchive-installed skill.\n",
        );
        let archive_path = root.join("demo-skill.tar.gz");
        {
            let tar_gz = fs::File::create(&archive_path).expect("create archive");
            let encoder = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
            let mut tar = tar::Builder::new(encoder);
            tar.append_dir_all("bundle", archive_source_root.join("bundle"))
                .expect("append archive directory");
            tar.finish().expect("finish archive");
        }

        let config = managed_runtime_config(&root);
        let outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "demo-skill.tar.gz"
                }),
            },
            &config,
        )
        .expect("archive install should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["source_kind"], "archive");
        assert!(
            root.join("external-skills-installed")
                .join("demo-skill")
                .join("SKILL.md")
                .exists()
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn inspect_returns_preview_and_missing_skill_md_is_rejected() {
        let root = unique_temp_dir("loongclaw-ext-skill-inspect");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nInspectable skill content.\n",
        );
        let config = managed_runtime_config(&root);

        crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &config,
        )
        .expect("install should succeed");

        let inspect_outcome = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.inspect".to_owned(),
                payload: json!({
                    "skill_id": "demo-skill"
                }),
            },
            &config,
        )
        .expect("inspect should succeed");
        assert_eq!(inspect_outcome.status, "ok");
        assert!(
            inspect_outcome.payload["instructions_preview"]
                .as_str()
                .expect("preview should exist")
                .contains("Inspectable skill content")
        );

        let missing_root = unique_temp_dir("loongclaw-ext-skill-missing");
        fs::create_dir_all(&missing_root).expect("create missing fixture root");
        write_file(
            &missing_root,
            "source/not-a-skill/README.md",
            "missing skill file",
        );
        let missing_config = managed_runtime_config(&missing_root);
        let error = crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/not-a-skill"
                }),
            },
            &missing_config,
        )
        .expect_err("missing skill file should fail");
        assert!(error.contains("SKILL.md"));

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&missing_root).ok();
    }

    #[test]
    fn installed_skill_snapshot_is_hidden_when_runtime_is_disabled() {
        let root = unique_temp_dir("loongclaw-ext-skill-snapshot-disabled");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "source/demo-skill/SKILL.md",
            "# Demo Skill\n\nSnapshot should not auto-expose disabled runtime.\n",
        );
        let enabled_config = managed_runtime_config(&root);
        crate::tools::execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "source/demo-skill"
                }),
            },
            &enabled_config,
        )
        .expect("install should succeed");

        let mut disabled_config = enabled_config.clone();
        disabled_config.external_skills.enabled = false;

        let lines = installed_skill_snapshot_lines_with_config(&disabled_config)
            .expect("snapshot should succeed");
        assert!(
            lines.is_empty(),
            "disabled runtime should not expose skills"
        );

        fs::remove_dir_all(&root).ok();
    }
}
