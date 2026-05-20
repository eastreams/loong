use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufWriter, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flate2::read::GzDecoder;
use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tar::Archive;
use tempfile::Builder as TempFileBuilder;

use super::skills_scan::{
    ExternalSkillSecurityDecision, parse_skill_security_decision, scan_external_skill_tree,
};
use super::skills_sources::{
    ExternalSkillSourceKind, ResolvedExternalSkillCandidate, resolve_external_skill_candidate,
};
#[cfg(test)]
use super::skills_sources::{
    default_external_skill_search_sources, parse_external_skill_source_kind,
    search_query_for_external_skill_source,
};
use super::tool_search::{rank_searchable_entries, searchable_entry_from_manual_definition};

const DEFAULT_DOWNLOAD_DIR_NAME: &str = "external-skills-downloads";
const DEFAULT_INSTALL_DIR_NAME: &str = ".loong/skills";
const DEFAULT_SKILL_FILENAME: &str = "SKILL.md";
const DEFAULT_INDEX_FILENAME: &str = "index.json";
const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;
const HARD_MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;
const DEFAULT_SKILL_RESOURCE_LIST_LIMIT: usize = 64;
#[cfg(test)]
const INSTALLED_SKILL_SNAPSHOT_HINT: &str =
    "installed managed skill; read its SKILL.md or use skills.inspect for details";
