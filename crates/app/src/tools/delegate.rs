use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use loongclaw_contracts::ToolCoreOutcome;
use serde_json::{Value, json};

use crate::conversation::{ConstrainedSubagentIdentity, DelegateBuiltinProfile};

use crate::config::DelegateToolConfig;
use crate::conversation::ConstrainedSubagentIsolation;
use crate::tools::runtime_config::{
    BrowserRuntimeNarrowing, ToolRuntimeNarrowing, WebFetchRuntimeNarrowing,
};

use super::payload::{optional_payload_string, required_payload_string};

#[cfg(test)]
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

const DELEGATE_PROFILE_VALID_VALUES: &str = "research, plan, verify";
const DELEGATE_ISOLATION_VALID_VALUES: &str = "shared, worktree";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateRequest {
    pub task: String,
    pub label: Option<String>,
    pub specialization: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub isolation: ConstrainedSubagentIsolation,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedDelegatePolicy {
    pub label: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub isolation: ConstrainedSubagentIsolation,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
    pub runtime_narrowing: ToolRuntimeNarrowing,
}

#[cfg(test)]
pub(crate) fn parse_delegate_request(payload: &Value) -> Result<DelegateRequest, String> {
    parse_delegate_request_with_default_timeout(payload, 0)
}

pub(crate) fn parse_delegate_request_with_default_timeout(
    payload: &Value,
    _default_timeout_seconds: u64,
) -> Result<DelegateRequest, String> {
    let task = required_payload_string(payload, "task", "delegate tool")?;
    let label = optional_payload_string(payload, "label");
    let specialization = optional_payload_string(payload, "specialization");
    let profile = match payload.get("profile") {
        Some(Value::String(raw_profile)) => Some(parse_delegate_profile(raw_profile)?),
        Some(value) => {
            return Err(format!(
                "invalid_delegate_profile: expected a string, got: {value}"
            ));
        }
        None => None,
    };
    let isolation = match payload.get("isolation") {
        Some(Value::String(raw_isolation)) => parse_delegate_isolation(raw_isolation)?,
        Some(value) => {
            return Err(format!(
                "invalid_delegate_isolation: expected a string, got: {value}"
            ));
        }
        None => ConstrainedSubagentIsolation::default(),
    };
    let timeout_seconds = parse_delegate_timeout_seconds(payload)?;

    Ok(DelegateRequest {
        task,
        label,
        specialization,
        timeout_seconds,
        profile,
        isolation,
    })
}

#[cfg(test)]
pub(crate) fn normalize_delegate_request(
    task: &str,
    label: Option<&str>,
    specialization: Option<&str>,
    timeout_seconds: Option<u64>,
    default_timeout_seconds: u64,
) -> Result<DelegateRequest, String> {
    let mut payload = json!({
        "task": task,
    });
    if let Some(label) = label
        && let Some(payload_object) = payload.as_object_mut()
    {
        payload_object.insert("label".to_owned(), json!(label));
    }
    if let Some(specialization) = specialization
        && let Some(payload_object) = payload.as_object_mut()
    {
        payload_object.insert("specialization".to_owned(), json!(specialization));
    }
    if let Some(timeout_seconds) = timeout_seconds
        && let Some(payload_object) = payload.as_object_mut()
    {
        payload_object.insert("timeout_seconds".to_owned(), json!(timeout_seconds));
    }
    if timeout_seconds.is_none()
        && default_timeout_seconds > 0
        && let Some(payload_object) = payload.as_object_mut()
    {
        payload_object.insert("timeout_seconds".to_owned(), json!(default_timeout_seconds));
    }
    parse_delegate_request_with_default_timeout(&payload, default_timeout_seconds)
}

fn parse_delegate_timeout_seconds(payload: &Value) -> Result<Option<u64>, String> {
    let Some(value) = payload.get("timeout_seconds") else {
        return Ok(None);
    };
    let timeout_seconds = value.as_u64().ok_or_else(|| {
        format!("invalid_timeout_seconds: expected a positive integer, got: {value}")
    })?;
    if timeout_seconds == 0 {
        return Err("invalid_timeout_seconds: expected a positive integer".to_owned());
    }

    Ok(Some(timeout_seconds))
}

pub(crate) fn subagent_identity_for_delegate_request(
    request: &DelegateRequest,
) -> Option<ConstrainedSubagentIdentity> {
    let identity = ConstrainedSubagentIdentity {
        nickname: request.label.clone(),
        specialization: request.specialization.clone(),
    };
    (!identity.is_empty()).then_some(identity)
}

pub(crate) fn next_delegate_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("delegate:{now_ms:x}{counter:x}")
}

