use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flate2::read::GzDecoder;
use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tar::Archive;

const DEFAULT_DOWNLOAD_DIR_NAME: &str = "external-skills-downloads";
const DEFAULT_INSTALL_DIR_NAME: &str = "external-skills-installed";
const DEFAULT_SKILL_FILENAME: &str = "SKILL.md";
const DEFAULT_INDEX_FILENAME: &str = "index.json";
const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;
const HARD_MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;
#[cfg(test)]
const INSTALLED_SKILL_SNAPSHOT_HINT: &str = "installed managed external skill; use external_skills.inspect or external_skills.invoke for details";
const PROJECT_DISCOVERY_DIRS: [(&str, usize); 4] = [
    (".agents/skills", 0),
    (".codex/skills", 1),
    (".claude/skills", 2),
    ("skills", 3),
];
const USER_DISCOVERY_DIRS: [(&str, usize); 3] = [
    (".agents/skills", 0),
    (".codex/skills", 1),
    (".claude/skills", 2),
];

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum DiscoveredSkillScope {
    Managed,
    User,
    Project,
}

impl DiscoveredSkillScope {
    const fn precedence_rank(self) -> usize {
        match self {
            Self::Managed => 0,
            Self::User => 1,
            Self::Project => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DiscoveredSkillEntry {
    skill_id: String,
    display_name: String,
    summary: String,
    scope: DiscoveredSkillScope,
    source_kind: String,
    source_path: String,
    skill_md_path: String,
    sha256: String,
    active: bool,
    install_path: Option<String>,
    model_visibility: SkillModelVisibility,
    required_env: Vec<String>,
    required_bin: Vec<String>,
    required_paths: Vec<String>,
    invocation_policy: SkillInvocationPolicy,
    required_config: Vec<String>,
    allowed_tools: Vec<String>,
    blocked_tools: Vec<String>,
    eligibility: SkillEligibility,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct DiscoveredSkillModelView {
    skill_id: String,
    display_name: String,
    summary: String,
    scope: DiscoveredSkillScope,
    source_kind: String,
    source_path: String,
    skill_md_path: String,
    sha256: String,
    active: bool,
    install_path: Option<String>,
}

impl From<DiscoveredSkillEntry> for DiscoveredSkillModelView {
    fn from(entry: DiscoveredSkillEntry) -> Self {
        Self {
            skill_id: entry.skill_id,
            display_name: entry.display_name,
            summary: entry.summary,
            scope: entry.scope,
            source_kind: entry.source_kind,
            source_path: entry.source_path,
            skill_md_path: entry.skill_md_path,
            sha256: entry.sha256,
            active: entry.active,
            install_path: entry.install_path,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum SkillModelVisibility {
    #[default]
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum SkillInvocationPolicy {
    #[default]
    Model,
    #[serde(alias = "user", alias = "operator")]
    Manual,
    Both,
}

/// `available` and `eligible` currently move together because a skill is only
/// runnable when its local prerequisites are present. Keep both fields so
/// operator-facing output can distinguish policy from current availability later.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct SkillEligibility {
    available: bool,
    eligible: bool,
    missing_env: Vec<String>,
    missing_bin: Vec<String>,
    missing_paths: Vec<String>,
    issues: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    model_visibility: SkillModelVisibility,
    #[serde(default)]
    invocation_policy: Option<SkillInvocationPolicy>,
    #[serde(default, alias = "requires_env")]
    required_env: Vec<String>,
    #[serde(
        default,
        alias = "requires_bin",
        alias = "requires_bins",
        alias = "requires_commands"
    )]
    required_bins: Vec<String>,
    #[serde(default, alias = "requires_paths")]
    required_paths: Vec<String>,
    #[serde(default)]
    required_config: Vec<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    blocked_tools: Vec<String>,
}

#[derive(Debug, Clone)]
struct DiscoveredSkillCandidate {
    entry: DiscoveredSkillEntry,
    probe_rank: usize,
    root_rank: usize,
}

#[derive(Debug, Clone)]
struct BlockedSkillCandidate {
    skill_id: String,
    scope: DiscoveredSkillScope,
    probe_rank: usize,
    root_rank: usize,
    source_path: String,
    error: String,
}

#[derive(Debug, Clone, Default)]
struct SkillCandidateDiscovery {
    candidates: Vec<DiscoveredSkillCandidate>,
    blocked_candidates: Vec<BlockedSkillCandidate>,
}

#[derive(Debug, Clone, Default)]
struct SkillDiscoveryInventory {
    skills: Vec<DiscoveredSkillEntry>,
    shadowed_skills: Vec<DiscoveredSkillEntry>,
    blocked_skill_errors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillAudience {
    Model,
    Operator,
}

#[derive(Debug, Clone, Default)]
struct ExternalSkillsPolicyOverride {
    enabled: Option<bool>,
    require_download_approval: Option<bool>,
    allowed_domains: Option<BTreeSet<String>>,
    blocked_domains: Option<BTreeSet<String>>,
}

#[derive(Debug, Default)]
struct ScopedDirCleanup(Option<PathBuf>);

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
        .filter(|value| !value.is_empty());
    let bundled_skill_id = payload
        .get("bundled_skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let replace = payload
        .get("replace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let explicit_skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim);

    if raw_path.is_some() && bundled_skill_id.is_some() {
        return Err(
            "external_skills.install accepts either payload.path or payload.bundled_skill_id, not both"
                .to_owned(),
        );
    }
    if raw_path.is_none() && bundled_skill_id.is_none() {
        return Err(
            "external_skills.install requires payload.path or payload.bundled_skill_id".to_owned(),
        );
    }

    require_enabled_runtime_policy(config)?;

    let install_root = resolve_install_root(config);
    fs::create_dir_all(&install_root).map_err(|error| {
        format!(
            "failed to create external skills install root {}: {error}",
            install_root.display()
        )
    })?;
    let (skill_id, display_name, summary, source_kind, source_path, incoming_root, digest) =
        if let Some(bundled_skill_id) = bundled_skill_id {
            if explicit_skill_id
                .and_then(|value| (!value.is_empty()).then_some(value))
                .is_some()
            {
                return Err(
                    "external_skills.install cannot override payload.skill_id when payload.bundled_skill_id is used"
                        .to_owned(),
                );
            }
            let bundled = super::bundled_skills::bundled_external_skill(bundled_skill_id)
                .ok_or_else(|| {
                    format!(
                        "external_skills.install does not recognize bundled skill `{bundled_skill_id}`"
                    )
                })?;
            let skill_id = normalize_skill_id(bundled.skill_id)?;
            let display_name = derive_skill_display_name(bundled.instructions, bundled.skill_id);
            let summary = derive_skill_summary(bundled.instructions);
            let incoming_root = unique_managed_install_transition_path(
                &install_root,
                skill_id.as_str(),
                "incoming",
            )?;
            let mut incoming_cleanup = ScopedDirCleanup::new(Some(incoming_root.clone()));
            fs::create_dir_all(&incoming_root).map_err(|error| {
                format!(
                    "failed to create bundled external skill staging directory {}: {error}",
                    incoming_root.display()
                )
            })?;
            let installed_skill_md_path = incoming_root.join(DEFAULT_SKILL_FILENAME);
            fs::write(&installed_skill_md_path, bundled.instructions).map_err(|error| {
                format!(
                    "failed to write bundled external skill source {}: {error}",
                    installed_skill_md_path.display()
                )
            })?;
            let digest = format!("{:x}", Sha256::digest(bundled.instructions.as_bytes()));
            incoming_cleanup.disarm();
            (
                skill_id,
                display_name,
                summary,
                "bundled".to_owned(),
                bundled.source_path.to_owned(),
                incoming_root,
                digest,
            )
        } else {
            let Some(raw_path) = raw_path else {
                return Err(
                    "external_skills.install internal error: missing path after payload validation"
                        .to_owned(),
                );
            };
            let source_path = super::file::resolve_safe_file_path_with_config(raw_path, config)?;
            let source_metadata = fs::symlink_metadata(&source_path).map_err(|error| {
                format!(
                    "failed to inspect external skill source {}: {error}",
                    source_path.display()
                )
            })?;
            let source_file_type = source_metadata.file_type();
            if source_file_type.is_symlink() {
                return Err(format!(
                    "external skill source {} cannot be a symlink",
                    source_path.display()
                ));
            }

            let (skill_root, source_kind, cleanup_root) = if source_file_type.is_dir() {
                let skill_root = resolve_skill_root(&source_path)?;
                (skill_root, "directory", None)
            } else if source_file_type.is_file() {
                let (staging_root, skill_root) =
                    extract_archive_to_staging(&source_path, &install_root)?;
                (skill_root, "archive", Some(staging_root))
            } else {
                return Err(format!(
                    "external skill source {} must be a directory or a regular file",
                    source_path.display()
                ));
            };
            let _cleanup_root = ScopedDirCleanup::new(cleanup_root);
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
                .unwrap_or_else(|| {
                    derive_skill_id_from_markdown(&skill_root, skill_markdown.as_str())
                });
            let display_name =
                derive_skill_display_name(skill_markdown.as_str(), skill_id.as_str());
            let summary = derive_skill_summary(skill_markdown.as_str());
            let incoming_root = unique_managed_install_transition_path(
                &install_root,
                skill_id.as_str(),
                "incoming",
            )?;
            let mut incoming_cleanup = ScopedDirCleanup::new(Some(incoming_root.clone()));
            fs::create_dir_all(&incoming_root).map_err(|error| {
                format!(
                    "failed to create external skill destination {}: {error}",
                    incoming_root.display()
                )
            })?;
            copy_dir_recursive(&skill_root, &incoming_root)?;
            let installed_skill_md_path = incoming_root.join(DEFAULT_SKILL_FILENAME);
            let installed_skill_markdown =
                fs::read_to_string(&installed_skill_md_path).map_err(|error| {
                    format!(
                        "failed to verify installed skill {}: {error}",
                        installed_skill_md_path.display()
                    )
                })?;
            let digest = format!("{:x}", Sha256::digest(installed_skill_markdown.as_bytes()));
            incoming_cleanup.disarm();
            (
                skill_id,
                display_name,
                summary,
                source_kind.to_owned(),
                source_path.display().to_string(),
                incoming_root,
                digest,
            )
        };
    let _incoming_cleanup = ScopedDirCleanup::new(Some(incoming_root.clone()));

    let mut index = load_installed_skill_index(&install_root)?;
    let previous_index = index.clone();
    if !replace && index.skills.iter().any(|entry| entry.skill_id == skill_id) {
        return Err(format!(
            "external skill `{skill_id}` is already installed; pass payload.replace=true to replace it"
        ));
    }

    let destination_root = managed_skill_install_path(&install_root, skill_id.as_str())?;
    let installed_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    index.skills.retain(|entry| entry.skill_id != skill_id);
    index.skills.push(InstalledSkillEntry {
        skill_id: skill_id.clone(),
        display_name: display_name.clone(),
        summary: summary.clone(),
        source_kind: source_kind.clone(),
        source_path: source_path.clone(),
        install_path: destination_root.display().to_string(),
        skill_md_path: destination_root
            .join(DEFAULT_SKILL_FILENAME)
            .display()
            .to_string(),
        sha256: digest.clone(),
        installed_at_unix,
        active: true,
    });

    let backup_root = if destination_root.exists() {
        Some(unique_managed_install_transition_path(
            &install_root,
            skill_id.as_str(),
            "backup",
        )?)
    } else {
        None
    };
    let replaced = backup_root.is_some();
    if let Some(backup_root) = backup_root.as_ref() {
        fs::rename(&destination_root, backup_root).map_err(|error| {
            format!(
                "failed to stage previous installed skill {} for replacement: {error}",
                destination_root.display()
            )
        })?;
    }

    if let Err(error) = fs::rename(&incoming_root, &destination_root) {
        if let Some(backup_root) = backup_root.as_ref() {
            fs::rename(backup_root, &destination_root).ok();
        }
        return Err(format!(
            "failed to activate managed external skill install {}: {error}",
            destination_root.display()
        ));
    }

    if let Err(error) = persist_installed_skill_index(&install_root, &mut index) {
        let mut rollback_notes = vec![format!("failed to update external skills index: {error}")];

        if destination_root.exists() {
            fs::remove_dir_all(&destination_root).map_err(|remove_error| {
                format!(
                    "{}; rollback failed to remove incomplete install {}: {remove_error}",
                    rollback_notes.join(""),
                    destination_root.display()
                )
            })?;
        }

        if let Some(backup_root) = backup_root.as_ref() {
            fs::rename(backup_root, &destination_root).map_err(|restore_error| {
                format!(
                    "{}; rollback failed to restore previous install from {}: {restore_error}",
                    rollback_notes.join(""),
                    backup_root.display()
                )
            })?;
        }

        let mut rollback_index = previous_index;
        if let Err(restore_error) =
            persist_installed_skill_index(&install_root, &mut rollback_index)
        {
            rollback_notes.push(format!(
                "; rollback failed to restore previous index: {restore_error}"
            ));
        }

        return Err(rollback_notes.join(""));
    }

    if let Some(backup_root) = backup_root {
        fs::remove_dir_all(&backup_root).map_err(|error| {
            format!(
                "failed to remove replaced external skill backup {}: {error}",
                backup_root.display()
            )
        })?;
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skill_id": skill_id,
            "display_name": display_name,
            "summary": summary,
            "source_kind": source_kind,
            "source_path": source_path,
            "install_path": destination_root.display().to_string(),
            "skill_md_path": destination_root.join(DEFAULT_SKILL_FILENAME).display().to_string(),
            "sha256": digest,
            "replaced": replaced,
        }),
    })
}

pub(super) fn execute_external_skills_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    require_enabled_runtime_policy(config)?;
    execute_external_skills_list_for_audience(request.tool_name, config, SkillAudience::Model)
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

    require_enabled_runtime_policy(config)?;
    execute_external_skills_inspect_for_audience(
        request.tool_name,
        config,
        skill_id,
        SkillAudience::Model,
    )
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

    let inventory = discover_skill_inventory(config)?;
    let skill = resolve_discovered_skill(&inventory, skill_id)?;
    ensure_skill_access_for_audience(&skill, SkillAudience::Model)?;
    let instructions = load_discovered_skill_markdown(config, &skill)?;
    if !skill.eligibility.available {
        return Err(format!(
            "external skill `{skill_id}` is not eligible in the current runtime: {}",
            skill.eligibility.issues.join("; ")
        ));
    }
    if matches!(skill.invocation_policy, SkillInvocationPolicy::Manual) {
        return Err(format!(
            "external skill `{skill_id}` is marked invocation_policy=manual and cannot be invoked through external_skills.invoke"
        ));
    }
    let invocation_policy_id = invocation_policy_id(skill.invocation_policy);
    let tool_restrictions_suffix = render_tool_restrictions_suffix(
        skill.allowed_tools.as_slice(),
        skill.blocked_tools.as_slice(),
    );
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "skill_id": skill.skill_id,
            "display_name": skill.display_name,
            "summary": skill.summary,
            "scope": skill.scope,
            "source_path": skill.source_path,
            "install_path": skill.install_path,
            "skill_md_path": skill.skill_md_path,
            "instructions": instructions,
            "metadata": metadata_payload_from_skill(&skill),
            "eligibility": skill.eligibility,
            "invocation_summary": format!(
                "Loaded external skill `{}` with invocation_policy={}. Apply the instructions in `SKILL.md` before continuing the task{}.",
                skill_id,
                invocation_policy_id,
                tool_restrictions_suffix
            ),
        }),
    })
}