const PROJECT_DISCOVERY_DIRS: [(&str, usize); 5] = [
    (".loong/skills", 0),
    (".agents/skills", 1),
    (".codex/skills", 2),
    (".claude/skills", 3),
    ("skills", 4),
];
const USER_DISCOVERY_DIRS: [(&str, usize); 4] = [
    (".loong/skills", 0),
    (".agents/skills", 1),
    (".codex/skills", 2),
    (".claude/skills", 3),
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
    license: Option<String>,
    compatibility: Option<String>,
    metadata: BTreeMap<String, String>,
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
    compatibility: Option<String>,
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
            compatibility: entry.compatibility,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelVisibleSkillCatalogEntry {
    pub(super) skill_id: String,
    pub(super) description: String,
    pub(super) location: String,
    pub(super) skill_root: Option<PathBuf>,
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
    missing_config: Vec<String>,
    issues: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    license: Option<String>,
    compatibility: Option<String>,
    #[serde(default, deserialize_with = "deserialize_skill_metadata_map")]
    metadata: BTreeMap<String, String>,
    #[serde(default, alias = "model-visibility")]
    model_visibility: SkillModelVisibility,
    #[serde(default, alias = "disable-model-invocation")]
    disable_model_invocation: bool,
    #[serde(default, alias = "invocation-policy")]
    invocation_policy: Option<SkillInvocationPolicy>,
    #[serde(default, alias = "requires_env", alias = "required-env")]
    required_env: Vec<String>,
    #[serde(
        default,
        alias = "requires_bin",
        alias = "required-bin",
        alias = "required_bins",
        alias = "requires_bins",
        alias = "requires_commands"
    )]
    required_bins: Vec<String>,
    #[serde(default, alias = "requires_paths", alias = "required-paths")]
    required_paths: Vec<String>,
    #[serde(default, alias = "required-config")]
    required_config: Vec<String>,
    #[serde(
        default,
        alias = "allowed-tools",
        deserialize_with = "deserialize_skill_string_list"
    )]
    allowed_tools: Vec<String>,
    #[serde(
        default,
        alias = "blocked-tools",
        deserialize_with = "deserialize_skill_string_list"
    )]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SkillDiscoveryResolution {
    Active,
    Shadowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillDiscoveryMode {
    Search,
    Recommend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SkillDiscoveryInventorySummary {
    visible_skill_count: usize,
    shadowed_skill_count: usize,
    blocked_skill_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct SkillResourceListing {
    files: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RankedSkillDiscoveryResult {
    #[serde(flatten)]
    skill: DiscoveredSkillEntry,
    resolution: SkillDiscoveryResolution,
    match_reasons: Vec<String>,
    limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RankedBlockedSkillDiscoveryResult {
    skill_id: String,
    error: String,
    match_reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillAudience {
    Model,
    Operator,
}

#[derive(Debug, Clone, Default)]
struct SkillsPolicyOverride {
    enabled: Option<bool>,
    require_download_approval: Option<bool>,
    allowed_domains: Option<BTreeSet<String>>,
    blocked_domains: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalSkillDownloadPlan {
    source_kind: ExternalSkillSourceKind,
    candidate: ResolvedExternalSkillCandidate,
    artifact_url: String,
    source_skill_id: Option<String>,
    selected_route_label: Option<String>,
    selected_route_url: Option<String>,
}

trait ExternalSkillDownloadPlanHttp {
    fn get_text(&self, url: &str) -> Result<String, String>;

    fn get_json(&self, url: &str) -> Result<Value, String>;
}

struct ReqwestExternalSkillDownloadPlanHttp<'a> {
    client: &'a reqwest::blocking::Client,
    policy: &'a super::runtime_config::SkillsRuntimePolicy,
}

struct ValidatedExternalSkillUrl {
    parsed_url: reqwest::Url,
    host: String,
}

#[derive(Debug, Default)]
struct ScopedDirCleanup(Option<PathBuf>);

static SKILLS_POLICY_OVERRIDE: OnceLock<RwLock<SkillsPolicyOverride>> = OnceLock::new();

pub(super) fn execute_skills_policy_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.policy payload must be an object".to_owned())?;
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("get")
        .to_ascii_lowercase();

    if !matches!(action.as_str(), "get" | "set" | "reset") {
        return Err(format!(
            "skills.policy payload.action must be `get`, `set`, or `reset`, got `{action}`"
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
                    "skills policy update requires explicit authorization; set payload.policy_update_approved=true after user approval"
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
                .map_err(|error| format!("skills policy lock poisoned: {error}"))?;

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
                    "skills policy update requires explicit authorization; set payload.policy_update_approved=true after user approval"
                        .to_owned(),
                );
            }

            let override_store = policy_override_store();
            let mut override_state = override_store
                .write()
                .map_err(|error| format!("skills policy lock poisoned: {error}"))?;
            *override_state = SkillsPolicyOverride::default();

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
        _ => Err("unreachable skills policy action".to_owned()),
    }
}

pub(super) fn execute_skills_fetch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.fetch payload must be an object".to_owned())?;
    let reference = parse_required_external_skill_reference(payload, "skills.fetch")?;
    if reqwest::Url::parse(reference.as_str()).is_ok() {
        ensure_external_skill_https_url(reference.as_str(), "download")?;
    }

    let policy = require_enabled_runtime_policy(config)?;
    if reqwest::Url::parse(reference.as_str()).is_ok() {
        let _ = validate_external_skill_network_target(reference.as_str(), &policy, "download")?;
    }

    let approval_granted = payload
        .get("approval_granted")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if policy.require_download_approval && !approval_granted {
        return Err(
            "skills download requires explicit authorization; set payload.approval_granted=true after user approval"
                .to_owned(),
        );
    }

    let max_bytes = parse_max_download_bytes(payload)?;
    let save_as = parse_optional_string(payload, "save_as")?;

    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .user_agent("loong-external-skills/0.1")
        .build()
        .map_err(|error| format!("failed to build HTTP client for skills download: {error}"))?;

    let resolution_http = ReqwestExternalSkillDownloadPlanHttp {
        client: &client,
        policy: &policy,
    };
    let download_plan = resolve_external_skill_download_plan(reference.as_str(), &resolution_http)?;
    let candidate_payload = serialize_external_skill_candidate(&download_plan.candidate)?;
    let validated_download_url = validate_external_skill_network_target(
        download_plan.artifact_url.as_str(),
        &policy,
        "download",
    )?;
    let response = send_external_skill_get_request(&client, &validated_download_url, "download")?;
    let content_length = response.content_length();
    let mut response = response;

    let output_dir = resolve_download_dir(config);
    fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create skills download directory {}: {error}",
            output_dir.display()
        )
    })?;

    let requested_name = save_as
        .as_deref()
        .map(sanitize_filename)
        .filter(|value| !value.is_empty());
    let derived_name = requested_name
        .unwrap_or_else(|| derive_filename_from_url(&validated_download_url.parsed_url));
    let download = stream_download_to_unique_path(
        &mut response,
        content_length,
        max_bytes,
        &output_dir,
        &derived_name,
        "skills download",
    )?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "reference": reference,
            "url": download_plan.artifact_url,
            "host": validated_download_url.host,
            "saved_path": download.path.display().to_string(),
            "bytes_downloaded": download.bytes_downloaded,
            "sha256": download.sha256,
            "approval_required": policy.require_download_approval,
            "approval_granted": approval_granted,
            "max_bytes": max_bytes,
            "source_kind": download_plan.source_kind.as_str(),
            "source_skill_id": download_plan.source_skill_id,
            "selected_route_label": download_plan.selected_route_label,
            "selected_route_url": download_plan.selected_route_url,
            "candidate": candidate_payload,
            "policy": policy_payload(&policy),
        }),
    })
}