pub(crate) fn resolve_delegate_policy(
    request: &DelegateRequest,
    config: &DelegateToolConfig,
) -> ResolvedDelegatePolicy {
    let profile = request.profile;
    let profile_runtime_narrowing = profile
        .map(delegate_profile_runtime_narrowing)
        .unwrap_or_default();
    let timeout_seconds = request.timeout_seconds.unwrap_or_else(|| {
        profile.map_or(config.timeout_seconds, |profile| {
            profile
                .default_timeout_seconds()
                .min(config.timeout_seconds)
        })
    });
    let timeout_seconds = timeout_seconds.min(config.timeout_seconds);
    let label = request
        .label
        .clone()
        .or_else(|| profile.map(|profile| profile.default_label().to_owned()));

    let child_tool_allowlist = match profile {
        Some(profile) => {
            let profile_allowlist = delegate_profile_child_tool_allowlist(profile);
            if config.child_tool_allowlist.is_empty() {
                profile_allowlist
            } else {
                profile_allowlist
                    .into_iter()
                    .filter(|tool_name| {
                        config
                            .child_tool_allowlist
                            .iter()
                            .any(|allowed_tool_name| allowed_tool_name == tool_name)
                    })
                    .collect()
            }
        }
        None => config.child_tool_allowlist.clone(),
    };
    let allow_shell_in_child = profile.map_or(config.allow_shell_in_child, |profile| {
        config.allow_shell_in_child && profile.allows_shell_in_child()
    });

    ResolvedDelegatePolicy {
        label,
        profile,
        isolation: request.isolation,
        timeout_seconds,
        allow_shell_in_child,
        child_tool_allowlist,
        runtime_narrowing: merge_runtime_narrowing(
            config.child_runtime.runtime_narrowing(),
            profile_runtime_narrowing,
        ),
    }
}

pub(crate) fn delegate_success_outcome(
    child_session_id: String,
    _parent_session_id: Option<String>,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    final_output: String,
    turn_count: usize,
    duration_ms: u64,
) -> ToolCoreOutcome {
    let mut payload = json!({
        "child_session_id": child_session_id,
        "label": label,
        "final_output": final_output,
        "turn_count": turn_count,
        "duration_ms": duration_ms,
    });
    if let Some(profile) = profile
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("profile".to_owned(), json!(profile.as_str()));
    }
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload,
    }
}

pub(crate) fn delegate_async_queued_outcome(
    child_session_id: String,
    _parent_session_id: Option<String>,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    timeout_seconds: u64,
) -> ToolCoreOutcome {
    let mut payload = json!({
        "child_session_id": child_session_id,
        "label": label,
        "mode": "async",
        "state": "queued",
        "timeout_seconds": timeout_seconds,
    });
    if let Some(profile) = profile
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("profile".to_owned(), json!(profile.as_str()));
    }
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload,
    }
}

pub(crate) fn delegate_timeout_outcome(
    child_session_id: String,
    _parent_session_id: Option<String>,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    duration_ms: u64,
) -> ToolCoreOutcome {
    let mut payload = json!({
        "child_session_id": child_session_id,
        "label": label,
        "duration_ms": duration_ms,
        "error": "delegate_timeout",
    });
    if let Some(profile) = profile
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("profile".to_owned(), json!(profile.as_str()));
    }
    ToolCoreOutcome {
        status: "timeout".to_owned(),
        payload,
    }
}