pub(crate) fn execute_external_skills_operator_list_tool_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_external_skills_list_for_audience(
        "external_skills.list".to_owned(),
        config,
        SkillAudience::Operator,
    )
}

pub(crate) fn execute_external_skills_operator_inspect_tool_with_config(
    skill_id: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_external_skills_inspect_for_audience(
        "external_skills.inspect".to_owned(),
        config,
        skill_id,
        SkillAudience::Operator,
    )
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

    require_enabled_runtime_policy(config)?;

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
    let install_path = PathBuf::from(entry.install_path);
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

fn execute_external_skills_list_for_audience(
    tool_name: String,
    config: &super::runtime_config::ToolRuntimeConfig,
    audience: SkillAudience,
) -> Result<ToolCoreOutcome, String> {
    let inventory = discover_skill_inventory(config)?;
    let filtered = filter_inventory_for_audience(inventory, audience);
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": tool_name,
            "skills": serialize_skill_entries_for_audience(filtered.skills, audience),
            "shadowed_skills": serialize_skill_entries_for_audience(filtered.shadowed_skills, audience),
        }),
    })
}

fn execute_external_skills_inspect_for_audience(
    tool_name: String,
    config: &super::runtime_config::ToolRuntimeConfig,
    skill_id: &str,
    audience: SkillAudience,
) -> Result<ToolCoreOutcome, String> {
    let inventory = discover_skill_inventory(config)?;
    let skill = resolve_discovered_skill(&inventory, skill_id)?;
    ensure_skill_access_for_audience(&skill, audience)?;
    let instructions = load_discovered_skill_markdown(config, &skill)?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": tool_name,
            "skill": serialize_skill_entry_for_audience(skill, audience),
            "instructions_preview": build_preview(instructions.as_str(), 240),
            "shadowed_skills": serialize_skill_entries_for_audience(
                inventory
                    .shadowed_skills
                    .into_iter()
                    .filter(|entry| entry.skill_id == skill_id)
                    .filter(|entry| skill_is_visible_to_audience(entry, audience))
                    .collect::<Vec<_>>(),
                audience,
            ),
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

impl ScopedDirCleanup {
    fn new(path: Option<PathBuf>) -> Self {
        Self(path)
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ScopedDirCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            fs::remove_dir_all(path).ok();
        }
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

pub(crate) fn normalize_domain_rule(raw: &str) -> Result<String, String> {
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

fn unique_managed_install_transition_path(
    install_root: &Path,
    skill_id: &str,
    phase: &str,
) -> Result<PathBuf, String> {
    let normalized_skill_id = normalize_skill_id(skill_id)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Ok(install_root.join(format!(".{phase}-{normalized_skill_id}-{nanos}")))
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
    if contains_regular_skill_markdown(root)? {
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

fn extract_archive_to_staging(
    archive_path: &Path,
    install_root: &Path,
) -> Result<(PathBuf, PathBuf), String> {
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
    let extraction = (|| -> Result<PathBuf, String> {
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
            let entry_type = entry.header().entry_type();
            if entry_type.is_symlink() || entry_type.is_hard_link() {
                return Err(format!(
                    "external skill archive {} cannot contain symlinks or hard links",
                    archive_path.display()
                ));
            }
            if !(entry_type.is_dir() || entry_type.is_file()) {
                return Err(format!(
                    "external skill archive {} contains unsupported entry types; only files and directories are allowed",
                    archive_path.display()
                ));
            }
            entry.unpack_in(&staging_root).map_err(|error| {
                format!(
                    "failed to extract external skill archive {}: {error}",
                    archive_path.display()
                )
            })?;
        }
        resolve_skill_root(&staging_root)
    })();

    match extraction {
        Ok(skill_root) => Ok((staging_root, skill_root)),
        Err(error) => {
            fs::remove_dir_all(&staging_root).ok();
            Err(error)
        }
    }
}

fn find_skill_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    visit_skill_roots(root, &mut roots)?;
    roots.sort();
    roots.dedup();
    Ok(roots)
}

pub(crate) fn discover_installable_skill_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    find_skill_roots(root)
}

pub(crate) fn resolve_installable_skill_id(root: &Path) -> Result<String, String> {
    let skill_markdown = load_directory_skill_markdown(root)?;
    Ok(derive_skill_id_from_markdown(root, skill_markdown.as_str()))
}

fn visit_skill_roots(root: &Path, roots: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            root.display()
        )
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(format!(
            "external skill source {} cannot contain symlinks",
            root.display()
        ));
    }
    if !file_type.is_dir() {
        if file_type.is_file() {
            return Ok(());
        }
        return Err(format!(
            "external skill source {} contains unsupported file types",
            root.display()
        ));
    }
    if contains_regular_skill_markdown(root)? {
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
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to inspect external skill source {}: {error}",
                path.display()
            )
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(format!(
                "external skill source {} cannot contain symlinks",
                path.display()
            ));
        }
        if file_type.is_dir() {
            visit_skill_roots(&path, roots)?;
        } else if !file_type.is_file() {
            return Err(format!(
                "external skill source {} contains unsupported file types",
                path.display()
            ));
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

fn derive_skill_id_from_markdown(root: &Path, skill_markdown: &str) -> String {
    parse_skill_frontmatter(skill_markdown)
        .ok()
        .unwrap_or_default()
        .name
        .as_deref()
        .and_then(|name| normalize_skill_id(name).ok())
        .unwrap_or_else(|| derive_skill_id(root))
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
    let frontmatter = parse_skill_frontmatter(skill_markdown)
        .ok()
        .unwrap_or_default();
    derive_skill_display_name_with_frontmatter(skill_markdown, &frontmatter, fallback)
}

fn derive_skill_display_name_with_frontmatter(
    skill_markdown: &str,
    frontmatter: &SkillFrontmatter,
    fallback: &str,
) -> String {
    // Prefer the visible document title when present so operator-facing listings match
    // the heading the skill author chose to present in SKILL.md.
    for line in skill_content_lines(skill_markdown) {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return title.to_owned();
            }
        }
    }
    if let Some(name) = frontmatter.name.as_deref()
        && !name.is_empty()
    {
        return name.to_owned();
    }
    fallback.to_owned()
}

fn derive_skill_summary(skill_markdown: &str) -> String {
    let frontmatter = parse_skill_frontmatter(skill_markdown)
        .ok()
        .unwrap_or_default();
    derive_skill_summary_with_frontmatter(skill_markdown, &frontmatter)
}

fn derive_skill_summary_with_frontmatter(
    skill_markdown: &str,
    frontmatter: &SkillFrontmatter,
) -> String {
    if let Some(description) = frontmatter.description.as_deref()
        && !description.is_empty()
    {
        return build_preview(description, 120);
    }
    for line in skill_content_lines(skill_markdown) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return build_preview(trimmed, 120);
    }
    "No summary provided.".to_owned()
}

fn parse_skill_frontmatter(skill_markdown: &str) -> Result<SkillFrontmatter, String> {
    let mut lines = skill_markdown.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Ok(SkillFrontmatter::default());
    }

    let mut raw_frontmatter = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            let raw = raw_frontmatter.join(
                "
",
            );
            if raw.trim().is_empty() {
                return Ok(SkillFrontmatter::default());
            }
            let parsed = serde_yaml::from_str::<YamlValue>(&raw)
                .map_err(|error| format!("failed to parse YAML: {error}"))?;
            let mut frontmatter = match parsed {
                YamlValue::Null => SkillFrontmatter::default(),
                YamlValue::Mapping(_) => serde_yaml::from_value(parsed).map_err(|error| {
                    format!("failed to decode supported metadata fields: {error}")
                })?,
                YamlValue::Bool(_)
                | YamlValue::Number(_)
                | YamlValue::String(_)
                | YamlValue::Sequence(_)
                | YamlValue::Tagged(_) => {
                    return Err(
                        "frontmatter must decode to a YAML mapping of scalar or list fields"
                            .to_owned(),
                    );
                }
            };
            normalize_skill_frontmatter(&mut frontmatter);
            return Ok(frontmatter);
        }
        raw_frontmatter.push(line);
    }
    Err("frontmatter is missing a closing `---` delimiter".to_owned())
}

fn normalize_skill_frontmatter(frontmatter: &mut SkillFrontmatter) {
    frontmatter.name = normalize_optional_metadata_string(frontmatter.name.take());
    frontmatter.description = normalize_optional_metadata_string(frontmatter.description.take());
    frontmatter.required_env =
        normalize_metadata_string_list(std::mem::take(&mut frontmatter.required_env));
    frontmatter.required_bins =
        normalize_metadata_string_list(std::mem::take(&mut frontmatter.required_bins));
    frontmatter.required_paths =
        normalize_metadata_string_list(std::mem::take(&mut frontmatter.required_paths));
    frontmatter.required_config =
        normalize_metadata_string_list(std::mem::take(&mut frontmatter.required_config));
    frontmatter.allowed_tools =
        normalize_metadata_string_list(std::mem::take(&mut frontmatter.allowed_tools));
    frontmatter.blocked_tools =
        normalize_metadata_string_list(std::mem::take(&mut frontmatter.blocked_tools));
}

