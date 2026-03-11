use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const DEFAULT_DOWNLOAD_DIR_NAME: &str = "external-skills-downloads";
const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 5 * 1024 * 1024;
const HARD_MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

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

    let policy = resolve_effective_policy(config)?;
    if !policy.enabled {
        return Err(
            "external skills runtime is disabled; enable `external_skills.enabled = true` first"
                .to_owned(),
        );
    }

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

fn policy_payload(policy: &super::runtime_config::ExternalSkillsRuntimePolicy) -> Value {
    json!({
        "enabled": policy.enabled,
        "require_download_approval": policy.require_download_approval,
        "allowed_domains": policy.allowed_domains.iter().cloned().collect::<Vec<_>>(),
        "blocked_domains": policy.blocked_domains.iter().cloned().collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
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
}