#[cfg(test)]
pub(super) fn execute_skills_resolve_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let _ = require_enabled_runtime_policy(config)?;
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.resolve payload must be an object".to_owned())?;
    let reference = parse_required_external_skill_reference(payload, "skills.resolve")?;
    let candidate = resolve_external_skill_candidate(reference.as_str())?;
    let candidate_payload = serialize_external_skill_candidate(&candidate)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "reference": reference,
            "candidate": candidate_payload,
        }),
    })
}

#[cfg(test)]
pub(super) fn execute_skills_source_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let _ = require_enabled_runtime_policy(config)?;
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.source_search payload must be an object".to_owned())?;
    let query = parse_required_query(payload, "skills.source_search")?;
    let max_results = parse_optional_source_search_limit(payload, "skills.source_search")?
        .unwrap_or(5)
        .clamp(1, 10);
    let source_kinds = parse_external_skill_search_sources(payload, "skills.source_search")?;
    let per_source_limit = max_results.clamp(1, 5);

    let mut collected_results = Vec::new();
    let mut source_errors = Vec::new();

    for (source_priority, source_kind) in source_kinds.iter().copied().enumerate() {
        let Some(source_query) =
            search_query_for_external_skill_source(source_kind, query.as_str())
        else {
            continue;
        };

        let search_request = ToolCoreRequest {
            tool_name: "web.search".to_owned(),
            payload: json!({
                "query": source_query,
                "max_results": per_source_limit,
            }),
        };
        let search_outcome =
            super::web_search::execute_web_search_tool_with_config(search_request, config);

        let search_outcome = match search_outcome {
            Ok(search_outcome) => search_outcome,
            Err(error) => {
                let error_payload = json!({
                    "source_kind": source_kind.as_str(),
                    "error": error,
                });
                source_errors.push(error_payload);
                continue;
            }
        };

        let raw_results = search_outcome
            .payload
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let normalized_results =
            normalize_external_skill_search_results(source_kind, source_priority, &raw_results)?;
        collected_results.extend(normalized_results);
    }

    collected_results.sort_by(search_result_ordering);
    collected_results.dedup_by(|left, right| {
        let left_reference = left
            .get("candidate")
            .and_then(|value| value.get("canonical_reference"));
        let right_reference = right
            .get("candidate")
            .and_then(|value| value.get("canonical_reference"));
        left_reference == right_reference
    });
    collected_results.truncate(max_results);

    if collected_results.is_empty() && !source_errors.is_empty() {
        let rendered_errors = source_errors
            .iter()
            .filter_map(|value| value["error"].as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!(
            "skills search failed for all requested sources: {rendered_errors}"
        ));
    }

    let resolved_sources = source_kinds
        .iter()
        .map(|source_kind| source_kind.as_str())
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "query": query,
            "sources": resolved_sources,
            "results": collected_results,
            "source_errors": source_errors,
        }),
    })
}