fn normalize_optional_metadata_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_metadata_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn skill_content_lines(skill_markdown: &str) -> impl Iterator<Item = &str> {
    let mut in_frontmatter = false;
    let mut frontmatter_started = false;
    skill_markdown.lines().filter(move |line| {
        let trimmed = line.trim();
        if !frontmatter_started && trimmed == "---" {
            frontmatter_started = true;
            in_frontmatter = true;
            return false;
        }
        if in_frontmatter {
            if trimmed == "---" {
                in_frontmatter = false;
            }
            return false;
        }
        true
    })
}

fn build_managed_discovered_skill_entry(
    config: &super::runtime_config::ToolRuntimeConfig,
    entry: InstalledSkillEntry,
) -> Result<DiscoveredSkillEntry, String> {
    let skill_markdown = load_managed_skill_markdown(&entry)?;
    build_discovered_skill_entry(
        config,
        DiscoveredSkillScope::Managed,
        entry.source_kind,
        entry.source_path,
        entry.skill_md_path,
        entry.skill_id,
        skill_markdown.as_str(),
        entry.active,
        Some(entry.install_path),
    )
}

fn build_discovered_skill_entry(
    config: &super::runtime_config::ToolRuntimeConfig,
    scope: DiscoveredSkillScope,
    source_kind: String,
    source_path: String,
    skill_md_path: String,
    skill_id: String,
    skill_markdown: &str,
    active: bool,
    install_path: Option<String>,
) -> Result<DiscoveredSkillEntry, String> {
    let frontmatter = parse_skill_frontmatter(skill_markdown).map_err(|error| {
        format!(
            "invalid external skill frontmatter in {}: {error}",
            skill_md_path
        )
    })?;
    let invocation_policy = frontmatter
        .invocation_policy
        .unwrap_or(SkillInvocationPolicy::Model);
    let eligibility = evaluate_skill_eligibility(config, &frontmatter);
    Ok(DiscoveredSkillEntry {
        display_name: derive_skill_display_name_with_frontmatter(
            skill_markdown,
            &frontmatter,
            skill_id.as_str(),
        ),
        summary: derive_skill_summary_with_frontmatter(skill_markdown, &frontmatter),
        scope,
        source_kind,
        source_path,
        skill_md_path,
        sha256: format!("{:x}", Sha256::digest(skill_markdown.as_bytes())),
        active,
        install_path,
        model_visibility: frontmatter.model_visibility,
        required_env: frontmatter.required_env.clone(),
        required_bin: frontmatter.required_bins.clone(),
        required_paths: frontmatter.required_paths.clone(),
        invocation_policy,
        required_config: frontmatter.required_config.clone(),
        allowed_tools: frontmatter.allowed_tools.clone(),
        blocked_tools: frontmatter.blocked_tools.clone(),
        eligibility,
        skill_id,
    })
}

// This currently answers both "can run right now" and "eligible to run" so
// operator output stays explicit without silently inventing separate semantics.
fn evaluate_skill_eligibility(
    config: &super::runtime_config::ToolRuntimeConfig,
    frontmatter: &SkillFrontmatter,
) -> SkillEligibility {
    let missing_env = frontmatter
        .required_env
        .iter()
        .filter(|name| !env_var_is_present(name))
        .cloned()
        .collect::<Vec<_>>();
    let missing_bin = frontmatter
        .required_bins
        .iter()
        .filter(|command| !command_exists(command))
        .cloned()
        .collect::<Vec<_>>();
    let missing_paths = frontmatter
        .required_paths
        .iter()
        .filter(|path| !required_path_exists(config, path))
        .cloned()
        .collect::<Vec<_>>();

    let mut issues = missing_env
        .iter()
        .map(|env_name| format!("missing env `{env_name}`"))
        .collect::<Vec<_>>();
    issues.extend(
        missing_bin
            .iter()
            .map(|binary| format!("missing binary `{binary}`")),
    );
    issues.extend(
        missing_paths
            .iter()
            .map(|path| format!("missing path `{path}`")),
    );
    for selector in &frontmatter.required_config {
        match runtime_config_selector_enabled(config, selector) {
            Some(true) => {}
            Some(false) => issues.push(format!("config gate `{selector}` is disabled")),
            None => issues.push(format!("unsupported config gate `{selector}`")),
        }
    }
    let available = issues.is_empty();
    SkillEligibility {
        available,
        eligible: available,
        missing_env,
        missing_bin,
        missing_paths,
        issues,
    }
}

fn env_var_is_present(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn command_exists(binary: &str) -> bool {
    let candidate = binary.trim();
    if candidate.is_empty() {
        return false;
    }
    which::which(candidate).is_ok()
}

fn serialize_skill_entry_for_audience(
    entry: DiscoveredSkillEntry,
    audience: SkillAudience,
) -> Value {
    match audience {
        SkillAudience::Operator => json!(entry),
        SkillAudience::Model => json!(DiscoveredSkillModelView::from(entry)),
    }
}

fn serialize_skill_entries_for_audience(
    entries: Vec<DiscoveredSkillEntry>,
    audience: SkillAudience,
) -> Value {
    match audience {
        SkillAudience::Operator => json!(entries),
        SkillAudience::Model => json!(
            entries
                .into_iter()
                .map(DiscoveredSkillModelView::from)
                .collect::<Vec<_>>()
        ),
    }
}

fn compare_candidate_priority(
    left_scope: DiscoveredSkillScope,
    left_probe_rank: usize,
    left_root_rank: usize,
    left_source_path: &str,
    right_scope: DiscoveredSkillScope,
    right_probe_rank: usize,
    right_root_rank: usize,
    right_source_path: &str,
) -> Ordering {
    left_scope
        .precedence_rank()
        .cmp(&right_scope.precedence_rank())
        .then_with(|| left_probe_rank.cmp(&right_probe_rank))
        .then_with(|| left_root_rank.cmp(&right_root_rank))
        .then_with(|| left_source_path.cmp(right_source_path))
}

fn compare_discovered_skill_candidates(
    left: &DiscoveredSkillCandidate,
    right: &DiscoveredSkillCandidate,
) -> Ordering {
    compare_candidate_priority(
        left.entry.scope,
        left.probe_rank,
        left.root_rank,
        &left.entry.source_path,
        right.entry.scope,
        right.probe_rank,
        right.root_rank,
        &right.entry.source_path,
    )
}

fn compare_blocked_skill_candidates(
    left: &BlockedSkillCandidate,
    right: &BlockedSkillCandidate,
) -> Ordering {
    compare_candidate_priority(
        left.scope,
        left.probe_rank,
        left.root_rank,
        &left.source_path,
        right.scope,
        right.probe_rank,
        right.root_rank,
        &right.source_path,
    )
}

fn blocked_candidate_precedes_discovered(
    blocked: &BlockedSkillCandidate,
    candidate: &DiscoveredSkillCandidate,
) -> bool {
    compare_candidate_priority(
        blocked.scope,
        blocked.probe_rank,
        blocked.root_rank,
        &blocked.source_path,
        candidate.entry.scope,
        candidate.probe_rank,
        candidate.root_rank,
        &candidate.entry.source_path,
    ) != Ordering::Greater
}

fn required_path_exists(config: &super::runtime_config::ToolRuntimeConfig, raw: &str) -> bool {
    resolve_required_path(config, raw).exists()
}

fn resolve_required_path(config: &super::runtime_config::ToolRuntimeConfig, raw: &str) -> PathBuf {
    let candidate = PathBuf::from(raw);
    if candidate.is_absolute() {
        return candidate;
    }
    config
        .file_root
        .clone()
        .or_else(|| project_discovery_root(config))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(candidate)
}

fn filter_inventory_for_audience(
    inventory: SkillDiscoveryInventory,
    audience: SkillAudience,
) -> SkillDiscoveryInventory {
    match audience {
        SkillAudience::Operator => inventory,
        SkillAudience::Model => SkillDiscoveryInventory {
            skills: inventory
                .skills
                .into_iter()
                .filter(|entry| skill_is_visible_to_audience(entry, audience))
                .collect(),
            shadowed_skills: inventory
                .shadowed_skills
                .into_iter()
                .filter(|entry| skill_is_visible_to_audience(entry, audience))
                .collect(),
            blocked_skill_errors: inventory.blocked_skill_errors,
        },
    }
}

fn skill_is_visible_to_audience(entry: &DiscoveredSkillEntry, audience: SkillAudience) -> bool {
    match audience {
        SkillAudience::Operator => true,
        SkillAudience::Model => {
            entry.active
                && entry.model_visibility == SkillModelVisibility::Visible
                && entry.eligibility.available
        }
    }
}

fn ensure_skill_access_for_audience(
    skill: &DiscoveredSkillEntry,
    audience: SkillAudience,
) -> Result<(), String> {
    if audience == SkillAudience::Operator {
        return Ok(());
    }
    if skill_is_visible_to_audience(skill, audience) {
        return Ok(());
    }

    let mut blockers = Vec::new();
    if !skill.active {
        blockers.push("a higher-precedence resolved skill is inactive".to_owned());
    }
    if skill.model_visibility == SkillModelVisibility::Hidden {
        blockers.push("the skill is operator-only and hidden from the model surface".to_owned());
    }
    if !skill.eligibility.missing_env.is_empty() {
        blockers.push(format!(
            "missing env vars: {}",
            skill.eligibility.missing_env.join(", ")
        ));
    }
    if !skill.eligibility.missing_bin.is_empty() {
        blockers.push(format!(
            "missing commands on PATH: {}",
            skill.eligibility.missing_bin.join(", ")
        ));
    }
    if !skill.eligibility.missing_paths.is_empty() {
        blockers.push(format!(
            "missing required paths: {}",
            skill.eligibility.missing_paths.join(", ")
        ));
    }

    Err(format!(
        "external skill `{}` is not available on the provider surface: {}",
        skill.skill_id,
        blockers.join("; ")
    ))
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
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            source.display()
        )
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(format!(
            "external skill source {} cannot contain symlinks",
            source.display()
        ));
    }
    if !file_type.is_dir() {
        return Err(format!(
            "external skill source {} must be a directory during install copy",
            source.display()
        ));
    }
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
        let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
            format!(
                "failed to inspect external skill source {}: {error}",
                source_path.display()
            )
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(format!(
                "external skill source {} cannot contain symlinks",
                source_path.display()
            ));
        }
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "failed to copy external skill file {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(format!(
                "external skill source {} contains unsupported file types",
                source_path.display()
            ));
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
    index.skills = index
        .skills
        .into_iter()
        .map(|entry| normalize_loaded_skill_entry(root, entry))
        .collect::<Result<Vec<_>, _>>()?;
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