pub(crate) fn delegate_error_outcome(
    child_session_id: String,
    _parent_session_id: Option<String>,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    error: String,
    duration_ms: u64,
) -> ToolCoreOutcome {
    let mut payload = json!({
        "child_session_id": child_session_id,
        "label": label,
        "duration_ms": duration_ms,
        "error": error,
    });
    if let Some(profile) = profile
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("profile".to_owned(), json!(profile.as_str()));
    }
    ToolCoreOutcome {
        status: "error".to_owned(),
        payload,
    }
}

fn parse_delegate_profile(raw: &str) -> Result<DelegateBuiltinProfile, String> {
    match raw.trim() {
        "research" => Ok(DelegateBuiltinProfile::Research),
        "plan" => Ok(DelegateBuiltinProfile::Plan),
        "verify" => Ok(DelegateBuiltinProfile::Verify),
        other => Err(format!(
            "invalid_delegate_profile: `{other}` is not supported; expected one of: {DELEGATE_PROFILE_VALID_VALUES}"
        )),
    }
}

fn parse_delegate_isolation(raw: &str) -> Result<ConstrainedSubagentIsolation, String> {
    match raw.trim() {
        "shared" => Ok(ConstrainedSubagentIsolation::Shared),
        "worktree" => Ok(ConstrainedSubagentIsolation::Worktree),
        other => Err(format!(
            "invalid_delegate_isolation: `{other}` is not supported; expected one of: {DELEGATE_ISOLATION_VALID_VALUES}"
        )),
    }
}

fn delegate_profile_child_tool_allowlist(profile: DelegateBuiltinProfile) -> Vec<String> {
    match profile {
        DelegateBuiltinProfile::Research => vec![
            "file.read".to_owned(),
            "web.fetch".to_owned(),
            "web.search".to_owned(),
            "browser.open".to_owned(),
            "browser.extract".to_owned(),
        ],
        DelegateBuiltinProfile::Plan => vec![
            "file.read".to_owned(),
            "web.fetch".to_owned(),
            "web.search".to_owned(),
        ],
        DelegateBuiltinProfile::Verify => {
            vec!["file.read".to_owned(), "web.fetch".to_owned()]
        }
    }
}

fn delegate_profile_runtime_narrowing(profile: DelegateBuiltinProfile) -> ToolRuntimeNarrowing {
    match profile {
        DelegateBuiltinProfile::Research => ToolRuntimeNarrowing {
            browser: BrowserRuntimeNarrowing {
                max_sessions: Some(1),
                max_links: Some(20),
                max_text_chars: Some(4_000),
            },
            web_fetch: WebFetchRuntimeNarrowing::default(),
        },
        DelegateBuiltinProfile::Plan => ToolRuntimeNarrowing {
            browser: BrowserRuntimeNarrowing::default(),
            web_fetch: WebFetchRuntimeNarrowing {
                timeout_seconds: Some(10),
                max_bytes: Some(512 * 1024),
                ..WebFetchRuntimeNarrowing::default()
            },
        },
        DelegateBuiltinProfile::Verify => ToolRuntimeNarrowing::default(),
    }
}