pub(super) fn execute_skills_install_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.install payload must be an object".to_owned())?;
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
    let security_decision = parse_skill_security_decision(payload, "skills.install")?;
    let explicit_skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim);
    let source_skill_id = payload
        .get("source_skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if raw_path.is_some() && bundled_skill_id.is_some() {
        return Err(
            "skills.install accepts either payload.path or payload.bundled_skill_id, not both"
                .to_owned(),
        );
    }
    if raw_path.is_none() && bundled_skill_id.is_none() {
        return Err("skills.install requires payload.path or payload.bundled_skill_id".to_owned());
    }

    require_enabled_runtime_policy(config)?;

    let install_root = resolve_install_root(config);
    fs::create_dir_all(&install_root).map_err(|error| {
        format!(
            "failed to create skills install root {}: {error}",
            install_root.display()
        )
    })?;
    let mut install_security_scan = None;
    let mut security_approval_used = false;
    let (skill_id, display_name, summary, source_kind, source_path, incoming_root, digest) =
        if let Some(bundled_skill_id) = bundled_skill_id {
            if explicit_skill_id
                .and_then(|value| (!value.is_empty()).then_some(value))
                .is_some()
            {
                return Err(
                    "skills.install cannot override payload.skill_id when payload.bundled_skill_id is used"
                        .to_owned(),
                );
            }
            let bundled =
                super::bundled_skills::bundled_skill(bundled_skill_id).ok_or_else(|| {
                    format!("skills.install does not recognize bundled skill `{bundled_skill_id}`")
                })?;
            let bundled_markdown = super::bundled_skills::bundled_skill_markdown(&bundled)?;
            let bundled_dir =
                super::bundled_skills::bundled_skill_dir(&bundled).map_err(|error| {
                    format!("failed to resolve bundled skill `{bundled_skill_id}`: {error}")
                })?;
            let skill_id = normalize_skill_id(bundled.skill_id)?;
            let display_name = derive_skill_display_name(bundled_markdown, bundled.skill_id);
            let summary = derive_skill_summary(bundled_markdown);
            let incoming_root = unique_managed_install_transition_path(
                &install_root,
                skill_id.as_str(),
                "incoming",
            )?;
            let mut incoming_cleanup = ScopedDirCleanup::new(Some(incoming_root.clone()));
            fs::create_dir_all(&incoming_root).map_err(|error| {
                format!(
                    "failed to create bundled skill staging directory {}: {error}",
                    incoming_root.display()
                )
            })?;
            copy_embedded_dir_recursive(bundled_dir, &incoming_root)?;
            let digest = digest_embedded_dir(bundled_dir);
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
                    "skills.install internal error: missing path after payload validation"
                        .to_owned(),
                );
            };
            let source_path = super::file::resolve_safe_file_path_with_config(raw_path, config)?;
            let source_metadata = fs::symlink_metadata(&source_path).map_err(|error| {
                format!(
                    "failed to inspect skill source {}: {error}",
                    source_path.display()
                )
            })?;
            let source_file_type = source_metadata.file_type();
            if source_file_type.is_symlink() {
                return Err(format!(
                    "skill source {} cannot be a symlink",
                    source_path.display()
                ));
            }

            let (skill_root, source_kind, cleanup_root) = if source_file_type.is_dir() {
                let skill_root = resolve_skill_root(&source_path, source_skill_id)?;
                (skill_root, "directory", None)
            } else if source_file_type.is_file() {
                let (staging_root, skill_root) =
                    extract_archive_to_staging(&source_path, &install_root, source_skill_id)?;
                (skill_root, "archive", Some(staging_root))
            } else {
                return Err(format!(
                    "skill source {} must be a directory or a regular file",
                    source_path.display()
                ));
            };
            let _cleanup_root = ScopedDirCleanup::new(cleanup_root);
            let security_scan = scan_external_skill_tree(&skill_root)?;
            let security_scan_payload = serde_json::to_value(&security_scan)
                .map_err(|error| format!("serialize skill security scan failed: {error}"))?;
            if security_scan.requires_approval() {
                match security_decision {
                    Some(ExternalSkillSecurityDecision::ApproveOnce) => {
                        security_approval_used = true;
                    }
                    Some(ExternalSkillSecurityDecision::Deny) => {
                        return Err(format!(
                            "skill installation cancelled by payload.security_decision=deny after {} security findings",
                            security_scan.findings.len()
                        ));
                    }
                    None => {
                        return Ok(ToolCoreOutcome {
                            status: "needs_approval".to_owned(),
                            payload: json!({
                                "adapter": "core-tools",
                                "tool_name": request.tool_name,
                                "action": "install",
                                "source_path": source_path.display().to_string(),
                                "allowed_decisions": ["approve_once", "deny"],
                                "security_scan": security_scan_payload,
                                "security_approval_used": false,
                            }),
                        });
                    }
                }
            }
            install_security_scan = Some(security_scan_payload);
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
                    "failed to create skill destination {}: {error}",
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
            let digest = hex::encode(Sha256::digest(installed_skill_markdown.as_bytes()));
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
            "skill `{skill_id}` is already installed; pass payload.replace=true to replace it"
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
            "failed to activate managed skill install {}: {error}",
            destination_root.display()
        ));
    }

    if let Err(error) = persist_installed_skill_index(&install_root, &mut index) {
        let mut rollback_notes = vec![format!("failed to update skills index: {error}")];

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
        remove_external_skill_path(&backup_root).map_err(|error| {
            format!(
                "failed to remove replaced skill backup {}: {error}",
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
            "security_scan": install_security_scan,
            "security_approval_used": security_approval_used,
        }),
    })
}