fn discover_skill_inventory(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillDiscoveryInventory, String> {
    let managed = discover_managed_skill_candidates(config)?;
    let user = discover_user_skill_candidates(config)?;
    let project = discover_project_skill_candidates(config)?;

    let mut grouped = BTreeMap::<String, Vec<DiscoveredSkillCandidate>>::new();
    for candidate in managed
        .candidates
        .into_iter()
        .chain(user.candidates)
        .chain(project.candidates)
    {
        grouped
            .entry(candidate.entry.skill_id.clone())
            .or_default()
            .push(candidate);
    }

    let mut blocked_grouped = BTreeMap::<String, Vec<BlockedSkillCandidate>>::new();
    for blocked in managed
        .blocked_candidates
        .into_iter()
        .chain(user.blocked_candidates)
        .chain(project.blocked_candidates)
    {
        blocked_grouped
            .entry(blocked.skill_id.clone())
            .or_default()
            .push(blocked);
    }

    let mut inventory = SkillDiscoveryInventory::default();
    let skill_ids = grouped
        .keys()
        .chain(blocked_grouped.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for skill_id in skill_ids {
        let mut candidates = grouped.remove(&skill_id).unwrap_or_default();
        let mut blocked_candidates = blocked_grouped.remove(&skill_id).unwrap_or_default();
        candidates.sort_by(compare_discovered_skill_candidates);
        blocked_candidates.sort_by(compare_blocked_skill_candidates);

        if let Some(blocked) = blocked_candidates.first()
            && candidates
                .first()
                .is_none_or(|candidate| blocked_candidate_precedes_discovered(blocked, candidate))
        {
            inventory
                .blocked_skill_errors
                .insert(skill_id, blocked.error.clone());
            continue;
        }

        if candidates.is_empty() {
            continue;
        }

        let winner = candidates.remove(0);
        inventory.skills.push(winner.entry);
        inventory
            .shadowed_skills
            .extend(candidates.into_iter().map(|candidate| candidate.entry));
    }

    inventory
        .skills
        .sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    inventory.shadowed_skills.sort_by(|left, right| {
        left.skill_id
            .cmp(&right.skill_id)
            .then_with(|| {
                left.scope
                    .precedence_rank()
                    .cmp(&right.scope.precedence_rank())
            })
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    Ok(inventory)
}

fn metadata_payload_from_skill(skill: &DiscoveredSkillEntry) -> Value {
    json!({
        "model_visibility": skill.model_visibility,
        "invocation_policy": skill.invocation_policy,
        "required_env": skill.required_env,
        "required_bins": skill.required_bin,
        "required_paths": skill.required_paths,
        "required_config": skill.required_config,
        "allowed_tools": skill.allowed_tools,
        "blocked_tools": skill.blocked_tools,
    })
}

fn runtime_config_selector_enabled(
    config: &super::runtime_config::ToolRuntimeConfig,
    selector: &str,
) -> Option<bool> {
    match selector.trim().to_ascii_lowercase().as_str() {
        "external_skills.enabled" | "tools.external_skills.enabled" => {
            Some(config.external_skills.enabled)
        }
        "browser.enabled" | "tools.browser.enabled" => Some(config.browser.enabled),
        "browser_companion.enabled" | "tools.browser_companion.enabled" => {
            Some(config.browser_companion.enabled)
        }
        "delegate.enabled" | "tools.delegate.enabled" => Some(config.delegate_enabled),
        "messages.enabled" | "tools.messages.enabled" => Some(config.messages_enabled),
        "sessions.enabled" | "tools.sessions.enabled" => Some(config.sessions_enabled),
        "web.enabled" | "tools.web.enabled" | "web_fetch.enabled" | "tools.web_fetch.enabled" => {
            Some(config.web_fetch.enabled)
        }
        "web_search.enabled" | "tools.web_search.enabled" => Some(config.web_search.enabled),
        _ => None,
    }
}

fn invocation_policy_id(policy: SkillInvocationPolicy) -> &'static str {
    match policy {
        SkillInvocationPolicy::Model => "model",
        SkillInvocationPolicy::Manual => "manual",
        SkillInvocationPolicy::Both => "both",
    }
}

fn render_tool_restrictions_suffix(allowed_tools: &[String], blocked_tools: &[String]) -> String {
    let mut fragments = Vec::new();
    if !allowed_tools.is_empty() {
        fragments.push(format!(" allowed_tools={}", allowed_tools.join(",")));
    }
    if !blocked_tools.is_empty() {
        fragments.push(format!(" blocked_tools={}", blocked_tools.join(",")));
    }
    fragments.concat()
}

#[cfg(test)]
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
        .filter_map(|entry| {
            if !entry.active {
                return None;
            }
            let rehydrated = rehydrate_installed_skill_entry(&install_root, entry).ok()?;
            let discovered = build_managed_discovered_skill_entry(config, rehydrated).ok()?;
            skill_is_visible_to_audience(&discovered, SkillAudience::Model).then(|| {
                format!(
                    "- {}: {}",
                    discovered.skill_id, INSTALLED_SKILL_SNAPSHOT_HINT
                )
            })
        })
        .collect())
}

fn discover_managed_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillCandidateDiscovery, String> {
    let install_root = resolve_install_root(config);
    let index = load_installed_skill_index(&install_root)?;
    let mut discovery = SkillCandidateDiscovery::default();
    for entry in index.skills {
        let skill_id = entry.skill_id.clone();
        let source_path = entry.source_path.clone();
        let entry = match rehydrate_installed_skill_entry(&install_root, entry) {
            Ok(entry) => entry,
            Err(error) => {
                discovery.blocked_candidates.push(BlockedSkillCandidate {
                    skill_id,
                    scope: DiscoveredSkillScope::Managed,
                    probe_rank: 0,
                    root_rank: 0,
                    source_path,
                    error,
                });
                continue;
            }
        };
        let entry = match build_managed_discovered_skill_entry(config, entry) {
            Ok(entry) => entry,
            Err(error) => {
                discovery.blocked_candidates.push(BlockedSkillCandidate {
                    skill_id,
                    scope: DiscoveredSkillScope::Managed,
                    probe_rank: 0,
                    root_rank: 0,
                    source_path,
                    error,
                });
                continue;
            }
        };
        discovery.candidates.push(DiscoveredSkillCandidate {
            probe_rank: 0,
            root_rank: 0,
            entry,
        });
    }
    Ok(discovery)
}

fn discover_user_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillCandidateDiscovery, String> {
    let Some(home_root) = user_home_dir() else {
        return Ok(SkillCandidateDiscovery::default());
    };
    discover_scoped_skill_candidates(
        config,
        &[home_root],
        DiscoveredSkillScope::User,
        &USER_DISCOVERY_DIRS,
    )
}

fn discover_project_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillCandidateDiscovery, String> {
    discover_scoped_skill_candidates(
        config,
        &project_discovery_probe_roots(config),
        DiscoveredSkillScope::Project,
        &PROJECT_DISCOVERY_DIRS,
    )
}

fn discover_scoped_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
    probe_roots: &[PathBuf],
    scope: DiscoveredSkillScope,
    dir_specs: &[(&str, usize)],
) -> Result<SkillCandidateDiscovery, String> {
    let mut discovery = SkillCandidateDiscovery::default();
    let mut seen = BTreeSet::new();
    for (probe_rank, probe_root) in probe_roots.iter().enumerate() {
        for (relative_dir, root_rank) in dir_specs {
            let container = probe_root.join(relative_dir);
            if !container.is_dir() {
                continue;
            }
            for skill_root in find_discoverable_skill_roots(&container) {
                let skill_md_path = skill_root.join(DEFAULT_SKILL_FILENAME);
                let key = skill_md_path.display().to_string();
                if !seen.insert(key.clone()) {
                    continue;
                }
                let skill_markdown = match load_directory_skill_markdown(&skill_root) {
                    Ok(skill_markdown) => skill_markdown,
                    Err(error) => {
                        discovery.blocked_candidates.push(BlockedSkillCandidate {
                            skill_id: derive_skill_id(&skill_root),
                            scope,
                            probe_rank,
                            root_rank: *root_rank,
                            source_path: skill_root.display().to_string(),
                            error,
                        });
                        continue;
                    }
                };
                let skill_id = derive_skill_id_from_markdown(&skill_root, skill_markdown.as_str());
                let entry = match build_discovered_skill_entry(
                    config,
                    scope,
                    "directory".to_owned(),
                    skill_root.display().to_string(),
                    key,
                    skill_id.clone(),
                    skill_markdown.as_str(),
                    true,
                    None,
                ) {
                    Ok(entry) => entry,
                    Err(error) => {
                        discovery.blocked_candidates.push(BlockedSkillCandidate {
                            skill_id,
                            scope,
                            probe_rank,
                            root_rank: *root_rank,
                            source_path: skill_root.display().to_string(),
                            error,
                        });
                        continue;
                    }
                };
                discovery.candidates.push(DiscoveredSkillCandidate {
                    probe_rank,
                    root_rank: *root_rank,
                    entry,
                });
            }
        }
    }
    Ok(discovery)
}

fn project_discovery_probe_roots(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Vec<PathBuf> {
    let Some(project_root) = project_discovery_root(config) else {
        return Vec::new();
    };
    let project_root = dunce::canonicalize(&project_root).unwrap_or(project_root);

    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        let current_dir = dunce::canonicalize(&current_dir).unwrap_or(current_dir);
        if current_dir.starts_with(&project_root) {
            let mut next = Some(current_dir.as_path());
            while let Some(path) = next {
                roots.push(path.to_path_buf());
                if path == project_root.as_path() {
                    break;
                }
                next = path.parent();
            }
        } else {
            roots.push(project_root);
        }
    } else {
        roots.push(project_root);
    }

    let mut seen = BTreeSet::new();
    roots.retain(|root| seen.insert(root.display().to_string()));
    roots
}

fn project_discovery_root(config: &super::runtime_config::ToolRuntimeConfig) -> Option<PathBuf> {
    config
        .config_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| config.file_root.clone())
        .or_else(|| std::env::current_dir().ok())
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn resolve_discovered_skill(
    inventory: &SkillDiscoveryInventory,
    skill_id: &str,
) -> Result<DiscoveredSkillEntry, String> {
    if let Some(skill) = inventory
        .skills
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned()
    {
        return Ok(skill);
    }
    if let Some(error) = inventory.blocked_skill_errors.get(skill_id) {
        return Err(error.clone());
    }
    Err(format!("external skill `{skill_id}` is not available"))
}

fn load_discovered_skill_markdown(
    config: &super::runtime_config::ToolRuntimeConfig,
    skill: &DiscoveredSkillEntry,
) -> Result<String, String> {
    match skill.scope {
        DiscoveredSkillScope::Managed => {
            let install_root = resolve_install_root(config);
            let (_entry, instructions) =
                load_installed_skill_material(&install_root, skill.skill_id.as_str())?;
            Ok(instructions)
        }
        DiscoveredSkillScope::User | DiscoveredSkillScope::Project => {
            load_directory_skill_markdown(Path::new(&skill.source_path))
        }
    }
}

fn load_directory_skill_markdown(skill_root: &Path) -> Result<String, String> {
    let metadata = fs::metadata(skill_root).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            skill_root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "external skill source {} must be a directory",
            skill_root.display()
        ));
    }
    let skill_md_path = skill_root.join(DEFAULT_SKILL_FILENAME);
    if !skill_md_path.is_file() {
        return Err(format!(
            "external skill source {} is missing `{DEFAULT_SKILL_FILENAME}`",
            skill_root.display()
        ));
    }
    let skill_md_metadata = fs::metadata(&skill_md_path).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            skill_md_path.display()
        )
    })?;
    if skill_md_metadata.len() > DEFAULT_MAX_DOWNLOAD_BYTES as u64 {
        return Err(format!(
            "external skill source {} exceeds the {} byte size limit",
            skill_md_path.display(),
            DEFAULT_MAX_DOWNLOAD_BYTES
        ));
    }
    fs::read_to_string(&skill_md_path).map_err(|error| {
        format!(
            "failed to read external skill source {}: {error}",
            skill_md_path.display()
        )
    })
}

fn find_discoverable_skill_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut visited = BTreeSet::new();
    visit_discoverable_skill_roots(root, &mut roots, &mut visited);
    roots.sort();
    roots.dedup();
    roots
}

fn visit_discoverable_skill_roots(
    root: &Path,
    roots: &mut Vec<PathBuf>,
    visited: &mut BTreeSet<String>,
) {
    let canonical = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let key = canonical.display().to_string();
    if !visited.insert(key) {
        return;
    }

    let Ok(metadata) = fs::metadata(&canonical) else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }

    let skill_md_path = canonical.join(DEFAULT_SKILL_FILENAME);
    if skill_md_path.is_file() {
        roots.push(canonical);
        return;
    }

    let Ok(entries) = fs::read_dir(&canonical) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        visit_discoverable_skill_roots(&entry.path(), roots, visited);
    }
}

fn contains_regular_skill_markdown(root: &Path) -> Result<bool, String> {
    let skill_md_path = root.join(DEFAULT_SKILL_FILENAME);
    let metadata = match fs::symlink_metadata(&skill_md_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed to inspect external skill source {}: {error}",
                skill_md_path.display()
            ));
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(format!(
            "external skill source {} cannot use a symlinked `{DEFAULT_SKILL_FILENAME}`",
            root.display()
        ));
    }
    if !file_type.is_file() {
        return Err(format!(
            "external skill source {} must contain a regular `{DEFAULT_SKILL_FILENAME}` file",
            root.display()
        ));
    }
    Ok(true)
}