fn merge_runtime_narrowing(
    base: ToolRuntimeNarrowing,
    overlay: ToolRuntimeNarrowing,
) -> ToolRuntimeNarrowing {
    ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: merge_option_min(base.browser.max_sessions, overlay.browser.max_sessions),
            max_links: merge_option_min(base.browser.max_links, overlay.browser.max_links),
            max_text_chars: merge_option_min(
                base.browser.max_text_chars,
                overlay.browser.max_text_chars,
            ),
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: match (
                base.web_fetch.allow_private_hosts,
                overlay.web_fetch.allow_private_hosts,
            ) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                (Some(value), None) | (None, Some(value)) => Some(value),
                (None, None) => None,
            },
            enforce_allowed_domains: base.web_fetch.enforces_allowed_domains()
                || overlay.web_fetch.enforces_allowed_domains(),
            allowed_domains: merge_allowed_domains(
                &base.web_fetch.allowed_domains,
                &overlay.web_fetch.allowed_domains,
            ),
            blocked_domains: base
                .web_fetch
                .blocked_domains
                .union(&overlay.web_fetch.blocked_domains)
                .cloned()
                .collect(),
            timeout_seconds: merge_option_min(
                base.web_fetch.timeout_seconds,
                overlay.web_fetch.timeout_seconds,
            ),
            max_bytes: merge_option_min(base.web_fetch.max_bytes, overlay.web_fetch.max_bytes),
            max_redirects: merge_option_min(
                base.web_fetch.max_redirects,
                overlay.web_fetch.max_redirects,
            ),
        },
    }
}

fn merge_allowed_domains(
    base: &std::collections::BTreeSet<String>,
    overlay: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    match (base.is_empty(), overlay.is_empty()) {
        (true, true) => std::collections::BTreeSet::new(),
        (false, true) => base.clone(),
        (true, false) => overlay.clone(),
        (false, false) => base.intersection(overlay).cloned().collect(),
    }
}