#[cfg(test)]
pub(super) fn execute_skills_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    require_enabled_runtime_policy(config)?;
    execute_skills_list_for_audience(request.tool_name, config, SkillAudience::Model)
}

#[cfg(test)]
pub(super) fn execute_skills_inspect_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.inspect payload must be an object".to_owned())?;
    let skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "skills.inspect requires payload.skill_id".to_owned())?;

    require_enabled_runtime_policy(config)?;
    execute_skills_inspect_for_audience(request.tool_name, config, skill_id, SkillAudience::Model)
}

pub(crate) fn execute_skills_list_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_list_for_audience("skills.list".to_owned(), config, SkillAudience::Operator)
}

pub(crate) fn execute_skills_search_with_config(
    query: &str,
    limit: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_search_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.search".to_owned(),
            payload: json!({
                "query": query,
                "limit": limit,
            }),
        },
        config,
    )
}

pub(crate) fn execute_skills_recommend_with_config(
    query: &str,
    limit: usize,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_recommend_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.recommend".to_owned(),
            payload: json!({
                "query": query,
                "limit": limit,
            }),
        },
        config,
    )
}

pub(crate) fn execute_skills_inspect_with_config(
    skill_id: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_inspect_for_audience(
        "skills.inspect".to_owned(),
        config,
        skill_id,
        SkillAudience::Operator,
    )
}

pub(crate) fn execute_skills_fetch_with_config(
    reference: &str,
    save_as: Option<&str>,
    max_bytes: Option<usize>,
    approval_granted: bool,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let mut payload = Map::new();
    payload.insert("url".to_owned(), json!(reference));
    if let Some(save_as) = save_as {
        payload.insert("save_as".to_owned(), json!(save_as));
    }
    if let Some(max_bytes) = max_bytes {
        payload.insert("max_bytes".to_owned(), json!(max_bytes));
    }
    if approval_granted {
        payload.insert("approval_granted".to_owned(), json!(true));
    }

    execute_skills_fetch_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.fetch".to_owned(),
            payload: Value::Object(payload),
        },
        config,
    )
}