fn normalize_loaded_skill_entry(
    install_root: &Path,
    mut entry: InstalledSkillEntry,
) -> Result<InstalledSkillEntry, String> {
    let normalized_skill_id = normalize_skill_id(entry.skill_id.as_str())?;
    if normalized_skill_id != entry.skill_id {
        return Err(format!(
            "external skills index contains non-normalized skill id `{}`",
            entry.skill_id
        ));
    }
    let install_path = managed_skill_install_path(install_root, entry.skill_id.as_str())?;
    let skill_md_path = install_path.join(DEFAULT_SKILL_FILENAME);
    entry.install_path = install_path.display().to_string();
    entry.skill_md_path = skill_md_path.display().to_string();
    Ok(entry)
}

fn load_installed_skill_material(
    install_root: &Path,
    skill_id: &str,
) -> Result<(InstalledSkillEntry, String), String> {
    let entry = installed_skill_by_id(&load_installed_skill_index(install_root)?, skill_id)?;
    let entry = rehydrate_installed_skill_entry(install_root, entry)?;
    let instructions = load_managed_skill_markdown(&entry)?;
    Ok((entry, instructions))
}

fn rehydrate_installed_skill_entry(
    install_root: &Path,
    mut entry: InstalledSkillEntry,
) -> Result<InstalledSkillEntry, String> {
    let install_path = managed_skill_install_path(install_root, entry.skill_id.as_str())?;
    let install_metadata = fs::symlink_metadata(&install_path).map_err(|error| {
        format!(
            "failed to inspect managed external skill install {}: {error}",
            install_path.display()
        )
    })?;
    let install_file_type = install_metadata.file_type();
    if install_file_type.is_symlink() {
        return Err(format!(
            "managed external skill install {} cannot be a symlink",
            install_path.display()
        ));
    }
    if !install_file_type.is_dir() {
        return Err(format!(
            "managed external skill install {} must be a directory",
            install_path.display()
        ));
    }

    entry.install_path = install_path.display().to_string();
    entry.skill_md_path = install_path
        .join(DEFAULT_SKILL_FILENAME)
        .display()
        .to_string();

    let skill_markdown = load_managed_skill_markdown(&entry)?;
    entry.display_name =
        derive_skill_display_name(skill_markdown.as_str(), entry.skill_id.as_str());
    entry.summary = derive_skill_summary(skill_markdown.as_str());
    entry.sha256 = format!("{:x}", Sha256::digest(skill_markdown.as_bytes()));
    Ok(entry)
}

fn load_managed_skill_markdown(entry: &InstalledSkillEntry) -> Result<String, String> {
    let install_path = PathBuf::from(entry.install_path.as_str());
    if !contains_regular_skill_markdown(&install_path)? {
        return Err(format!(
            "managed external skill install {} is missing `{DEFAULT_SKILL_FILENAME}`",
            install_path.display()
        ));
    }
    fs::read_to_string(&entry.skill_md_path).map_err(|error| {
        format!(
            "failed to read installed skill {}: {error}",
            entry.skill_md_path
        )
    })
}

