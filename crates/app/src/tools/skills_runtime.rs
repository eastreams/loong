fn policy_override_store() -> &'static RwLock<SkillsPolicyOverride> {
    SKILLS_POLICY_OVERRIDE.get_or_init(|| RwLock::new(SkillsPolicyOverride::default()))
}

fn policy_override_is_active() -> Result<bool, String> {
    let guard = policy_override_store()
        .read()
        .map_err(|error| format!("skills policy lock poisoned: {error}"))?;
    Ok(guard.has_values())
}

fn resolve_effective_policy(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<super::runtime_config::SkillsRuntimePolicy, String> {
    let override_state = policy_override_store()
        .read()
        .map_err(|error| format!("skills policy lock poisoned: {error}"))?;
    Ok(build_effective_policy(config, &override_state))
}

fn require_enabled_runtime_policy(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<super::runtime_config::SkillsRuntimePolicy, String> {
    let policy = resolve_effective_policy(config)?;
    if !policy.enabled {
        return Err("skills runtime is disabled; enable `skills.enabled = true` first".to_owned());
    }
    Ok(policy)
}

fn build_effective_policy(
    config: &super::runtime_config::ToolRuntimeConfig,
    override_state: &SkillsPolicyOverride,
) -> super::runtime_config::SkillsRuntimePolicy {
    let mut effective = config.skills.clone();
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

impl SkillsPolicyOverride {
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

fn remove_external_skill_path(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        return fs::remove_dir_all(path);
    }
    fs::remove_file(path)
}

fn parse_optional_bool(payload: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    let parsed = value
        .as_bool()
        .ok_or_else(|| format!("skills.policy payload.{key} must be a boolean"))?;
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
        .ok_or_else(|| format!("skills.fetch payload.{key} must be a non-empty string"))?;
    Ok(Some(parsed.to_owned()))
}

#[cfg(test)]
fn parse_required_query(payload: &Map<String, Value>, tool_name: &str) -> Result<String, String> {
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{tool_name} requires payload.query"))?;
    Ok(query.to_owned())
}

fn parse_required_external_skill_reference(
    payload: &Map<String, Value>,
    tool_name: &str,
) -> Result<String, String> {
    let reference = payload
        .get("reference")
        .or_else(|| payload.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{tool_name} requires payload.reference or payload.url"))?;
    Ok(reference.to_owned())
}

#[cfg(test)]
fn parse_optional_source_search_limit(
    payload: &Map<String, Value>,
    tool_name: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = payload.get("max_results") else {
        return Ok(None);
    };
    let raw_value = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.max_results must be an integer"))?;
    let parsed_value = usize::try_from(raw_value)
        .map_err(|error| format!("invalid {tool_name} payload.max_results: {error}"))?;
    Ok(Some(parsed_value))
}

#[cfg(test)]
fn parse_external_skill_search_sources(
    payload: &Map<String, Value>,
    tool_name: &str,
) -> Result<Vec<ExternalSkillSourceKind>, String> {
    let Some(value) = payload.get("sources") else {
        return Ok(default_external_skill_search_sources());
    };

    let items = value
        .as_array()
        .ok_or_else(|| format!("{tool_name} payload.sources must be an array"))?;
    if items.is_empty() {
        return Ok(default_external_skill_search_sources());
    }

    let mut seen = BTreeSet::new();
    let mut source_kinds = Vec::new();

    for item in items {
        let raw_source = item
            .as_str()
            .ok_or_else(|| format!("{tool_name} payload.sources must contain only strings"))?;
        let source_kind = parse_external_skill_source_kind(raw_source)
            .ok_or_else(|| format!("unsupported skills search source `{raw_source}`"))?;
        if seen.insert(source_kind.as_str().to_owned()) {
            source_kinds.push(source_kind);
        }
    }

    Ok(source_kinds)
}

fn serialize_external_skill_candidate(
    candidate: &ResolvedExternalSkillCandidate,
) -> Result<Value, String> {
    serde_json::to_value(candidate)
        .map_err(|error| format!("serialize external skill candidate failed: {error}"))
}

#[cfg(test)]
fn normalize_external_skill_search_results(
    expected_source_kind: ExternalSkillSourceKind,
    source_priority: usize,
    raw_results: &[Value],
) -> Result<Vec<Value>, String> {
    let mut normalized_results = Vec::new();

    for (result_rank, raw_result) in raw_results.iter().enumerate() {
        let result_url = raw_result
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(result_url) = result_url else {
            continue;
        };

        let candidate = match resolve_external_skill_candidate(result_url) {
            Ok(candidate) => candidate,
            Err(_) => continue,
        };
        if candidate.source_kind != expected_source_kind {
            continue;
        }

        let candidate_payload = serialize_external_skill_candidate(&candidate)?;
        let title = raw_result
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(candidate.display_name.as_str())
            .to_owned();
        let snippet = raw_result
            .get("snippet")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();

        let normalized_result = json!({
            "source_kind": expected_source_kind.as_str(),
            "source_priority": source_priority,
            "result_rank": result_rank,
            "title": title,
            "snippet": snippet,
            "candidate": candidate_payload,
        });
        normalized_results.push(normalized_result);
    }

    Ok(normalized_results)
}

#[cfg(test)]
fn search_result_ordering(left: &Value, right: &Value) -> Ordering {
    let left_source_priority = left
        .get("source_priority")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let right_source_priority = right
        .get("source_priority")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let left_result_rank = left
        .get("result_rank")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let right_result_rank = right
        .get("result_rank")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let left_reference = left
        .get("candidate")
        .and_then(|value| value.get("canonical_reference"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let right_reference = right
        .get("candidate")
        .and_then(|value| value.get("canonical_reference"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    left_source_priority
        .cmp(&right_source_priority)
        .then_with(|| left_result_rank.cmp(&right_result_rank))
        .then_with(|| left_reference.cmp(right_reference))
}

impl ExternalSkillDownloadPlanHttp for ReqwestExternalSkillDownloadPlanHttp<'_> {
    fn get_text(&self, url: &str) -> Result<String, String> {
        let validated_url =
            validate_external_skill_network_target(url, self.policy, "source resolution")?;
        let response =
            send_external_skill_get_request(self.client, &validated_url, "source resolution")?;
        response
            .text()
            .map_err(|error| format!("failed to read skills source response `{url}`: {error}"))
    }

    fn get_json(&self, url: &str) -> Result<Value, String> {
        let body = self.get_text(url)?;
        serde_json::from_str::<Value>(body.as_str())
            .map_err(|error| format!("failed to decode skills source JSON `{url}`: {error}"))
    }
}

fn ensure_external_skill_https_url(url: &str, operation: &str) -> Result<(), String> {
    let parsed_url =
        reqwest::Url::parse(url).map_err(|error| format!("invalid skills url `{url}`: {error}"))?;
    if parsed_url.scheme() != "https" {
        return Err(format!(
            "skills {operation} requires https url, got scheme `{}`",
            parsed_url.scheme()
        ));
    }
    Ok(())
}

fn validate_external_skill_network_target(
    url: &str,
    policy: &super::runtime_config::SkillsRuntimePolicy,
    operation: &str,
) -> Result<ValidatedExternalSkillUrl, String> {
    let parsed_url =
        reqwest::Url::parse(url).map_err(|error| format!("invalid skills url `{url}`: {error}"))?;
    ensure_external_skill_https_url(url, operation)?;
    let host = parsed_url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("skills url `{url}` has no host"))?;
    if let Some(rule) = first_matching_domain_rule(host.as_str(), &policy.blocked_domains) {
        return Err(format!(
            "skills {operation} blocked: host `{host}` matches blocked domain rule `{rule}`"
        ));
    }
    if !policy.allowed_domains.is_empty()
        && first_matching_domain_rule(host.as_str(), &policy.allowed_domains).is_none()
    {
        return Err(format!(
            "skills {operation} denied: host `{host}` is not in allowed_domains"
        ));
    }

    Ok(ValidatedExternalSkillUrl { parsed_url, host })
}

fn send_external_skill_get_request(
    client: &reqwest::blocking::Client,
    validated_url: &ValidatedExternalSkillUrl,
    operation: &str,
) -> Result<reqwest::blocking::Response, String> {
    let response = client
        .get(validated_url.parsed_url.clone())
        .send()
        .map_err(|error| {
            format!(
                "skills {operation} request failed for `{}`: {error}",
                validated_url.parsed_url
            )
        })?;

    if response.status().is_redirection() {
        return Err(format!(
            "skills {operation} rejected redirect response {} for `{}`",
            response.status(),
            validated_url.parsed_url
        ));
    }
    if !response.status().is_success() {
        return Err(format!(
            "skills {operation} returned non-success status {} for `{}`",
            response.status(),
            validated_url.parsed_url
        ));
    }

    Ok(response)
}

fn resolve_external_skill_download_plan(
    reference: &str,
    http: &impl ExternalSkillDownloadPlanHttp,
) -> Result<ExternalSkillDownloadPlan, String> {
    let candidate = resolve_external_skill_candidate(reference)?;
    resolve_external_skill_download_plan_from_candidate(candidate, http)
}

fn resolve_external_skill_download_plan_from_candidate(
    candidate: ResolvedExternalSkillCandidate,
    http: &impl ExternalSkillDownloadPlanHttp,
) -> Result<ExternalSkillDownloadPlan, String> {
    match candidate.source_kind {
        ExternalSkillSourceKind::DirectUrl => build_direct_url_download_plan(candidate),
        ExternalSkillSourceKind::Github => resolve_github_download_plan_from_candidate(
            candidate,
            ExternalSkillSourceKind::Github,
            None,
            http,
        ),
        ExternalSkillSourceKind::SkillsSh => {
            resolve_skills_sh_download_plan_from_candidate(candidate, http)
        }
        ExternalSkillSourceKind::Clawhub => {
            resolve_clawhub_download_plan_from_candidate(candidate, http)
        }
        ExternalSkillSourceKind::Npm => resolve_npm_download_plan_from_candidate(candidate, http),
    }
}

fn build_direct_url_download_plan(
    candidate: ResolvedExternalSkillCandidate,
) -> Result<ExternalSkillDownloadPlan, String> {
    let artifact_url = candidate
        .artifact_routes
        .first()
        .map(|route| route.url.clone())
        .unwrap_or_else(|| candidate.canonical_reference.clone());
    let selected_route_label = candidate
        .artifact_routes
        .first()
        .map(|route| route.label.clone());
    let selected_route_url = candidate
        .artifact_routes
        .first()
        .map(|route| route.url.clone());

    Ok(ExternalSkillDownloadPlan {
        source_kind: candidate.source_kind,
        candidate,
        artifact_url,
        source_skill_id: None,
        selected_route_label,
        selected_route_url,
    })
}

fn resolve_github_download_plan_from_candidate(
    candidate: ResolvedExternalSkillCandidate,
    source_kind: ExternalSkillSourceKind,
    source_skill_id: Option<String>,
    http: &impl ExternalSkillDownloadPlanHttp,
) -> Result<ExternalSkillDownloadPlan, String> {
    if let Some(route) = candidate.artifact_routes.first().cloned() {
        let artifact_url = route.url.clone();
        return Ok(ExternalSkillDownloadPlan {
            source_kind,
            candidate,
            artifact_url,
            source_skill_id,
            selected_route_label: Some(route.label.clone()),
            selected_route_url: Some(route.url),
        });
    }

    let metadata_url = candidate
        .metadata_url
        .as_deref()
        .ok_or_else(|| "GitHub candidate is missing metadata_url".to_owned())?;
    let metadata = http.get_json(metadata_url)?;
    let default_branch = extract_github_default_branch(&metadata)?;
    let (owner, repo) = github_owner_and_repo_from_candidate(&candidate)?;
    let artifact_url =
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/heads/{default_branch}");

    Ok(ExternalSkillDownloadPlan {
        source_kind,
        candidate,
        artifact_url: artifact_url.clone(),
        source_skill_id,
        selected_route_label: Some("github_default_branch_tarball".to_owned()),
        selected_route_url: Some(artifact_url),
    })
}

fn resolve_skills_sh_download_plan_from_candidate(
    candidate: ResolvedExternalSkillCandidate,
    http: &impl ExternalSkillDownloadPlanHttp,
) -> Result<ExternalSkillDownloadPlan, String> {
    let landing_url = candidate
        .landing_url
        .as_deref()
        .ok_or_else(|| "skills.sh candidate is missing landing_url".to_owned())?;
    let document = http.get_text(landing_url)?;
    let install_source = extract_skills_sh_install_source(document.as_str())?;
    let github_candidate = resolve_external_skill_candidate(install_source.github_url.as_str())?;
    let github_plan = resolve_github_download_plan_from_candidate(
        github_candidate,
        ExternalSkillSourceKind::SkillsSh,
        Some(install_source.source_skill_id.clone()),
        http,
    )?;

    Ok(ExternalSkillDownloadPlan {
        source_kind: ExternalSkillSourceKind::SkillsSh,
        candidate,
        artifact_url: github_plan.artifact_url,
        source_skill_id: Some(install_source.source_skill_id),
        selected_route_label: Some("skills_sh_primary".to_owned()),
        selected_route_url: Some("https://skills.sh".to_owned()),
    })
}

fn resolve_clawhub_download_plan_from_candidate(
    candidate: ResolvedExternalSkillCandidate,
    http: &impl ExternalSkillDownloadPlanHttp,
) -> Result<ExternalSkillDownloadPlan, String> {
    let landing_url = candidate
        .landing_url
        .as_deref()
        .ok_or_else(|| "clawhub candidate is missing landing_url".to_owned())?;
    let source_skill_id = last_url_path_segment(landing_url);
    let mut route_errors = Vec::new();

    let routes = candidate.endpoint_routes.clone();
    for route in routes {
        let route_landing_url = rewrite_landing_url_for_route(landing_url, route.url.as_str())?;
        let document = match http.get_text(route_landing_url.as_str()) {
            Ok(document) => document,
            Err(error) => {
                route_errors.push(format!("{}: {error}", route.url));
                continue;
            }
        };
        let artifact_url =
            match extract_clawhub_download_url(document.as_str(), route_landing_url.as_str()) {
                Ok(artifact_url) => artifact_url,
                Err(error) => {
                    route_errors.push(format!("{}: {error}", route.url));
                    continue;
                }
            };
        return Ok(ExternalSkillDownloadPlan {
            source_kind: ExternalSkillSourceKind::Clawhub,
            candidate,
            artifact_url,
            source_skill_id,
            selected_route_label: Some(route.label),
            selected_route_url: Some(route.url),
        });
    }

    let rendered_errors = route_errors.join("; ");
    Err(format!(
        "failed to resolve clawhub download artifact from `{landing_url}`: {rendered_errors}"
    ))
}

fn resolve_npm_download_plan_from_candidate(
    candidate: ResolvedExternalSkillCandidate,
    http: &impl ExternalSkillDownloadPlanHttp,
) -> Result<ExternalSkillDownloadPlan, String> {
    let metadata_url = candidate
        .metadata_url
        .as_deref()
        .ok_or_else(|| "npm candidate is missing metadata_url".to_owned())?;
    let metadata = http.get_json(metadata_url)?;
    let artifact_url = extract_npm_tarball_url(&metadata)?;

    Ok(ExternalSkillDownloadPlan {
        source_kind: ExternalSkillSourceKind::Npm,
        candidate,
        artifact_url: artifact_url.clone(),
        source_skill_id: None,
        selected_route_label: Some("npm_registry".to_owned()),
        selected_route_url: Some(artifact_url),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillsShInstallSource {
    github_url: String,
    source_skill_id: String,
}

fn extract_skills_sh_install_source(document: &str) -> Result<SkillsShInstallSource, String> {
    static SKILLS_SH_INSTALL_SOURCE_RE: OnceLock<Option<Regex>> = OnceLock::new();
    let regex = SKILLS_SH_INSTALL_SOURCE_RE.get_or_init(|| {
        Regex::new(
            r#"npx\s+skills\s+add\s+(https://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+)(?:\.git)?\s+--skill\s+([A-Za-z0-9._-]+)"#,
        )
        .ok()
    });
    let Some(regex) = regex.as_ref() else {
        return Err("internal error: skills.sh install source regex is invalid".to_owned());
    };
    let captures = regex
        .captures(document)
        .ok_or_else(|| "skills.sh page is missing a supported install command".to_owned())?;
    let github_url = captures
        .get(1)
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| "skills.sh install command is missing a GitHub repository".to_owned())?;
    let source_skill_id = captures
        .get(2)
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| "skills.sh install command is missing `--skill`".to_owned())?;

    Ok(SkillsShInstallSource {
        github_url,
        source_skill_id,
    })
}

fn extract_github_default_branch(metadata: &Value) -> Result<String, String> {
    metadata
        .get("default_branch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "GitHub repository metadata is missing `default_branch`".to_owned())
}

fn github_owner_and_repo_from_candidate(
    candidate: &ResolvedExternalSkillCandidate,
) -> Result<(String, String), String> {
    let raw_reference = candidate
        .canonical_reference
        .strip_prefix("github:")
        .ok_or_else(|| {
            format!(
                "candidate `{}` is not a normalized GitHub reference",
                candidate.canonical_reference
            )
        })?;
    let mut parts = raw_reference.split('/');
    let owner = parts
        .next()
        .ok_or_else(|| "GitHub reference is missing owner".to_owned())?;
    let repo = parts
        .next()
        .ok_or_else(|| "GitHub reference is missing repository".to_owned())?;
    Ok((owner.to_owned(), repo.to_owned()))
}

fn extract_npm_tarball_url(metadata: &Value) -> Result<String, String> {
    if let Some(tarball_url) = metadata
        .get("dist")
        .and_then(|value| value.get("tarball"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(tarball_url.to_owned());
    }

    let latest_version = metadata
        .get("dist-tags")
        .and_then(|value| value.get("latest"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "npm package metadata is missing `dist-tags.latest`".to_owned())?;
    metadata
        .get("versions")
        .and_then(|value| value.get(latest_version))
        .and_then(|value| value.get("dist"))
        .and_then(|value| value.get("tarball"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("npm package metadata is missing `versions.{latest_version}.dist.tarball`")
        })
}

fn extract_clawhub_download_url(document: &str, landing_url: &str) -> Result<String, String> {
    static CLAWHUB_DOWNLOAD_RE: OnceLock<Option<Regex>> = OnceLock::new();
    let regex = CLAWHUB_DOWNLOAD_RE
        .get_or_init(|| Regex::new(r#"href=["']([^"']*/api/v1/download[^"']*)["']"#).ok());
    let Some(regex) = regex.as_ref() else {
        return Err("internal error: clawhub download regex is invalid".to_owned());
    };
    let captures = regex
        .captures(document)
        .ok_or_else(|| "clawhub page is missing a download link".to_owned())?;
    let raw_href = captures
        .get(1)
        .map(|value| value.as_str())
        .ok_or_else(|| "clawhub download link is missing href".to_owned())?;
    let base_url = reqwest::Url::parse(landing_url)
        .map_err(|error| format!("invalid clawhub landing url `{landing_url}`: {error}"))?;
    let joined_url = base_url.join(raw_href).map_err(|error| {
        format!("failed to resolve clawhub download href `{raw_href}`: {error}")
    })?;
    Ok(joined_url.to_string())
}

fn rewrite_landing_url_for_route(
    landing_url: &str,
    route_base_url: &str,
) -> Result<String, String> {
    let base_url = reqwest::Url::parse(route_base_url)
        .map_err(|error| format!("invalid skills route `{route_base_url}`: {error}"))?;
    let landing_url = reqwest::Url::parse(landing_url)
        .map_err(|error| format!("invalid skills landing url `{landing_url}`: {error}"))?;
    let mut rewritten_url = base_url;
    rewritten_url.set_path(landing_url.path());
    rewritten_url.set_query(landing_url.query());
    rewritten_url.set_fragment(None);
    Ok(rewritten_url.to_string())
}

fn last_url_path_segment(url: &str) -> Option<String> {
    let parsed_url = reqwest::Url::parse(url).ok()?;
    let mut segments = parsed_url.path_segments()?;
    segments
        .rfind(|segment| !segment.is_empty())
        .map(str::to_owned)
}

fn parse_optional_domain_list(
    payload: &Map<String, Value>,
    key: &str,
) -> Result<Option<BTreeSet<String>>, String> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };

    let items = value
        .as_array()
        .ok_or_else(|| format!("skills.policy payload.{key} must be an array of strings"))?;

    let mut normalized = BTreeSet::new();
    for item in items {
        let raw = item
            .as_str()
            .ok_or_else(|| format!("skills.policy payload.{key} must contain only strings"))?;
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
        .ok_or_else(|| "skills.fetch payload.max_bytes must be an integer".to_owned())?;
    if parsed == 0 {
        return Err("skills.fetch payload.max_bytes must be >= 1".to_owned());
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

#[derive(Debug)]
struct StreamedDownload {
    path: PathBuf,
    bytes_downloaded: usize,
    sha256: String,
}

fn stream_download_to_unique_path<R: Read>(
    reader: &mut R,
    content_length: Option<u64>,
    max_bytes: usize,
    output_dir: &Path,
    filename: &str,
    surface_name: &str,
) -> Result<StreamedDownload, String> {
    const MAX_PERSIST_COLLISION_RETRIES: usize = 16;

    let mut budget = super::download_guard::ByteBudget::new(max_bytes);

    budget.reject_if_content_length_exceeds(content_length, surface_name)?;

    let mut target_path = unique_output_path(output_dir, filename);
    let mut temp_file = TempFileBuilder::new()
        .prefix(".download-")
        .tempfile_in(output_dir)
        .map_err(|error| {
            format!(
                "failed to create temporary download file in {}: {error}",
                output_dir.display()
            )
        })?;
    let mut writer = BufWriter::new(temp_file.as_file_mut());
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8_192];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {surface_name} body: {error}"))?;
        if read == 0 {
            break;
        }

        budget.try_consume(read, surface_name)?;
        let chunk = buffer
            .get(..read)
            .ok_or_else(|| format!("failed to slice {surface_name} buffer"))?;

        writer
            .write_all(chunk)
            .map_err(|error| format!("failed to write {surface_name} body: {error}"))?;
        hasher.update(chunk);
    }

    writer
        .flush()
        .map_err(|error| format!("failed to flush {surface_name} body: {error}"))?;
    drop(writer);

    // Claim the final name without clobbering a sibling download that won the
    // same derived filename race first.
    for attempt in 0..MAX_PERSIST_COLLISION_RETRIES {
        let persist_result = temp_file.persist_noclobber(&target_path);

        match persist_result {
            Ok(_) => {
                break;
            }
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
                let is_last_attempt = attempt + 1 == MAX_PERSIST_COLLISION_RETRIES;

                if is_last_attempt {
                    return Err(format!(
                        "failed to persist downloaded artifact {} after {} name collisions",
                        target_path.display(),
                        MAX_PERSIST_COLLISION_RETRIES
                    ));
                }

                temp_file = error.file;
                target_path = unique_output_path(output_dir, filename);
            }
            Err(error) => {
                return Err(format!(
                    "failed to persist downloaded artifact {}: {}",
                    target_path.display(),
                    error.error
                ));
            }
        }
    }

    let sha256 = hex::encode(hasher.finalize());

    Ok(StreamedDownload {
        path: target_path,
        bytes_downloaded: budget.consumed(),
        sha256,
    })
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

fn install_bundled_skill_for_bootstrap(
    install_root: &Path,
    index: &mut InstalledSkillIndex,
    skill_id: &str,
) -> Result<bool, String> {
    let bundled = super::bundled_skills::bundled_skill(skill_id).ok_or_else(|| {
        format!("startup bootstrap does not recognize bundled skill `{skill_id}`")
    })?;
    let bundled_markdown = super::bundled_skills::bundled_skill_markdown(&bundled)?;
    let bundled_dir = super::bundled_skills::bundled_skill_dir(&bundled).map_err(|error| {
        format!("failed to resolve bundled startup skill `{skill_id}`: {error}")
    })?;
    let normalized_skill_id = normalize_skill_id(bundled.skill_id)?;
    let display_name = derive_skill_display_name(bundled_markdown, bundled.skill_id);
    let summary = derive_skill_summary(bundled_markdown);
    let digest = digest_embedded_dir(bundled_dir);
    let destination_root = managed_skill_install_path(install_root, normalized_skill_id.as_str())?;
    let existing_entry = index
        .skills
        .iter()
        .find(|entry| entry.skill_id == normalized_skill_id)
        .cloned();
    let destination_ready = destination_root.join(DEFAULT_SKILL_FILENAME).is_file();
    if existing_entry
        .as_ref()
        .is_some_and(|entry| entry.sha256 == digest && entry.active)
        && destination_ready
    {
        return Ok(false);
    }

    let incoming_root = unique_managed_install_transition_path(
        install_root,
        normalized_skill_id.as_str(),
        "incoming",
    )?;
    let mut incoming_cleanup = ScopedDirCleanup::new(Some(incoming_root.clone()));
    fs::create_dir_all(&incoming_root).map_err(|error| {
        format!(
            "failed to create bundled startup skill staging directory {}: {error}",
            incoming_root.display()
        )
    })?;
    copy_embedded_dir_recursive(bundled_dir, &incoming_root)?;
    incoming_cleanup.disarm();

    let backup_root = if destination_root.exists() {
        Some(unique_managed_install_transition_path(
            install_root,
            normalized_skill_id.as_str(),
            "backup",
        )?)
    } else {
        None
    };
    if let Some(backup_root) = backup_root.as_ref() {
        fs::rename(&destination_root, backup_root).map_err(|error| {
            format!(
                "failed to stage previous startup skill install {} for replacement: {error}",
                destination_root.display()
            )
        })?;
    }

    if let Err(error) = fs::rename(&incoming_root, &destination_root) {
        if let Some(backup_root) = backup_root.as_ref() {
            fs::rename(backup_root, &destination_root).ok();
        }
        return Err(format!(
            "failed to activate bundled startup skill {}: {error}",
            destination_root.display()
        ));
    }

    let installed_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    index
        .skills
        .retain(|entry| entry.skill_id != normalized_skill_id);
    index.skills.push(InstalledSkillEntry {
        skill_id: normalized_skill_id,
        display_name,
        summary,
        source_kind: "bundled".to_owned(),
        source_path: bundled.source_path.to_owned(),
        install_path: destination_root.display().to_string(),
        skill_md_path: destination_root
            .join(DEFAULT_SKILL_FILENAME)
            .display()
            .to_string(),
        sha256: digest,
        installed_at_unix,
        active: true,
    });
    Ok(true)
}

fn remove_installed_skill_from_index(
    install_root: &Path,
    index: &mut InstalledSkillIndex,
    skill_id: &str,
) -> Result<bool, String> {
    let normalized_skill_id = normalize_skill_id(skill_id)?;
    let Some(position) = index
        .skills
        .iter()
        .position(|entry| entry.skill_id == normalized_skill_id)
    else {
        return Ok(false);
    };
    let entry = index.skills.remove(position);
    let install_path = PathBuf::from(entry.install_path);
    let expected_path = managed_skill_install_path(install_root, normalized_skill_id.as_str())?;
    if install_path.exists() {
        fs::remove_dir_all(&install_path).map_err(|error| {
            format!(
                "failed to remove installed skill {}: {error}",
                install_path.display()
            )
        })?;
    } else if expected_path.exists() {
        fs::remove_dir_all(&expected_path).map_err(|error| {
            format!(
                "failed to remove installed skill {}: {error}",
                expected_path.display()
            )
        })?;
    }
    Ok(true)
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
    if let Some(path) = config.skills.install_root.clone() {
        return path;
    }
    let root = config
        .file_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    root.join(DEFAULT_INSTALL_DIR_NAME)
}

fn resolve_skill_root(root: &Path, source_skill_id: Option<&str>) -> Result<PathBuf, String> {
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
        _ => select_skill_root_from_candidates(root, &candidates, source_skill_id),
    }
}

fn extract_archive_to_staging(
    archive_path: &Path,
    install_root: &Path,
    source_skill_id: Option<&str>,
) -> Result<(PathBuf, PathBuf), String> {
    let filename = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_tar_gzip_archive = filename.ends_with(".tgz") || filename.ends_with(".tar.gz");
    let is_zip_archive = filename.ends_with(".zip");
    if !is_tar_gzip_archive && !is_zip_archive {
        return Err(format!(
            "external skill archive {} must end with .tgz, .tar.gz, or .zip",
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
        if is_tar_gzip_archive {
            extract_tar_gzip_archive_to_staging(archive_path, &staging_root)?;
        } else {
            extract_zip_archive_to_staging(archive_path, &staging_root)?;
        }
        resolve_skill_root(&staging_root, source_skill_id)
    })();

    match extraction {
        Ok(skill_root) => Ok((staging_root, skill_root)),
        Err(error) => {
            fs::remove_dir_all(&staging_root).ok();
            Err(error)
        }
    }
}

fn extract_tar_gzip_archive_to_staging(
    archive_path: &Path,
    staging_root: &Path,
) -> Result<(), String> {
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
        entry.unpack_in(staging_root).map_err(|error| {
            format!(
                "failed to extract external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
    }
    Ok(())
}

fn extract_zip_archive_to_staging(archive_path: &Path, staging_root: &Path) -> Result<(), String> {
    let file = fs::File::open(archive_path).map_err(|error| {
        format!(
            "failed to open external skill archive {}: {error}",
            archive_path.display()
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        format!(
            "failed to read external skill archive {}: {error}",
            archive_path.display()
        )
    })?;

    for entry_index in 0..archive.len() {
        let mut entry = archive.by_index(entry_index).map_err(|error| {
            format!(
                "failed to inspect external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
        let unix_mode = entry.unix_mode();
        if zip_entry_is_symlink(unix_mode) {
            return Err(format!(
                "external skill archive {} cannot contain symlinks or hard links",
                archive_path.display()
            ));
        }

        let enclosed_name = entry.enclosed_name().ok_or_else(|| {
            format!(
                "external skill archive {} contains a path traversal entry",
                archive_path.display()
            )
        })?;
        let relative_path = enclosed_name.to_path_buf();
        let output_path = staging_root.join(relative_path);

        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                format!(
                    "failed to extract external skill archive {}: {error}",
                    archive_path.display()
                )
            })?;
            continue;
        }

        let Some(parent) = output_path.parent() else {
            return Err(format!(
                "failed to extract external skill archive {}: entry has no parent directory",
                archive_path.display()
            ));
        };
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to extract external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
        let mut output_file = fs::File::create(&output_path).map_err(|error| {
            format!(
                "failed to extract external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
        std::io::copy(&mut entry, &mut output_file).map_err(|error| {
            format!(
                "failed to extract external skill archive {}: {error}",
                archive_path.display()
            )
        })?;
    }

    Ok(())
}

fn zip_entry_is_symlink(unix_mode: Option<u32>) -> bool {
    let Some(unix_mode) = unix_mode else {
        return false;
    };
    let file_type_bits = unix_mode & 0o170000;
    file_type_bits == 0o120000
}

fn select_skill_root_from_candidates(
    root: &Path,
    candidates: &[PathBuf],
    source_skill_id: Option<&str>,
) -> Result<PathBuf, String> {
    let Some(source_skill_id) = source_skill_id else {
        return Err(format!(
            "external skill source {} contains multiple `{DEFAULT_SKILL_FILENAME}` roots; provide payload.source_skill_id or a more specific path",
            root.display()
        ));
    };

    let normalized_source_skill_id = normalize_skill_id(source_skill_id)?;
    let mut matching_roots = Vec::new();

    for candidate in candidates {
        let candidate_skill_id = resolve_installable_skill_id(candidate)?;
        if candidate_skill_id == normalized_source_skill_id {
            matching_roots.push(candidate.clone());
        }
    }

    match matching_roots.as_slice() {
        [single] => Ok(single.clone()),
        [] => Err(format!(
            "external skill source {} contains multiple `{DEFAULT_SKILL_FILENAME}` roots and none match payload.source_skill_id=`{normalized_source_skill_id}`",
            root.display()
        )),
        _ => Err(format!(
            "external skill source {} contains multiple `{DEFAULT_SKILL_FILENAME}` roots that match payload.source_skill_id=`{normalized_source_skill_id}`; provide a more specific path",
            root.display()
        )),
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
            let raw = raw_frontmatter.join("\n");
            if raw.trim().is_empty() {
                return Ok(SkillFrontmatter::default());
            }
            let parsed = parse_skill_frontmatter_yaml(raw.as_str())?;
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

fn parse_skill_frontmatter_yaml(raw: &str) -> Result<YamlValue, String> {
    match serde_yaml::from_str::<YamlValue>(raw) {
        Ok(parsed) => Ok(parsed),
        Err(original_error) => {
            let repaired = repair_skill_frontmatter_yaml(raw);
            if repaired == raw {
                return Err(format!("failed to parse YAML: {original_error}"));
            }

            serde_yaml::from_str::<YamlValue>(&repaired).map_err(|repaired_error| {
                format!(
                    "failed to parse YAML: {original_error}; attempted lenient colon repair but still failed: {repaired_error}"
                )
            })
        }
    }
}

fn repair_skill_frontmatter_yaml(raw: &str) -> String {
    raw.lines()
        .map(repair_skill_frontmatter_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn repair_skill_frontmatter_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("- ") {
        return line.to_owned();
    }
    let Some((prefix, value)) = line.split_once(':') else {
        return line.to_owned();
    };
    let key = prefix.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return line.to_owned();
    }

    let value_trimmed = value.trim();
    if value_trimmed.is_empty()
        || value_trimmed.starts_with('"')
        || value_trimmed.starts_with('\'')
        || value_trimmed.starts_with('[')
        || value_trimmed.starts_with('{')
        || value_trimmed.starts_with('|')
        || value_trimmed.starts_with('>')
        || !value_trimmed.contains(':')
    {
        return line.to_owned();
    }

    let leading_whitespace_len = value.len() - value.trim_start_matches(char::is_whitespace).len();
    let leading_whitespace = &value[..leading_whitespace_len];
    let escaped = value_trimmed.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{prefix}:{leading_whitespace}\"{escaped}\"")
}

fn normalize_skill_frontmatter(frontmatter: &mut SkillFrontmatter) {
    frontmatter.name = normalize_optional_metadata_string(frontmatter.name.take());
    frontmatter.description = normalize_optional_metadata_string(frontmatter.description.take());
    frontmatter.license = normalize_optional_metadata_string(frontmatter.license.take());
    frontmatter.compatibility =
        normalize_optional_metadata_string(frontmatter.compatibility.take());
    frontmatter.metadata = normalize_skill_metadata_map(std::mem::take(&mut frontmatter.metadata));
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
    if frontmatter.disable_model_invocation {
        frontmatter.model_visibility = SkillModelVisibility::Hidden;
    }
}

fn normalize_optional_metadata_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn deserialize_skill_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawSkillStringList {
        Single(String),
        Many(Vec<String>),
    }

    let raw = RawSkillStringList::deserialize(deserializer)?;
    let values = match raw {
        RawSkillStringList::Single(value) => value
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        RawSkillStringList::Many(values) => values,
    };
    Ok(values)
}

fn deserialize_skill_metadata_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MetadataValue {
        String(String),
        Bool(bool),
        I64(i64),
        U64(u64),
        F64(f64),
    }

    let raw = BTreeMap::<String, MetadataValue>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(key, value)| {
            let normalized_value = match value {
                MetadataValue::String(value) => value,
                MetadataValue::Bool(value) => value.to_string(),
                MetadataValue::I64(value) => value.to_string(),
                MetadataValue::U64(value) => value.to_string(),
                MetadataValue::F64(value) => value.to_string(),
            };
            (key, normalized_value)
        })
        .collect())
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

fn normalize_skill_metadata_map(values: BTreeMap<String, String>) -> BTreeMap<String, String> {
    values
        .into_iter()
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
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
        license: frontmatter.license.clone(),
        compatibility: frontmatter.compatibility.clone(),
        metadata: frontmatter.metadata.clone(),
        scope,
        source_kind,
        source_path,
        skill_md_path,
        sha256: hex::encode(Sha256::digest(skill_markdown.as_bytes())),
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
    let mut missing_config = Vec::new();
    for selector in &frontmatter.required_config {
        let selector_enabled = runtime_config_selector_enabled(config, selector);
        match selector_enabled {
            Some(true) => {}
            Some(false) => {
                missing_config.push(selector.clone());
                issues.push(format!("config gate `{selector}` is disabled"));
            }
            None => {
                missing_config.push(selector.clone());
                issues.push(format!("unsupported config gate `{selector}`"));
            }
        }
    }
    let available = issues.is_empty();
    SkillEligibility {
        available,
        eligible: available,
        missing_env,
        missing_bin,
        missing_paths,
        missing_config,
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
        SkillAudience::Operator => {
            let mut value = serde_json::to_value(&entry).unwrap_or_else(|_| json!({}));
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "pack_memberships".to_owned(),
                    pack_membership_payload_from_skill(entry.skill_id.as_str()),
                );
            }
            value
        }
        SkillAudience::Model => json!(DiscoveredSkillModelView::from(entry)),
    }
}

fn serialize_skill_entries_for_audience(
    entries: Vec<DiscoveredSkillEntry>,
    audience: SkillAudience,
) -> Value {
    match audience {
        SkillAudience::Operator => json!(
            entries
                .into_iter()
                .map(|entry| serialize_skill_entry_for_audience(entry, SkillAudience::Operator))
                .collect::<Vec<_>>()
        ),
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
                && entry.invocation_policy != SkillInvocationPolicy::Manual
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
    if skill.invocation_policy == SkillInvocationPolicy::Manual {
        blockers
            .push("the skill is manual-only and not invokable from the model surface".to_owned());
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
    if !skill.eligibility.missing_config.is_empty() {
        blockers.push(format!(
            "disabled or unavailable config gates: {}",
            skill.eligibility.missing_config.join(", ")
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

fn copy_embedded_dir_recursive(
    source: &include_dir::Dir<'static>,
    destination: &Path,
) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create bundled external skill destination {}: {error}",
            destination.display()
        )
    })?;
    for entry in source.entries() {
        match entry {
            include_dir::DirEntry::Dir(dir) => {
                let Some(name) = dir.path().file_name() else {
                    return Err(format!(
                        "bundled external skill directory `{}` has no terminal name",
                        dir.path().display()
                    ));
                };
                copy_embedded_dir_recursive(dir, &destination.join(name))?;
            }
            include_dir::DirEntry::File(file) => {
                let Some(name) = file.path().file_name() else {
                    return Err(format!(
                        "bundled external skill file `{}` has no terminal name",
                        file.path().display()
                    ));
                };
                fs::write(destination.join(name), file.contents()).map_err(|error| {
                    format!(
                        "failed to write bundled external skill file {}: {error}",
                        destination.join(name).display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn digest_embedded_dir(source: &include_dir::Dir<'static>) -> String {
    fn update_dir(hasher: &mut Sha256, dir: &include_dir::Dir<'static>) {
        for entry in dir.entries() {
            match entry {
                include_dir::DirEntry::Dir(child) => {
                    hasher.update(b"dir:");
                    hasher.update(child.path().to_string_lossy().as_bytes());
                    hasher.update(b"\n");
                    update_dir(hasher, child);
                }
                include_dir::DirEntry::File(file) => {
                    hasher.update(b"file:");
                    hasher.update(file.path().to_string_lossy().as_bytes());
                    hasher.update(b"\n");
                    hasher.update(file.contents());
                    hasher.update(b"\n");
                }
            }
        }
    }

    let mut hasher = Sha256::new();
    update_dir(&mut hasher, source);
    let digest = hasher.finalize();
    hex::encode(digest)
}

fn load_installed_skill_index(root: &Path) -> Result<InstalledSkillIndex, String> {
    let index_path = root.join(DEFAULT_INDEX_FILENAME);
    if !index_path.exists() {
        return Ok(InstalledSkillIndex::default());
    }
    let raw = fs::read_to_string(&index_path).map_err(|error| {
        format!(
            "failed to read skills index {}: {error}",
            index_path.display()
        )
    })?;
    let mut index: InstalledSkillIndex = serde_json::from_str(raw.as_str()).map_err(|error| {
        format!(
            "failed to parse skills index {}: {error}",
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
            "failed to create skills install root {}: {error}",
            root.display()
        )
    })?;
    let index_path = root.join(DEFAULT_INDEX_FILENAME);
    let encoded = serde_json::to_string_pretty(index)
        .map_err(|error| format!("failed to encode skills index: {error}"))?;
    fs::write(&index_path, encoded).map_err(|error| {
        format!(
            "failed to write skills index {}: {error}",
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
        "license": skill.license.clone(),
        "compatibility": skill.compatibility.clone(),
        "metadata": skill.metadata.clone(),
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

fn pack_membership_payload_from_skill(skill_id: &str) -> Value {
    json!(
        super::bundled_skills::bundled_skill_pack_memberships(skill_id)
            .into_iter()
            .map(|pack| {
                json!({
                    "pack_id": pack.pack_id,
                    "display_name": pack.display_name,
                    "onboarding_visible": pack.onboarding_visible,
                    "recommended": pack.recommended,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn runtime_config_selector_enabled(
    config: &super::runtime_config::ToolRuntimeConfig,
    selector: &str,
) -> Option<bool> {
    let normalized_selector = selector.trim().to_ascii_lowercase();

    if let Some(server_name) = mcp_server_selector_name(normalized_selector.as_str()) {
        return load_runtime_mcp_snapshot(config).map(|snapshot| {
            snapshot
                .servers
                .iter()
                .any(|server| server.name == server_name && mcp_server_satisfies_skill_gate(server))
        });
    }

    if let Some(server_name) = acp_bootstrap_mcp_server_selector_name(normalized_selector.as_str())
    {
        return load_runtime_mcp_snapshot(config).map(|snapshot| {
            snapshot.servers.iter().any(|server| {
                server.name == server_name
                    && server.selected_for_acp_bootstrap
                    && mcp_server_satisfies_skill_gate(server)
            })
        });
    }

    match normalized_selector.as_str() {
        "skills.enabled" | "tools.skills.enabled" => Some(config.skills.enabled),
        "browser.enabled" | "tools.browser.enabled" => Some(config.browser.enabled),
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

fn mcp_server_selector_name(selector: &str) -> Option<String> {
    [
        "mcp.server.",
        "mcp.servers.",
        "tools.mcp.server.",
        "tools.mcp.servers.",
    ]
    .iter()
    .find_map(|prefix| selector.strip_prefix(prefix))
    .and_then(canonical_mcp_server_selector_name)
}

fn acp_bootstrap_mcp_server_selector_name(selector: &str) -> Option<String> {
    [
        "acp.bootstrap_mcp_server.",
        "acp.bootstrap_mcp_servers.",
        "acp.dispatch.bootstrap_mcp_server.",
        "acp.dispatch.bootstrap_mcp_servers.",
    ]
    .iter()
    .find_map(|prefix| selector.strip_prefix(prefix))
    .and_then(canonical_mcp_server_selector_name)
}

fn canonical_mcp_server_selector_name(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn load_runtime_mcp_snapshot(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Option<crate::mcp::McpRuntimeSnapshot> {
    let config_path = config.config_path.as_ref()?;
    let config_path = config_path.to_string_lossy();
    let (_, loong_config) = crate::config::load(Some(config_path.as_ref())).ok()?;
    crate::mcp::collect_mcp_runtime_snapshot(&loong_config).ok()
}

fn mcp_server_satisfies_skill_gate(server: &crate::mcp::McpRuntimeServerSnapshot) -> bool {
    server.enabled
        && matches!(
            server.status.kind,
            crate::mcp::McpServerStatusKind::Pending | crate::mcp::McpServerStatusKind::Connected
        )
}

fn invocation_policy_id(policy: SkillInvocationPolicy) -> &'static str {
    match policy {
        SkillInvocationPolicy::Model => "model",
        SkillInvocationPolicy::Manual => "manual",
        SkillInvocationPolicy::Both => "both",
    }
}

pub(crate) fn install_bundled_preinstall_targets_for_bootstrap(
    config: &super::runtime_config::ToolRuntimeConfig,
    selected_target_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    if selected_target_ids.is_empty() {
        return Ok(Vec::new());
    }
    let _ = require_enabled_runtime_policy(config)?;
    let install_root = resolve_install_root(config);
    fs::create_dir_all(&install_root).map_err(|error| {
        format!(
            "failed to create skills install root {}: {error}",
            install_root.display()
        )
    })?;

    let mut index = load_installed_skill_index(&install_root)?;
    let mut installed = BTreeSet::new();
    for target_id in selected_target_ids {
        let target = super::bundled_skills::bundled_preinstall_targets()
            .iter()
            .find(|target| target.install_id == target_id.as_str())
            .ok_or_else(|| {
                format!("unknown bundled preinstall target `{target_id}` during startup bootstrap")
            })?;
        for skill_id in target.skill_ids {
            if install_bundled_skill_for_bootstrap(&install_root, &mut index, skill_id)? {
                installed.insert((*skill_id).to_owned());
            }
        }
    }

    persist_installed_skill_index(&install_root, &mut index)?;
    Ok(installed.into_iter().collect())
}

pub(crate) fn remove_bundled_preinstall_targets_for_bootstrap(
    config: &super::runtime_config::ToolRuntimeConfig,
    selected_target_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    if selected_target_ids.is_empty() {
        return Ok(Vec::new());
    }
    let _ = require_enabled_runtime_policy(config)?;
    let install_root = resolve_install_root(config);
    let mut index = load_installed_skill_index(&install_root)?;
    let mut removed = BTreeSet::new();
    for target_id in selected_target_ids {
        let target = super::bundled_skills::bundled_preinstall_targets()
            .iter()
            .find(|target| target.install_id == target_id.as_str())
            .ok_or_else(|| {
                format!("unknown bundled preinstall target `{target_id}` during startup bootstrap")
            })?;
        for skill_id in target.skill_ids {
            if remove_installed_skill_from_index(&install_root, &mut index, skill_id)? {
                removed.insert(normalize_skill_id(skill_id)?);
            }
        }
    }

    persist_installed_skill_index(&install_root, &mut index)?;
    Ok(removed.into_iter().collect())
}