pub(crate) fn execute_skills_install_with_config(
    path: Option<&str>,
    bundled_skill_id: Option<&str>,
    skill_id: Option<&str>,
    source_skill_id: Option<&str>,
    approve_security_once: bool,
    replace: bool,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let mut payload = Map::new();
    if let Some(path) = path {
        payload.insert("path".to_owned(), json!(path));
    }
    if let Some(bundled_skill_id) = bundled_skill_id {
        payload.insert("bundled_skill_id".to_owned(), json!(bundled_skill_id));
    }
    if let Some(skill_id) = skill_id {
        payload.insert("skill_id".to_owned(), json!(skill_id));
    }
    if let Some(source_skill_id) = source_skill_id {
        payload.insert("source_skill_id".to_owned(), json!(source_skill_id));
    }
    if approve_security_once {
        payload.insert("security_decision".to_owned(), json!("approve_once"));
    }
    payload.insert("replace".to_owned(), json!(replace));

    execute_skills_install_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.install".to_owned(),
            payload: Value::Object(payload),
        },
        config,
    )
}

pub(crate) fn execute_skills_remove_with_config(
    skill_id: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_remove_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.remove".to_owned(),
            payload: json!({
                "skill_id": skill_id,
            }),
        },
        config,
    )
}

pub(crate) fn execute_skills_policy_get_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_policy_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.policy".to_owned(),
            payload: json!({
                "action": "get",
            }),
        },
        config,
    )
}

pub(crate) fn execute_skills_policy_set_with_config(
    enabled: Option<bool>,
    require_download_approval: Option<bool>,
    allowed_domains: Option<BTreeSet<String>>,
    blocked_domains: Option<BTreeSet<String>>,
    policy_update_approved: bool,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let mut payload = Map::new();
    payload.insert("action".to_owned(), json!("set"));
    payload.insert(
        "policy_update_approved".to_owned(),
        json!(policy_update_approved),
    );
    if let Some(enabled) = enabled {
        payload.insert("enabled".to_owned(), json!(enabled));
    }
    if let Some(require_download_approval) = require_download_approval {
        payload.insert(
            "require_download_approval".to_owned(),
            json!(require_download_approval),
        );
    }
    if let Some(allowed_domains) = allowed_domains {
        payload.insert("allowed_domains".to_owned(), json!(allowed_domains));
    }
    if let Some(blocked_domains) = blocked_domains {
        payload.insert("blocked_domains".to_owned(), json!(blocked_domains));
    }

    execute_skills_policy_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.policy".to_owned(),
            payload: Value::Object(payload),
        },
        config,
    )
}

pub(crate) fn execute_skills_policy_reset_with_config(
    policy_update_approved: bool,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_policy_tool_with_config(
        ToolCoreRequest {
            tool_name: "skills.policy".to_owned(),
            payload: json!({
                "action": "reset",
                "policy_update_approved": policy_update_approved,
            }),
        },
        config,
    )
}

pub(super) fn execute_skills_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_discovery_tool_with_config(request, config, SkillDiscoveryMode::Search)
}

pub(super) fn execute_skills_recommend_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_skills_discovery_tool_with_config(request, config, SkillDiscoveryMode::Recommend)
}

pub(super) fn execute_skills_remove_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "skills.remove payload must be an object".to_owned())?;
    let skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "skills.remove requires payload.skill_id".to_owned())?;

    require_enabled_runtime_policy(config)?;

    let install_root = resolve_install_root(config);
    let mut index = load_installed_skill_index(&install_root)?;
    let removed = remove_installed_skill_from_index(&install_root, &mut index, skill_id)?;
    if !removed {
        return Err(format!("skill `{skill_id}` is not installed"));
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

// Managed-skill runtime policy, fetch/install flow, and bootstrap persistence live in a dedicated ownership slice.
include!("skills_runtime.rs");

pub(crate) fn installed_managed_skill_ids_for_bootstrap(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<BTreeSet<String>, String> {
    let policy = resolve_effective_policy(config)?;
    if !policy.enabled {
        return Ok(BTreeSet::new());
    }
    let install_root = resolve_install_root(config);
    let index = load_installed_skill_index(&install_root)?;
    Ok(index
        .skills
        .into_iter()
        .filter(|entry| entry.active)
        .map(|entry| entry.skill_id)
        .collect())
}

// Discovery, catalog projection, and model-visible context live in a dedicated ownership slice.
include!("skills_discovery.rs");

#[cfg(test)]
mod tests {
    include!("skills_tests.rs");
}