fn managed_skill_install_path(install_root: &Path, skill_id: &str) -> Result<PathBuf, String> {
    let normalized_skill_id = normalize_skill_id(skill_id)?;
    if normalized_skill_id != skill_id {
        return Err(format!(
            "external skill id `{skill_id}` must be normalized before path resolution"
        ));
    }
    Ok(install_root.join(skill_id))
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;
    use crate::tools::runtime_config::{ExternalSkillsRuntimePolicy, ToolRuntimeConfig};

    static POLICY_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_policy_test_lock<T>(f: impl FnOnce() -> T) -> T {
        let lock = POLICY_TEST_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f()
    }

    struct PolicyOverrideResetGuard;

    impl Drop for PolicyOverrideResetGuard {
        fn drop(&mut self) {
            reset_policy_override_for_test();
        }
    }

    fn reset_policy_override_for_test() {
        if let Some(store) = EXTERNAL_SKILLS_POLICY_OVERRIDE.get()
            && let Ok(mut guard) = store.write()
        {
            *guard = ExternalSkillsPolicyOverride::default();
        }
    }

    fn with_managed_runtime_test<T>(f: impl FnOnce() -> T) -> T {
        with_policy_test_lock(|| {
            reset_policy_override_for_test();
            let _reset_guard = PolicyOverrideResetGuard;
            f()
        })
    }

    fn base_runtime_config() -> ToolRuntimeConfig {
        ToolRuntimeConfig {
            file_root: Some(std::env::temp_dir().join("loongclaw-ext-skills-tests")),
            config_path: None,
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: false,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: true,
            },
            ..ToolRuntimeConfig::default()
        }
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    struct ScopedHomeFixture {
        _env: crate::test_support::ScopedEnv,
        path: PathBuf,
    }

    impl ScopedHomeFixture {
        fn new(prefix: &str) -> Self {
            let path = unique_temp_dir(prefix);
            fs::create_dir_all(&path).expect("create isolated home");
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &path);
            Self { _env: env, path }
        }

        fn set_env(&mut self, key: &'static str, value: impl AsRef<std::ffi::OsStr>) {
            self._env.set(key, value);
        }
    }

    impl Drop for ScopedHomeFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).ok();
        }
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
            file_root: Some(root.to_path_buf()),
            config_path: None,
            external_skills: ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: true,
            },
            ..ToolRuntimeConfig::default()
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
    fn policy_test_lock_recovers_after_mutex_poison() {
        let panic_result = std::thread::spawn(|| {
            with_policy_test_lock(|| {
                panic!("poison policy lock for test");
            });
        })
        .join();

        assert!(panic_result.is_err(), "setup thread should poison the lock");

        let recovered = std::panic::catch_unwind(|| with_policy_test_lock(|| ()));
        assert!(
            recovered.is_ok(),
            "with_policy_test_lock should recover from a poisoned mutex"
        );
    }

    #[test]
    fn install_from_directory_writes_managed_index_and_copy() {
        with_managed_runtime_test(|| {
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
            assert_eq!(outcome.payload["display_name"], "Demo Skill");
            assert_eq!(outcome.payload["replaced"], false);
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
        });
    }

    #[test]
    fn install_from_bundled_skill_id_writes_managed_index_and_copy() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-install-bundled");
            fs::create_dir_all(&root).expect("create fixture root");
            let config = managed_runtime_config(&root);

            let outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "bundled_skill_id": "browser-companion-preview"
                    }),
                },
                &config,
            )
            .expect("bundled install should succeed");

            assert_eq!(outcome.status, "ok");
            assert_eq!(outcome.payload["skill_id"], "browser-companion-preview");
            assert_eq!(outcome.payload["source_kind"], "bundled");
            assert_eq!(
                outcome.payload["source_path"],
                "bundled://browser-companion-preview"
            );
            let installed_skill = root
                .join("external-skills-installed")
                .join("browser-companion-preview")
                .join("SKILL.md");
            assert!(
                installed_skill.exists(),
                "bundled managed skill should exist"
            );
            let installed_skill_body =
                fs::read_to_string(&installed_skill).expect("read bundled managed skill");
            assert!(
                installed_skill_body.contains("agent-browser"),
                "bundled preview instructions should preserve the packaged browser companion guidance"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn install_rejects_path_and_bundled_skill_id_together() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-install-bundled-conflict");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Demo Skill\n\nConflicting payload should fail.\n",
            );
            let config = managed_runtime_config(&root);

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/demo-skill",
                        "bundled_skill_id": "browser-companion-preview"
                    }),
                },
                &config,
            )
            .expect_err("mixed path and bundled skill id should fail");

            assert!(error.contains("either payload.path or payload.bundled_skill_id"));

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn install_replace_reports_actual_replacement_state() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-install-replace");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Demo Skill\n\nFirst install.\n",
            );
            write_file(
                &root,
                "source/demo-skill-v2/SKILL.md",
                "# Demo Skill\n\nReplacement install.\n",
            );
            let config = managed_runtime_config(&root);

            let first_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/demo-skill",
                        "replace": true
                    }),
                },
                &config,
            )
            .expect("first install with replace flag should succeed");
            assert_eq!(first_outcome.payload["display_name"], "Demo Skill");
            assert_eq!(first_outcome.payload["replaced"], false);

            let replace_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/demo-skill-v2",
                        "skill_id": "demo-skill",
                        "replace": true
                    }),
                },
                &config,
            )
            .expect("second install should report a real replacement");
            assert_eq!(replace_outcome.payload["display_name"], "Demo Skill");
            assert_eq!(replace_outcome.payload["replaced"], true);

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn install_requires_enabled_runtime() {
        with_managed_runtime_test(|| {
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
        });
    }

    #[test]
    fn list_inspect_and_remove_require_enabled_runtime() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-disabled-management");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Demo Skill\n\nManagement operations should require enabled runtime.\n",
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

            let mut disabled_config = enabled_config;
            disabled_config.external_skills.enabled = false;

            for (tool_name, payload) in [
                ("external_skills.list", json!({})),
                (
                    "external_skills.inspect",
                    json!({ "skill_id": "demo-skill" }),
                ),
                (
                    "external_skills.remove",
                    json!({ "skill_id": "demo-skill" }),
                ),
            ] {
                let error = crate::tools::execute_tool_core_with_config(
                    ToolCoreRequest {
                        tool_name: tool_name.to_owned(),
                        payload,
                    },
                    &disabled_config,
                )
                .expect_err("disabled runtime should block lifecycle management");
                assert!(
                    error.contains("external skills runtime is disabled"),
                    "unexpected error for {tool_name}: {error}"
                );
            }

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn list_and_invoke_installed_skill_return_managed_metadata() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-list-invoke");
            fs::create_dir_all(&root).expect("create fixture root");
            let _home = ScopedHomeFixture::new("loongclaw-ext-skill-list-invoke-home");
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
            assert!(
                list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| {
                        skill["skill_id"] == "demo-skill" && skill["scope"] == "managed"
                    }),
                "managed install should appear in resolved skills list"
            );

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
        });
    }

    #[test]
    fn inspect_and_invoke_surface_skill_metadata_contract() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-metadata-contract");
            fs::create_dir_all(&root).expect("create fixture root");
            let mut home = ScopedHomeFixture::new("loongclaw-ext-skill-metadata-contract-home");
            home.set_env("LOONGCLAW_RELEASE_GUARD_TOKEN", "present");
            write_file(
                &home.path,
                ".agents/skills/release-guard/SKILL.md",
                "---\nname: release-guard\ndescription: Guard release discipline.\ninvocation_policy: both\nrequired_env:\n- LOONGCLAW_RELEASE_GUARD_TOKEN\nrequired_bins:\n- sh\nrequired_config:\n- external_skills.enabled\nallowed_tools:\n- shell.exec\nblocked_tools:\n- web.fetch\n---\n\n# Release Guard\n\nPrefer release checklists.\n",
            );
            let config = managed_runtime_config(&root);

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed");
            let listed_skill = list_outcome.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "release-guard")
                .cloned()
                .expect("release-guard should be listed");
            assert!(
                listed_skill.get("metadata").is_none(),
                "model list should not expose operator metadata: {listed_skill:?}"
            );

            let operator_list = execute_external_skills_operator_list_tool_with_config(&config)
                .expect("operator list should succeed");
            let operator_skill = operator_list.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "release-guard")
                .cloned()
                .expect("release-guard should be listed for operators");
            assert_eq!(operator_skill["invocation_policy"], "both");
            assert_eq!(operator_skill["allowed_tools"], json!(["shell.exec"]));
            assert_eq!(operator_skill["blocked_tools"], json!(["web.fetch"]));
            assert_eq!(operator_skill["eligibility"]["available"], json!(true));

            let inspect_outcome =
                execute_external_skills_operator_inspect_tool_with_config("release-guard", &config)
                    .expect("operator inspect should succeed");
            assert_eq!(
                inspect_outcome.payload["skill"]["required_env"],
                json!(["LOONGCLAW_RELEASE_GUARD_TOKEN"])
            );
            assert_eq!(
                inspect_outcome.payload["skill"]["required_config"],
                json!(["external_skills.enabled"])
            );
            assert_eq!(
                inspect_outcome.payload["skill"]["eligibility"]["available"],
                json!(true)
            );

            let invoke_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "release-guard"
                    }),
                },
                &config,
            )
            .expect("invoke should succeed");
            assert_eq!(
                invoke_outcome.payload["metadata"]["invocation_policy"],
                "both"
            );
            assert_eq!(
                invoke_outcome.payload["eligibility"]["available"],
                json!(true)
            );
            assert!(
                invoke_outcome.payload["invocation_summary"]
                    .as_str()
                    .expect("invocation summary should be text")
                    .contains("allowed_tools=shell.exec"),
                "tool restrictions should surface in invocation summary"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn invoke_rejects_manual_or_ineligible_skill_metadata_contracts() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-metadata-contract-reject");
            fs::create_dir_all(&root).expect("create fixture root");
            let home = ScopedHomeFixture::new("loongclaw-ext-skill-metadata-contract-reject-home");
            write_file(
                &home.path,
                ".agents/skills/manual-only/SKILL.md",
                "---\ninvocation_policy: manual\n---\n\n# Manual Only\n\nUse this skill only for operator-driven checks.\n",
            );
            write_file(
                &home.path,
                ".agents/skills/env-gated/SKILL.md",
                "---\nrequired_env:\n- LOONGCLAW_MISSING_TOKEN\n---\n\n# Env Gated\n\nNeeds a token before it can run.\n",
            );
            let config = managed_runtime_config(&root);

            let operator_list = execute_external_skills_operator_list_tool_with_config(&config)
                .expect("operator list should succeed");
            let env_gated = operator_list.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "env-gated")
                .cloned()
                .expect("env-gated should be listed for operators");
            assert_eq!(env_gated["eligibility"]["available"], json!(false));
            assert!(
                env_gated["eligibility"]["issues"]
                    .as_array()
                    .expect("eligibility issues should be an array")
                    .iter()
                    .any(|issue| issue.as_str() == Some("missing env `LOONGCLAW_MISSING_TOKEN`"))
            );

            let manual_error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "manual-only"
                    }),
                },
                &config,
            )
            .expect_err("manual-only skills should reject model invocation");
            assert!(manual_error.contains("invocation_policy=manual"));

            let env_error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "env-gated"
                    }),
                },
                &config,
            )
            .expect_err("missing env requirements should reject invocation");
            assert!(env_error.contains("LOONGCLAW_MISSING_TOKEN"));

            fs::remove_dir_all(&root).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn list_marks_non_executable_required_bin_as_ineligible() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-bin-eligibility");
            fs::create_dir_all(&root).expect("create fixture root");
            let mut home = ScopedHomeFixture::new("loongclaw-ext-skill-bin-eligibility-home");
            let bin_dir = unique_temp_dir("loongclaw-ext-skill-bin-eligibility-bin");
            fs::create_dir_all(&bin_dir).expect("create fake bin dir");

            let fake_bin = bin_dir.join("release-check");
            fs::write(&fake_bin, "#!/bin/sh\nexit 0\n").expect("write fake binary");
            let mut permissions = fs::metadata(&fake_bin)
                .expect("read fake binary metadata")
                .permissions();
            permissions.set_mode(0o644);
            fs::set_permissions(&fake_bin, permissions)
                .expect("mark fake binary as non-executable");

            home.set_env("PATH", &bin_dir);
            write_file(
                &home.path,
                ".agents/skills/bin-gated/SKILL.md",
                "---\nrequired_bins:\n- release-check\n---\n\n# Bin Gated\n\nNeeds a real executable on PATH.\n",
            );
            let config = managed_runtime_config(&root);

            let operator_list = execute_external_skills_operator_list_tool_with_config(&config)
                .expect("operator list should succeed");
            let listed_skill = operator_list.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "bin-gated")
                .cloned()
                .expect("bin-gated skill should be listed for operators");
            assert_eq!(listed_skill["eligibility"]["available"], json!(false));
            assert!(
                listed_skill["eligibility"]["issues"]
                    .as_array()
                    .expect("eligibility issues should be an array")
                    .iter()
                    .any(|issue| issue.as_str() == Some("missing binary `release-check`")),
                "non-executable files on PATH must not satisfy required_bins"
            );

            fs::remove_dir_all(&bin_dir).ok();
            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn discovery_resolves_managed_user_and_project_scopes_with_shadowed_duplicates() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-discovery-precedence");
            let home = unique_temp_dir("loongclaw-ext-skill-discovery-home");
            fs::create_dir_all(&root).expect("create fixture root");
            fs::create_dir_all(&home).expect("create home root");

            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Managed Demo Skill\n\nManaged install should win precedence.\n",
            );
            write_file(
                &root,
                ".agents/skills/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: Project-scoped demo skill.\n---\n\n# Project Demo Skill\n\nProject copy should be shadowed by managed.\n",
            );
            write_file(
                &root,
                ".claude/skills/project-only/SKILL.md",
                "---\nname: project-only\ndescription: Project-only skill.\n---\n\nProject-only instructions.\n",
            );
            write_file(
                &home,
                ".agents/skills/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: User-scoped demo skill.\n---\n\n# User Demo Skill\n\nUser copy should be shadowed by managed.\n",
            );
            write_file(
                &home,
                ".agents/skills/user-only/SKILL.md",
                "---\nname: user-only\ndescription: User-only skill.\n---\n\nUser-only instructions.\n",
            );

            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &home);

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
            let skills = list_outcome.payload["skills"]
                .as_array()
                .expect("skills should be an array");
            assert_eq!(
                skills.len(),
                3,
                "resolved list should contain one entry per skill id"
            );
            assert!(
                skills.iter().any(|skill| {
                    skill["skill_id"] == "demo-skill"
                        && skill["scope"] == "managed"
                        && skill["display_name"] == "Managed Demo Skill"
                }),
                "managed skill should win precedence in resolved list: {skills:?}"
            );
            assert!(
                skills.iter().any(|skill| {
                    skill["skill_id"] == "project-only"
                        && skill["scope"] == "project"
                        && skill["summary"] == "Project-only skill."
                }),
                "project-only skill should be discovered from project scope: {skills:?}"
            );
            assert!(
                skills.iter().any(|skill| {
                    skill["skill_id"] == "user-only"
                        && skill["scope"] == "user"
                        && skill["summary"] == "User-only skill."
                }),
                "user-only skill should be discovered from user scope: {skills:?}"
            );

            let shadowed = list_outcome.payload["shadowed_skills"]
                .as_array()
                .expect("shadowed_skills should be an array");
            assert_eq!(
                shadowed.len(),
                2,
                "duplicate lower-priority skills should be reported as shadowed"
            );
            assert!(
                shadowed
                    .iter()
                    .any(|skill| skill["skill_id"] == "demo-skill" && skill["scope"] == "user"),
                "user duplicate should be shadowed by managed precedence: {shadowed:?}"
            );
            assert!(
                shadowed
                    .iter()
                    .any(|skill| skill["skill_id"] == "demo-skill" && skill["scope"] == "project"),
                "project duplicate should be shadowed by managed precedence: {shadowed:?}"
            );

            let inspect_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.inspect".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("inspect should resolve the managed winner");
            assert_eq!(inspect_outcome.payload["skill"]["scope"], "managed");
            assert_eq!(
                inspect_outcome.payload["skill"]["display_name"],
                "Managed Demo Skill"
            );
            assert_eq!(
                inspect_outcome.payload["shadowed_skills"]
                    .as_array()
                    .expect("inspect should include shadowed duplicates")
                    .len(),
                2
            );

            let user_invoke = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "user-only"
                    }),
                },
                &config,
            )
            .expect("invoke should resolve user-only skills");
            assert_eq!(user_invoke.payload["scope"], "user");
            assert!(
                user_invoke.payload["instructions"]
                    .as_str()
                    .expect("instructions should be text")
                    .contains("User-only instructions"),
                "invoke should load user-scope instructions"
            );

            let project_invoke = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "project-only"
                    }),
                },
                &config,
            )
            .expect("invoke should resolve project-only skills");
            assert_eq!(project_invoke.payload["scope"], "project");
            assert!(
                project_invoke.payload["instructions"]
                    .as_str()
                    .expect("instructions should be text")
                    .contains("Project-only instructions"),
                "invoke should load project-scope instructions"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&home).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn discovery_follows_symlinked_user_skill_directories() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::symlink;

            let root = unique_temp_dir("loongclaw-ext-skill-discovery-symlink-root");
            let home = unique_temp_dir("loongclaw-ext-skill-discovery-symlink-home");
            let shared = unique_temp_dir("loongclaw-ext-skill-discovery-symlink-target");
            fs::create_dir_all(&root).expect("create fixture root");
            fs::create_dir_all(home.join(".agents/skills")).expect("create user skills root");
            fs::create_dir_all(&shared).expect("create shared skill root");
            write_file(
                &shared,
                "portable-skill/SKILL.md",
                "---\nname: portable-skill\ndescription: Symlinked user skill.\n---\n\nPortable instructions.\n",
            );
            symlink(
                shared.join("portable-skill"),
                home.join(".agents/skills/portable-skill"),
            )
            .expect("create user skill symlink");

            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &home);
            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &managed_runtime_config(&root),
            )
            .expect("symlinked user skills should be discoverable");
            assert!(
                list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| {
                        skill["skill_id"] == "portable-skill" && skill["scope"] == "user"
                    }),
                "symlinked user skill should appear in resolved discovery output"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&home).ok();
            fs::remove_dir_all(&shared).ok();
        });
    }

    #[test]
    fn invoke_requires_enabled_runtime() {
        with_managed_runtime_test(|| {
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

            let mut disabled_config = enabled_config;
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
        });
    }

    #[test]
    fn remove_installed_skill_clears_managed_entry() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-remove");
            fs::create_dir_all(&root).expect("create fixture root");
            let _home = ScopedHomeFixture::new("loongclaw-ext-skill-remove-home");
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
        });
    }

    #[test]
    fn provider_surface_does_not_fall_back_when_managed_winner_is_inactive() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-inactive-winner");
            let home = unique_temp_dir("loongclaw-ext-skill-inactive-winner-home");
            fs::create_dir_all(&root).expect("create fixture root");
            fs::create_dir_all(&home).expect("create home root");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Managed Demo Skill\n\nManaged winner should keep precedence even when inactive.\n",
            );
            write_file(
                &home,
                ".agents/skills/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: user fallback should stay shadowed.\n---\n\n# User Demo Skill\n\nDo not silently take over.\n",
            );

            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &home);

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

            let install_root = root.join("external-skills-installed");
            let index_path = install_root.join("index.json");
            let mut index: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&index_path).expect("read index"))
                    .expect("parse index");
            index["skills"][0]["active"] = json!(false);
            fs::write(
                &index_path,
                serde_json::to_string_pretty(&index).expect("encode index"),
            )
            .expect("write tampered index");

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed even when managed winner is inactive");
            assert!(
                !list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| skill["skill_id"] == "demo-skill"),
                "provider surface should not fall back to lower-scope duplicates: {}",
                list_outcome.payload
            );

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect_err("invoke should reject inactive managed winners");
            assert!(
                error.contains("inactive"),
                "expected inactive winner error, got: {error}"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&home).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn provider_surface_skips_unreadable_local_skills_without_failing_discovery() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::PermissionsExt;

            let root = unique_temp_dir("loongclaw-ext-skill-unreadable-discovery");
            fs::create_dir_all(&root).expect("create fixture root");
            let _home = ScopedHomeFixture::new("loongclaw-ext-skill-unreadable-discovery-home");
            write_file(
                &root,
                ".agents/skills/healthy-skill/SKILL.md",
                "---\nname: healthy-skill\ndescription: healthy project skill.\n---\n\nHealthy skill instructions.\n",
            );
            write_file(
                &root,
                ".agents/skills/broken-skill/SKILL.md",
                "---\nname: broken-skill\ndescription: unreadable project skill.\n---\n\nBroken skill instructions.\n",
            );

            let unreadable_path = root.join(".agents/skills/broken-skill/SKILL.md");
            let mut perms = fs::metadata(&unreadable_path)
                .expect("read metadata")
                .permissions();
            perms.set_mode(0o000);
            fs::set_permissions(&unreadable_path, perms).expect("set unreadable permissions");

            let config = managed_runtime_config(&root);
            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed when one discovered skill is unreadable");

            let skills = list_outcome.payload["skills"]
                .as_array()
                .expect("skills should be an array");
            assert!(
                skills
                    .iter()
                    .any(|skill| skill["skill_id"] == "healthy-skill"),
                "healthy project skill should remain discoverable: {skills:?}"
            );
            assert!(
                skills
                    .iter()
                    .all(|skill| skill["skill_id"] != "broken-skill"),
                "unreadable skill should be skipped instead of failing discovery: {skills:?}"
            );

            let mut cleanup_perms = fs::metadata(&unreadable_path)
                .expect("read metadata for cleanup")
                .permissions();
            cleanup_perms.set_mode(0o644);
            fs::set_permissions(&unreadable_path, cleanup_perms).ok();
            fs::remove_dir_all(&root).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn provider_surface_fails_closed_when_unreadable_user_winner_has_project_fallback() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::PermissionsExt;

            let root = unique_temp_dir("loongclaw-ext-skill-unreadable-user-winner");
            let home = unique_temp_dir("loongclaw-ext-skill-unreadable-user-winner-home");
            fs::create_dir_all(&root).expect("create fixture root");
            fs::create_dir_all(&home).expect("create home root");
            write_file(
                &root,
                ".agents/skills/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: project fallback should stay blocked.\n---\n\nProject fallback instructions.\n",
            );
            write_file(
                &home,
                ".agents/skills/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: unreadable user winner.\n---\n\nBroken user instructions.\n",
            );

            let unreadable_path = home.join(".agents/skills/demo-skill/SKILL.md");
            let mut perms = fs::metadata(&unreadable_path)
                .expect("read metadata")
                .permissions();
            perms.set_mode(0o000);
            fs::set_permissions(&unreadable_path, perms).expect("set unreadable permissions");

            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &home);

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed when the higher-precedence local winner is unreadable");

            assert!(
                list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .all(|skill| skill["skill_id"] != "demo-skill"),
                "provider surface should fail closed instead of promoting the project fallback: {}",
                list_outcome.payload
            );

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect_err("invoke should report the unreadable higher-precedence local winner");
            assert!(
                error.contains("failed to read external skill source")
                    || error.contains("failed to inspect external skill source"),
                "expected unreadable local winner error, got: {error}"
            );

            let mut cleanup_perms = fs::metadata(&unreadable_path)
                .expect("read metadata for cleanup")
                .permissions();
            cleanup_perms.set_mode(0o644);
            fs::set_permissions(&unreadable_path, cleanup_perms).ok();
            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&home).ok();
        });
    }

    #[test]
    fn provider_surface_hides_model_hidden_skills_and_snapshot_auto_exposure() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-model-hidden");
            fs::create_dir_all(&root).expect("create fixture root");
            let _home = ScopedHomeFixture::new("loongclaw-ext-skill-model-hidden-home");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: operator-only managed skill.\nmodel_visibility: hidden\n---\n\n# Demo Skill\n\nHide this skill from the model surface.\n",
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
            assert!(
                !list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| skill["skill_id"] == "demo-skill"),
                "model-hidden skills should stay off the provider-visible surface: {}",
                list_outcome.payload
            );

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect_err("invoke should reject model-hidden skills on the provider surface");
            assert!(
                error.contains("operator-only") || error.contains("model"),
                "expected model-hidden error, got: {error}"
            );

            let lines = installed_skill_snapshot_lines_with_config(&config)
                .expect("snapshot should succeed");
            assert!(
                lines.is_empty(),
                "model-hidden skills should not be auto-exposed in installed snapshots: {lines:?}"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn provider_surface_hides_skills_with_missing_required_env() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-required-env");
            fs::create_dir_all(&root).expect("create fixture root");
            let _home = ScopedHomeFixture::new("loongclaw-ext-skill-required-env-home");
            write_file(
                &root,
                ".agents/skills/env-guarded/SKILL.md",
                "---\nname: env-guarded\ndescription: requires an explicit env var.\nrequires_env:\n  - DEMO_SKILL_TOKEN\n---\n\n# Env Guarded Skill\n\nOnly run when the token exists.\n",
            );
            let config = managed_runtime_config(&root);

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed");
            assert!(
                !list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| skill["skill_id"] == "env-guarded"),
                "skills with missing required env should stay hidden from provider list: {}",
                list_outcome.payload
            );

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "env-guarded"
                    }),
                },
                &config,
            )
            .expect_err("invoke should reject skills with missing required env");
            assert!(
                error.contains("DEMO_SKILL_TOKEN"),
                "expected missing env variable in error, got: {error}"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn model_surface_redacts_operator_only_skill_metadata() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-model-redaction");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                ".agents/skills/demo-skill/SKILL.md",
                "---\nname: demo-skill\ndescription: eligible project skill.\nrequires_env:\n  - DEMO_SKILL_TOKEN\nrequires_bin:\n  - sh\nrequires_paths:\n  - fixtures/present.txt\n---\n\n# Demo Skill\n\nOnly expose model-safe metadata on the provider surface.\n",
            );
            write_file(&root, "fixtures/present.txt", "present");
            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("DEMO_SKILL_TOKEN", "present");

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("model list should succeed");
            let model_skill = list_outcome.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "demo-skill")
                .expect("model list should include demo-skill");
            assert!(
                model_skill.get("model_visibility").is_none(),
                "model list should not expose visibility internals: {model_skill:?}"
            );
            assert!(
                model_skill.get("required_env").is_none(),
                "model list should not expose required_env: {model_skill:?}"
            );
            assert!(
                model_skill.get("required_bin").is_none(),
                "model list should not expose required_bin: {model_skill:?}"
            );
            assert!(
                model_skill.get("required_paths").is_none(),
                "model list should not expose required_paths: {model_skill:?}"
            );
            assert!(
                model_skill.get("eligibility").is_none(),
                "model list should not expose eligibility diagnostics: {model_skill:?}"
            );

            let operator_list = execute_external_skills_operator_list_tool_with_config(&config)
                .expect("operator list should succeed");
            let operator_skill = operator_list.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "demo-skill")
                .expect("operator list should include demo-skill");
            assert_eq!(operator_skill["model_visibility"], "visible");
            assert_eq!(operator_skill["required_env"], json!(["DEMO_SKILL_TOKEN"]));
            assert_eq!(operator_skill["required_bin"], json!(["sh"]));
            assert_eq!(
                operator_skill["required_paths"],
                json!(["fixtures/present.txt"])
            );
            assert_eq!(operator_skill["eligibility"]["available"], true);

            let inspect_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.inspect".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("model inspect should succeed");
            let model_inspect_skill = inspect_outcome.payload["skill"]
                .as_object()
                .expect("inspect skill should be an object");
            assert!(
                !model_inspect_skill.contains_key("model_visibility"),
                "model inspect should not expose visibility internals: {model_inspect_skill:?}"
            );
            assert!(
                !model_inspect_skill.contains_key("required_env"),
                "model inspect should not expose required_env: {model_inspect_skill:?}"
            );
            assert!(
                !model_inspect_skill.contains_key("required_bin"),
                "model inspect should not expose required_bin: {model_inspect_skill:?}"
            );
            assert!(
                !model_inspect_skill.contains_key("required_paths"),
                "model inspect should not expose required_paths: {model_inspect_skill:?}"
            );
            assert!(
                !model_inspect_skill.contains_key("eligibility"),
                "model inspect should not expose eligibility diagnostics: {model_inspect_skill:?}"
            );

            let operator_inspect =
                execute_external_skills_operator_inspect_tool_with_config("demo-skill", &config)
                    .expect("operator inspect should succeed");
            assert_eq!(
                operator_inspect.payload["skill"]["required_env"],
                json!(["DEMO_SKILL_TOKEN"])
            );
            assert_eq!(
                operator_inspect.payload["skill"]["required_bin"],
                json!(["sh"])
            );
            assert_eq!(
                operator_inspect.payload["skill"]["required_paths"],
                json!(["fixtures/present.txt"])
            );
            assert_eq!(
                operator_inspect.payload["skill"]["eligibility"]["available"],
                true
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn provider_surface_hides_skills_with_non_executable_required_commands() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::PermissionsExt;

            let root = unique_temp_dir("loongclaw-ext-skill-required-bin-exec");
            fs::create_dir_all(root.join("bin")).expect("create bin dir");
            write_file(
                &root,
                ".agents/skills/bin-guarded/SKILL.md",
                "---\nname: bin-guarded\ndescription: requires an executable command.\nrequires_bin:\n  - demo-bin\n---\n\n# Bin Guarded\n\nOnly run when the command is executable.\n",
            );
            write_file(&root, "bin/demo-bin", "#!/bin/sh\necho guarded\n");
            let command_path = root.join("bin/demo-bin");
            let mut perms = fs::metadata(&command_path)
                .expect("read command metadata")
                .permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&command_path, perms).expect("set non-executable permissions");

            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("PATH", root.join("bin").as_os_str());

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed");
            assert!(
                !list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| skill["skill_id"] == "bin-guarded"),
                "skills with non-executable required commands should stay hidden: {}",
                list_outcome.payload
            );

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "bin-guarded"
                    }),
                },
                &config,
            )
            .expect_err("invoke should reject non-executable required commands");
            assert!(
                error.contains("demo-bin"),
                "expected missing command in error, got: {error}"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn provider_surface_skips_broken_managed_installs_without_failing_discovery() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-broken-managed-discovery");
            let home = unique_temp_dir("loongclaw-ext-skill-broken-managed-discovery-home");
            fs::create_dir_all(&root).expect("create fixture root");
            fs::create_dir_all(&home).expect("create home root");
            write_file(
                &root,
                "source/healthy-skill/SKILL.md",
                "---\nname: healthy-skill\ndescription: healthy managed skill.\n---\n\nHealthy managed skill instructions.\n",
            );
            write_file(
                &home,
                ".agents/skills/broken-skill/SKILL.md",
                "---\nname: broken-skill\ndescription: lower-precedence fallback should stay shadowed.\n---\n\nDo not silently fall back.\n",
            );
            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &home);

            crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/healthy-skill"
                    }),
                },
                &config,
            )
            .expect("healthy managed install should succeed");

            let install_root = root.join("external-skills-installed");
            let mut index =
                load_installed_skill_index(&install_root).expect("load managed skill index");
            index.skills.push(InstalledSkillEntry {
                skill_id: "broken-skill".to_owned(),
                display_name: "Broken Skill".to_owned(),
                summary: "broken managed skill".to_owned(),
                source_kind: "directory".to_owned(),
                source_path: root.join("source/broken-skill").display().to_string(),
                install_path: install_root.join("broken-skill").display().to_string(),
                skill_md_path: install_root
                    .join("broken-skill/SKILL.md")
                    .display()
                    .to_string(),
                sha256: "deadbeef".to_owned(),
                installed_at_unix: 0,
                active: true,
            });
            persist_installed_skill_index(&install_root, &mut index)
                .expect("persist index with broken managed entry");

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed when one managed install is broken");
            let skills = list_outcome.payload["skills"]
                .as_array()
                .expect("skills should be an array");
            assert!(
                skills
                    .iter()
                    .any(|skill| skill["skill_id"] == "healthy-skill"),
                "healthy managed skill should remain discoverable: {skills:?}"
            );
            assert!(
                skills
                    .iter()
                    .all(|skill| skill["skill_id"] != "broken-skill"),
                "broken managed skill should fail closed instead of falling back: {skills:?}"
            );

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "broken-skill"
                    }),
                },
                &config,
            )
            .expect_err("broken managed install should not fall back to user scope");
            assert!(
                error.contains("failed to inspect managed external skill install"),
                "expected managed install error, got: {error}"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&home).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn replace_failed_install_preserves_previous_managed_skill() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::PermissionsExt;

            let root = unique_temp_dir("loongclaw-ext-skill-replace-rollback");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                "source/demo-skill-v1/SKILL.md",
                "# Demo Skill\n\nStable installed skill.\n",
            );
            write_file(
                &root,
                "source/demo-skill-v2/SKILL.md",
                "# Demo Skill\n\nReplacement should fail safely.\n",
            );
            write_file(
                &root,
                "source/demo-skill-v2/private.txt",
                "copy should fail on unreadable file",
            );
            let unreadable_path = root.join("source/demo-skill-v2/private.txt");
            let mut perms = fs::metadata(&unreadable_path)
                .expect("read metadata")
                .permissions();
            perms.set_mode(0o000);
            fs::set_permissions(&unreadable_path, perms).expect("set unreadable permissions");

            let config = managed_runtime_config(&root);
            crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/demo-skill-v1",
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("initial install should succeed");

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/demo-skill-v2",
                        "skill_id": "demo-skill",
                        "replace": true
                    }),
                },
                &config,
            )
            .expect_err("replacement install should fail");
            assert!(
                error.contains("failed to copy external skill file"),
                "unexpected replacement failure: {error}"
            );

            let invoke_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("previous install should remain available after failed replace");
            assert!(
                invoke_outcome.payload["instructions"]
                    .as_str()
                    .expect("instructions should be text")
                    .contains("Stable installed skill"),
                "failed replace must preserve previous managed install"
            );

            let install_root = root.join("external-skills-installed");
            let transient_entries = fs::read_dir(&install_root)
                .expect("install root should exist")
                .map(|entry| {
                    entry
                        .expect("read install root entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .filter(|name| name.starts_with(".incoming-") || name.starts_with(".backup-"))
                .collect::<Vec<_>>();
            assert!(
                transient_entries.is_empty(),
                "failed replace must clean temporary directories: {transient_entries:?}"
            );

            let mut cleanup_perms = fs::metadata(&unreadable_path)
                .expect("read metadata for cleanup")
                .permissions();
            cleanup_perms.set_mode(0o644);
            fs::set_permissions(&unreadable_path, cleanup_perms).ok();
            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn tampered_index_paths_do_not_escape_managed_install_root() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-index-tamper");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Demo Skill\n\nInspectable managed content.\n",
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

            let install_root = root.join("external-skills-installed");
            let index_path = install_root.join("index.json");
            let escape_root = unique_temp_dir("loongclaw-ext-skill-index-escape");
            fs::create_dir_all(&escape_root).expect("create escape root");
            write_file(
                &escape_root,
                "SKILL.md",
                "# Escape Skill\n\nDo not trust me.\n",
            );

            let mut index: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&index_path).expect("read index"))
                    .expect("parse index");
            index["skills"][0]["install_path"] = json!(escape_root.display().to_string());
            index["skills"][0]["skill_md_path"] =
                json!(escape_root.join("SKILL.md").display().to_string());
            fs::write(
                &index_path,
                serde_json::to_string_pretty(&index).expect("encode tampered index"),
            )
            .expect("write tampered index");

            let inspect_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.inspect".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("inspect should stay inside managed root");
            assert!(
                inspect_outcome.payload["instructions_preview"]
                    .as_str()
                    .expect("preview should exist")
                    .contains("Inspectable managed content"),
                "inspect should read the managed skill content"
            );

            crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.remove".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("remove should stay inside managed root");

            assert!(
                escape_root.exists(),
                "tampered install path outside managed root must not be removed"
            );
            assert!(
                !install_root.join("demo-skill").exists(),
                "managed install should be removed"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&escape_root).ok();
        });
    }

    #[test]
    fn tampered_index_metadata_is_rehydrated_from_managed_skill_markdown() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-index-metadata");
            fs::create_dir_all(&root).expect("create fixture root");
            let _home = ScopedHomeFixture::new("loongclaw-ext-skill-index-metadata-home");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Demo Skill\n\nPrefer evidence over stale index metadata.\n",
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

            let install_root = root.join("external-skills-installed");
            let index_path = install_root.join("index.json");
            let mut index: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&index_path).expect("read index"))
                    .expect("parse index");
            index["skills"][0]["display_name"] = json!("Forged Display");
            index["skills"][0]["summary"] = json!("Forged summary");
            index["skills"][0]["sha256"] = json!("forged-digest");
            fs::write(
                &index_path,
                serde_json::to_string_pretty(&index).expect("encode tampered index"),
            )
            .expect("write tampered index");

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("list should succeed with rehydrated metadata");
            let demo_skill = list_outcome.payload["skills"]
                .as_array()
                .expect("skills should be an array")
                .iter()
                .find(|skill| skill["skill_id"] == "demo-skill")
                .expect("managed demo-skill should remain discoverable");
            assert_eq!(demo_skill["display_name"], "Demo Skill");
            assert_eq!(
                demo_skill["summary"],
                "Prefer evidence over stale index metadata."
            );
            assert_ne!(demo_skill["sha256"], "forged-digest");

            let invoke_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.invoke".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("invoke should succeed with rehydrated metadata");
            assert_eq!(invoke_outcome.payload["display_name"], "Demo Skill");
            assert_eq!(
                invoke_outcome.payload["summary"],
                "Prefer evidence over stale index metadata."
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn list_skips_missing_managed_installs_instead_of_failing_discovery() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-discovery-broken-managed");
            let home = unique_temp_dir("loongclaw-ext-skill-discovery-broken-managed-home");
            fs::create_dir_all(&root).expect("create fixture root");
            fs::create_dir_all(&home).expect("create home root");

            write_file(
                &root,
                "source/broken-managed/SKILL.md",
                "# Broken Managed\n\nThis managed install will be removed after indexing.\n",
            );
            write_file(
                &home,
                ".agents/skills/user-only/SKILL.md",
                "# User Only\n\nKeep discovery alive when managed state is broken.\n",
            );

            let config = managed_runtime_config(&root);
            let mut env = crate::test_support::ScopedEnv::new();
            env.set("HOME", &home);

            crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/broken-managed"
                    }),
                },
                &config,
            )
            .expect("install should succeed");

            fs::remove_dir_all(
                root.join("external-skills-installed")
                    .join("broken-managed"),
            )
            .expect("remove managed install to simulate broken index entry");

            let list_outcome = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.list".to_owned(),
                    payload: json!({}),
                },
                &config,
            )
            .expect("broken managed installs should be skipped during discovery");

            assert!(
                list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| skill["skill_id"] == "user-only" && skill["scope"] == "user"),
                "healthy user skills should remain discoverable when a managed install is missing"
            );
            assert!(
                !list_outcome.payload["skills"]
                    .as_array()
                    .expect("skills should be an array")
                    .iter()
                    .any(|skill| skill["skill_id"] == "broken-managed"),
                "broken managed installs should be dropped from discovery output"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&home).ok();
        });
    }

    #[cfg(unix)]
    #[test]
    fn inspect_rejects_symlinked_managed_install_directory() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::symlink;

            let root = unique_temp_dir("loongclaw-ext-skill-install-symlink-swap");
            fs::create_dir_all(&root).expect("create fixture root");
            write_file(
                &root,
                "source/demo-skill/SKILL.md",
                "# Demo Skill\n\nManaged install should stay real.\n",
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

            let install_path = root.join("external-skills-installed").join("demo-skill");
            fs::remove_dir_all(&install_path).expect("remove managed install");

            let escape_root = unique_temp_dir("loongclaw-ext-skill-install-symlink-target");
            fs::create_dir_all(&escape_root).expect("create escape root");
            write_file(
                &escape_root,
                "SKILL.md",
                "# Escape Skill\n\nDo not follow symlinked installs.\n",
            );
            symlink(&escape_root, &install_path).expect("create managed install symlink");

            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.inspect".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect_err("inspect should reject symlinked managed installs");
            assert!(error.contains("cannot be a symlink"));

            crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.remove".to_owned(),
                    payload: json!({
                        "skill_id": "demo-skill"
                    }),
                },
                &config,
            )
            .expect("remove should delete only the managed symlink");

            assert!(
                escape_root.exists(),
                "managed remove must not delete the symlink target"
            );
            assert!(
                !install_path.exists(),
                "managed symlink should be removed from install root"
            );

            fs::remove_dir_all(&root).ok();
            fs::remove_dir_all(&escape_root).ok();
        });
    }

    #[test]
    fn install_from_tar_gz_archive_extracts_wrapped_skill_root() {
        with_managed_runtime_test(|| {
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
            let install_root = root.join("external-skills-installed");
            let staging_entries = fs::read_dir(&install_root)
                .expect("install root should exist")
                .map(|entry| {
                    entry
                        .expect("read install root entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .filter(|name| name.starts_with(".staging-"))
                .collect::<Vec<_>>();
            assert!(
                staging_entries.is_empty(),
                "successful archive install must clean staging directories: {staging_entries:?}"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn install_from_archive_rejects_symlink_entries() {
        with_managed_runtime_test(|| {
            let root = unique_temp_dir("loongclaw-ext-skill-archive-symlink");
            fs::create_dir_all(&root).expect("create fixture root");
            let archive_path = root.join("demo-skill.tar.gz");
            {
                let tar_gz = fs::File::create(&archive_path).expect("create archive");
                let encoder = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
                let mut tar = tar::Builder::new(encoder);

                let skill_bytes = b"# Demo Skill\n\nArchive symlink should fail.\n";
                let mut skill_header = tar::Header::new_gnu();
                skill_header
                    .set_path("bundle/demo-skill/SKILL.md")
                    .expect("set skill path");
                skill_header.set_size(skill_bytes.len() as u64);
                skill_header.set_mode(0o644);
                skill_header.set_cksum();
                tar.append(&skill_header, &skill_bytes[..])
                    .expect("append skill file");

                let mut symlink_header = tar::Header::new_gnu();
                symlink_header
                    .set_path("bundle/demo-skill/leak.txt")
                    .expect("set symlink path");
                symlink_header.set_entry_type(tar::EntryType::Symlink);
                symlink_header
                    .set_link_name("/etc/passwd")
                    .expect("set symlink target");
                symlink_header.set_size(0);
                symlink_header.set_mode(0o777);
                symlink_header.set_cksum();
                tar.append(&symlink_header, std::io::empty())
                    .expect("append symlink");

                tar.finish().expect("finish archive");
            }

            let config = managed_runtime_config(&root);
            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "demo-skill.tar.gz"
                    }),
                },
                &config,
            )
            .expect_err("archive symlink should be rejected");
            assert!(error.contains("cannot contain symlinks or hard links"));
            let install_root = root.join("external-skills-installed");
            let staging_entries = fs::read_dir(&install_root)
                .expect("install root should exist")
                .map(|entry| {
                    entry
                        .expect("read install root entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .filter(|name| name.starts_with(".staging-"))
                .collect::<Vec<_>>();
            assert!(
                staging_entries.is_empty(),
                "failed archive install must not leave staging directories behind: {staging_entries:?}"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn inspect_returns_preview_and_missing_skill_md_is_rejected() {
        with_managed_runtime_test(|| {
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
        });
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_symlinked_skill_markdown() {
        with_managed_runtime_test(|| {
            use std::os::unix::fs::symlink;

            let root = unique_temp_dir("loongclaw-ext-skill-symlinked-skill-md");
            fs::create_dir_all(root.join("source/demo-skill")).expect("create skill directory");
            write_file(&root, "outside.md", "# Outside\n\nDo not follow.\n");
            symlink(
                root.join("outside.md"),
                root.join("source/demo-skill").join("SKILL.md"),
            )
            .expect("create symlink");

            let config = managed_runtime_config(&root);
            let error = crate::tools::execute_tool_core_with_config(
                ToolCoreRequest {
                    tool_name: "external_skills.install".to_owned(),
                    payload: json!({
                        "path": "source/demo-skill"
                    }),
                },
                &config,
            )
            .expect_err("symlinked skill markdown should be rejected");
            assert!(error.contains("symlink"));

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn installed_skill_snapshot_is_hidden_when_runtime_is_disabled() {
        with_managed_runtime_test(|| {
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

            let mut disabled_config = enabled_config;
            disabled_config.external_skills.enabled = false;

            let lines = installed_skill_snapshot_lines_with_config(&disabled_config)
                .expect("snapshot should succeed");
            assert!(
                lines.is_empty(),
                "disabled runtime should not expose skills"
            );

            fs::remove_dir_all(&root).ok();
        });
    }

    #[test]
    fn load_directory_skill_markdown_rejects_oversized_skill_files() {
        let root = unique_temp_dir("loongclaw-ext-skill-oversized");
        fs::create_dir_all(&root).expect("create fixture root");
        fs::write(
            root.join(DEFAULT_SKILL_FILENAME),
            vec![b'a'; DEFAULT_MAX_DOWNLOAD_BYTES + 1],
        )
        .expect("write oversized skill markdown");

        let error = load_directory_skill_markdown(&root).expect_err("oversized skill should fail");
        assert!(
            error.contains("exceeds the"),
            "unexpected oversized skill error: {error}"
        );

        fs::remove_dir_all(&root).ok();
    }
}