fn merge_option_min<T: Ord + Copy>(base: Option<T>, overlay: Option<T>) -> Option<T> {
    match (base, overlay) {
        (Some(base), Some(overlay)) => Some(base.min(overlay)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_delegate_request_requires_task() {
        let error =
            parse_delegate_request(&json!({})).expect_err("missing task should be rejected");
        assert!(error.contains("payload.task"), "error: {error}");
    }

    #[test]
    fn parse_delegate_request_uses_defaults() {
        let request = parse_delegate_request(&json!({
            "task": "research"
        }))
        .expect("delegate request");
        assert_eq!(request.task, "research");
        assert_eq!(request.label, None);
        assert_eq!(request.timeout_seconds, None);
        assert_eq!(request.profile, None);
        assert_eq!(request.isolation, ConstrainedSubagentIsolation::Shared);
    }

    #[test]
    fn normalize_delegate_request_trims_cli_inputs() {
        let request = normalize_delegate_request(
            "  research  ",
            Some("  release-check  "),
            Some("  reviewer  "),
            None,
            DEFAULT_TIMEOUT_SECONDS,
        )
        .expect("delegate request");
        assert_eq!(request.task, "research");
        assert_eq!(request.label.as_deref(), Some("release-check"));
        assert_eq!(request.timeout_seconds, Some(DEFAULT_TIMEOUT_SECONDS));
    }

    #[test]
    fn parse_delegate_request_includes_optional_specialization() {
        let request = parse_delegate_request(&json!({
            "task": "research",
            "label": "child",
            "specialization": "reviewer"
        }))
        .expect("delegate request");
        assert_eq!(request.specialization.as_deref(), Some("reviewer"));
        assert_eq!(
            subagent_identity_for_delegate_request(&request),
            Some(ConstrainedSubagentIdentity {
                nickname: Some("child".to_owned()),
                specialization: Some("reviewer".to_owned())
            })
        );
    }

    #[test]
    fn delegate_session_ids_use_expected_prefix() {
        let session_id = next_delegate_session_id();
        assert!(session_id.starts_with("delegate:"));
    }

    #[test]
    fn parse_delegate_request_accepts_builtin_profile() {
        let request = parse_delegate_request(&json!({
            "task": "investigate the bug",
            "profile": "research"
        }))
        .expect("delegate request");

        assert_eq!(request.profile, Some(DelegateBuiltinProfile::Research));
    }

    #[test]
    fn parse_delegate_request_accepts_isolation_mode() {
        let request = parse_delegate_request(&json!({
            "task": "prepare isolated edits",
            "isolation": "worktree"
        }))
        .expect("delegate request");

        assert_eq!(request.isolation, ConstrainedSubagentIsolation::Worktree);
    }

    #[test]
    fn parse_delegate_request_rejects_unknown_profile() {
        let error = parse_delegate_request(&json!({
            "task": "investigate the bug",
            "profile": "custom"
        }))
        .expect_err("unknown delegate profile should be rejected");

        assert!(error.contains("invalid_delegate_profile"), "error: {error}");
    }

    #[test]
    fn parse_delegate_request_rejects_invalid_timeout() {
        let error = parse_delegate_request(&json!({
            "task": "investigate the bug",
            "timeout_seconds": "60"
        }))
        .expect_err("invalid timeout should be rejected");

        assert!(error.contains("invalid_timeout_seconds"), "error: {error}");
    }

    #[test]
    fn parse_delegate_request_rejects_non_string_profile() {
        let error = parse_delegate_request(&json!({
            "task": "review the patch",
            "profile": 1
        }))
        .expect_err("non-string profile should be rejected");

        assert!(error.contains("invalid_delegate_profile"), "error: {error}");
    }

    #[test]
    fn parse_delegate_request_rejects_non_string_isolation() {
        let error = parse_delegate_request(&json!({
            "task": "review the patch",
            "isolation": 1
        }))
        .expect_err("non-string isolation should be rejected");

        assert!(
            error.contains("invalid_delegate_isolation"),
            "error: {error}"
        );
    }

    #[test]
    fn resolve_delegate_policy_uses_profile_defaults_without_widening_operator_allowlist() {
        let request = DelegateRequest {
            task: "review the patch".to_owned(),
            label: None,
            specialization: None,
            profile: Some(DelegateBuiltinProfile::Verify),
            isolation: ConstrainedSubagentIsolation::Shared,
            timeout_seconds: None,
        };
        let config = DelegateToolConfig {
            allow_shell_in_child: true,
            timeout_seconds: 60,
            child_tool_allowlist: vec!["file.read".to_owned(), "file.write".to_owned()],
            ..DelegateToolConfig::default()
        };

        let policy = resolve_delegate_policy(&request, &config);

        assert_eq!(policy.label.as_deref(), Some("Verify"));
        assert_eq!(policy.profile, Some(DelegateBuiltinProfile::Verify));
        assert_eq!(policy.isolation, ConstrainedSubagentIsolation::Shared);
        assert_eq!(policy.timeout_seconds, 45);
        assert!(policy.allow_shell_in_child);
        assert_eq!(policy.child_tool_allowlist, vec!["file.read".to_owned()]);
    }

    #[test]
    fn resolve_delegate_policy_uses_profile_allowlist_when_operator_allowlist_is_empty() {
        let request = DelegateRequest {
            task: "research the bug".to_owned(),
            label: None,
            specialization: None,
            profile: Some(DelegateBuiltinProfile::Plan),
            isolation: ConstrainedSubagentIsolation::Shared,
            timeout_seconds: None,
        };
        let config = DelegateToolConfig {
            child_tool_allowlist: Vec::new(),
            ..DelegateToolConfig::default()
        };

        let policy = resolve_delegate_policy(&request, &config);

        assert_eq!(
            policy.child_tool_allowlist,
            vec![
                "file.read".to_owned(),
                "web.fetch".to_owned(),
                "web.search".to_owned(),
            ]
        );
    }

    #[test]
    fn resolve_delegate_policy_caps_explicit_timeout_at_config_max() {
        let request = DelegateRequest {
            task: "review the patch".to_owned(),
            label: None,
            specialization: None,
            profile: None,
            isolation: ConstrainedSubagentIsolation::Shared,
            timeout_seconds: Some(120),
        };
        let config = DelegateToolConfig {
            timeout_seconds: 45,
            ..DelegateToolConfig::default()
        };

        let policy = resolve_delegate_policy(&request, &config);

        assert_eq!(policy.timeout_seconds, 45);
    }
}
