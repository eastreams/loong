use reqwest::Url;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExternalSkillSourceKind {
    DirectUrl,
    Github,
    SkillsSh,
    Clawhub,
    Npm,
}

impl ExternalSkillSourceKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DirectUrl => "direct_url",
            Self::Github => "github",
            Self::SkillsSh => "skills_sh",
            Self::Clawhub => "clawhub",
            Self::Npm => "npm",
        }
    }
}

pub(crate) const DEFAULT_EXTERNAL_SKILL_SEARCH_SOURCE_ORDER: [ExternalSkillSourceKind; 4] = [
    ExternalSkillSourceKind::SkillsSh,
    ExternalSkillSourceKind::Clawhub,
    ExternalSkillSourceKind::Github,
    ExternalSkillSourceKind::Npm,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExternalSkillRouteCandidate {
    pub label: String,
    pub url: String,
    pub priority: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResolvedExternalSkillCandidate {
    pub source_kind: ExternalSkillSourceKind,
    pub canonical_reference: String,
    pub display_name: String,
    pub landing_url: Option<String>,
    pub metadata_url: Option<String>,
    pub endpoint_routes: Vec<ExternalSkillRouteCandidate>,
    pub artifact_routes: Vec<ExternalSkillRouteCandidate>,
}

pub(crate) fn detect_external_skill_source_kind(
    raw: &str,
) -> Result<ExternalSkillSourceKind, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("external skill reference must not be empty".to_owned());
    }

    let parsed_url = Url::parse(trimmed);
    if let Ok(url) = parsed_url {
        return detect_source_kind_from_url(&url);
    }

    if is_github_repo_shorthand(trimmed) {
        return Ok(ExternalSkillSourceKind::Github);
    }

    if is_npm_package_name(trimmed) {
        return Ok(ExternalSkillSourceKind::Npm);
    }

    Err(format!(
        "unsupported external skill reference `{trimmed}`; expected a supported URL, GitHub repo shorthand, or npm package name"
    ))
}

pub(crate) fn resolve_external_skill_candidate(
    raw: &str,
) -> Result<ResolvedExternalSkillCandidate, String> {
    let trimmed = raw.trim();
    let source_kind = detect_external_skill_source_kind(trimmed)?;

    let parsed_url = Url::parse(trimmed);
    if let Ok(url) = parsed_url {
        let candidate = match source_kind {
            ExternalSkillSourceKind::DirectUrl => resolve_direct_url_candidate(url),
            ExternalSkillSourceKind::Github => resolve_github_url_candidate(url)?,
            ExternalSkillSourceKind::SkillsSh => resolve_skills_sh_candidate(url),
            ExternalSkillSourceKind::Clawhub => resolve_clawhub_candidate(url),
            ExternalSkillSourceKind::Npm => resolve_npm_url_candidate(url)?,
        };
        return Ok(candidate);
    }

    match source_kind {
        ExternalSkillSourceKind::Github => resolve_github_shorthand_candidate(trimmed),
        ExternalSkillSourceKind::Npm => Ok(resolve_npm_package_candidate(trimmed)),
        ExternalSkillSourceKind::DirectUrl
        | ExternalSkillSourceKind::SkillsSh
        | ExternalSkillSourceKind::Clawhub => Err(format!(
            "unsupported external skill reference `{trimmed}`; expected a supported URL, GitHub repo shorthand, or npm package name"
        )),
    }
}

pub(crate) fn parse_external_skill_source_kind(raw: &str) -> Option<ExternalSkillSourceKind> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "direct_url" | "direct-url" | "url" => Some(ExternalSkillSourceKind::DirectUrl),
        "github" => Some(ExternalSkillSourceKind::Github),
        "skills_sh" | "skills-sh" | "skillssh" => Some(ExternalSkillSourceKind::SkillsSh),
        "clawhub" | "clawhub_ai" | "clawhub-ai" => Some(ExternalSkillSourceKind::Clawhub),
        "npm" => Some(ExternalSkillSourceKind::Npm),
        _ => None,
    }
}

pub(crate) fn default_external_skill_search_sources() -> Vec<ExternalSkillSourceKind> {
    DEFAULT_EXTERNAL_SKILL_SEARCH_SOURCE_ORDER.to_vec()
}

pub(crate) fn search_query_for_external_skill_source(
    source_kind: ExternalSkillSourceKind,
    query: &str,
) -> Option<String> {
    let trimmed_query = query.trim();
    if trimmed_query.is_empty() {
        return None;
    }

    let search_query = match source_kind {
        ExternalSkillSourceKind::DirectUrl => return None,
        ExternalSkillSourceKind::Github => {
            format!("site:github.com {trimmed_query} \"SKILL.md\"")
        }
        ExternalSkillSourceKind::SkillsSh => {
            format!("site:skills.sh {trimmed_query}")
        }
        ExternalSkillSourceKind::Clawhub => {
            format!("site:clawhub.ai/skills {trimmed_query}")
        }
        ExternalSkillSourceKind::Npm => {
            format!("site:npmjs.com/package {trimmed_query} skill")
        }
    };

    Some(search_query)
}

fn detect_source_kind_from_url(url: &Url) -> Result<ExternalSkillSourceKind, String> {
    let scheme = url.scheme();
    if scheme != "https" {
        return Err(format!(
            "external skill references must use https URLs when a URL is provided, got `{scheme}`"
        ));
    }

    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "external skill URL is missing a host".to_owned())?;

    if host == "clawhub.io" || host.ends_with(".clawhub.io") {
        return Err(format!(
            "external skill source `{host}` is blocked because `clawhub.io` is not a supported ClawHub domain"
        ));
    }

    if host == "github.com" || host == "www.github.com" {
        return Ok(ExternalSkillSourceKind::Github);
    }

    if host == "skills.sh" || host == "www.skills.sh" {
        return Ok(ExternalSkillSourceKind::SkillsSh);
    }

    if host == "clawhub.ai" || host.ends_with(".clawhub.ai") {
        return Ok(ExternalSkillSourceKind::Clawhub);
    }

    if host == "npmjs.com" || host == "www.npmjs.com" || host == "registry.npmjs.org" {
        return Ok(ExternalSkillSourceKind::Npm);
    }

    Ok(ExternalSkillSourceKind::DirectUrl)
}

fn resolve_direct_url_candidate(url: Url) -> ResolvedExternalSkillCandidate {
    let url_text = url.to_string();
    let display_name = derive_display_name_from_url_path(&url);
    let endpoint_route = build_route_candidate("primary", url_text.as_str(), 0);
    let canonical_reference = url_text.clone();
    let landing_url = Some(url_text);

    ResolvedExternalSkillCandidate {
        source_kind: ExternalSkillSourceKind::DirectUrl,
        canonical_reference,
        display_name,
        landing_url,
        metadata_url: None,
        endpoint_routes: vec![endpoint_route.clone()],
        artifact_routes: vec![endpoint_route],
    }
}

fn resolve_github_url_candidate(url: Url) -> Result<ResolvedExternalSkillCandidate, String> {
    let path_segments = github_path_segments(&url)?;
    let owner = required_path_segment(&path_segments, 0, "GitHub owner")?;
    let repo = required_path_segment(&path_segments, 1, "GitHub repository")?;
    let canonical_reference = format!("github:{owner}/{repo}");
    let display_name = repo.to_owned();
    let landing_url = format!("https://github.com/{owner}/{repo}");
    let metadata_url = format!("https://api.github.com/repos/{owner}/{repo}");
    let api_route = build_route_candidate("github_api", metadata_url.as_str(), 0);
    let codeload_base = format!("https://codeload.github.com/{owner}/{repo}");
    let codeload_route = build_route_candidate("github_codeload", codeload_base.as_str(), 1);

    let mut artifact_routes = Vec::new();
    let tree_branch = github_tree_branch(&path_segments);
    if let Some(branch) = tree_branch {
        let artifact_url =
            format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/heads/{branch}");
        let artifact_route = build_route_candidate("github_tree_tarball", artifact_url.as_str(), 0);
        artifact_routes.push(artifact_route);
    }

    let release_asset_url = github_release_asset_url(&url, &path_segments);
    if let Some(release_asset_url) = release_asset_url {
        let release_route =
            build_route_candidate("github_release_asset", release_asset_url.as_str(), 0);
        artifact_routes.push(release_route);
    }

    Ok(ResolvedExternalSkillCandidate {
        source_kind: ExternalSkillSourceKind::Github,
        canonical_reference,
        display_name,
        landing_url: Some(landing_url),
        metadata_url: Some(metadata_url),
        endpoint_routes: vec![api_route, codeload_route],
        artifact_routes,
    })
}

fn resolve_github_shorthand_candidate(raw: &str) -> Result<ResolvedExternalSkillCandidate, String> {
    let mut parts = raw.split('/');
    let owner = parts
        .next()
        .ok_or_else(|| "GitHub shorthand is missing an owner".to_owned())?;
    let repo = parts
        .next()
        .ok_or_else(|| "GitHub shorthand is missing a repository".to_owned())?;
    let landing_url = format!("https://github.com/{owner}/{repo}");
    let url = Url::parse(landing_url.as_str())
        .map_err(|error| format!("failed to normalize GitHub shorthand `{raw}`: {error}"))?;
    resolve_github_url_candidate(url)
}

fn resolve_skills_sh_candidate(url: Url) -> ResolvedExternalSkillCandidate {
    let url_text = url.to_string();
    let path = url.path().trim_matches('/').to_owned();
    let display_name = derive_display_name_from_url_path(&url);
    let canonical_reference = format!("skills.sh:{path}");
    let endpoint_route = build_route_candidate("skills_sh_primary", "https://skills.sh", 0);

    ResolvedExternalSkillCandidate {
        source_kind: ExternalSkillSourceKind::SkillsSh,
        canonical_reference,
        display_name,
        landing_url: Some(url_text.clone()),
        metadata_url: Some(url_text),
        endpoint_routes: vec![endpoint_route],
        artifact_routes: Vec::new(),
    }
}

fn resolve_clawhub_candidate(url: Url) -> ResolvedExternalSkillCandidate {
    let url_text = url.to_string();
    let path = url.path().trim_matches('/').to_owned();
    let display_name = derive_display_name_from_url_path(&url);
    let canonical_reference = format!("clawhub:{path}");
    let metadata_url = "https://clawhub.ai/.well-known/clawhub.json".to_owned();
    let primary_route = build_route_candidate("clawhub_primary", "https://clawhub.ai", 0);
    let mirror_route =
        build_route_candidate("clawhub_cn_mirror", "https://mirror-cn.clawhub.ai", 1);

    ResolvedExternalSkillCandidate {
        source_kind: ExternalSkillSourceKind::Clawhub,
        canonical_reference,
        display_name,
        landing_url: Some(url_text),
        metadata_url: Some(metadata_url),
        endpoint_routes: vec![primary_route, mirror_route],
        artifact_routes: Vec::new(),
    }
}

fn resolve_npm_url_candidate(url: Url) -> Result<ResolvedExternalSkillCandidate, String> {
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "npm URL is missing a host".to_owned())?;
    if host == "registry.npmjs.org" {
        let path = url.path().trim_matches('/');
        let decoded_name = percent_decode_path(path);
        return Ok(resolve_npm_package_candidate(decoded_name.as_str()));
    }

    let path_segments = collect_path_segments(&url);
    let package_index = path_segments
        .iter()
        .position(|segment| *segment == "package")
        .ok_or_else(|| "npm package URL must contain `/package/<name>`".to_owned())?;
    let package_start = package_index.saturating_add(1);
    let package_parts = path_segments
        .get(package_start..)
        .ok_or_else(|| "npm package URL must contain a package name".to_owned())?;
    let package_name = package_name_from_url_segments(package_parts)?;
    Ok(resolve_npm_package_candidate(package_name.as_str()))
}

fn resolve_npm_package_candidate(raw: &str) -> ResolvedExternalSkillCandidate {
    let canonical_reference = format!("npm:{raw}");
    let display_name = raw
        .rsplit('/')
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| raw.to_owned());
    let encoded_name = encode_npm_package_name(raw);
    let landing_url = format!("https://www.npmjs.com/package/{raw}");
    let metadata_url = format!("https://registry.npmjs.org/{encoded_name}");
    let endpoint_route = build_route_candidate("npm_registry", "https://registry.npmjs.org", 0);

    ResolvedExternalSkillCandidate {
        source_kind: ExternalSkillSourceKind::Npm,
        canonical_reference,
        display_name,
        landing_url: Some(landing_url),
        metadata_url: Some(metadata_url),
        endpoint_routes: vec![endpoint_route],
        artifact_routes: Vec::new(),
    }
}

fn build_route_candidate(label: &str, url: &str, priority: usize) -> ExternalSkillRouteCandidate {
    ExternalSkillRouteCandidate {
        label: label.to_owned(),
        url: url.to_owned(),
        priority,
    }
}

fn github_path_segments(url: &Url) -> Result<Vec<String>, String> {
    let segments = collect_path_segments(url);
    if segments.len() < 2 {
        return Err("GitHub URL must contain an owner and repository".to_owned());
    }
    Ok(segments)
}

fn github_tree_branch(path_segments: &[String]) -> Option<&str> {
    let tree_segment = path_segments.get(2)?;
    if tree_segment != "tree" {
        return None;
    }
    path_segments.get(3).map(String::as_str)
}

fn github_release_asset_url(url: &Url, path_segments: &[String]) -> Option<String> {
    let release_segment = path_segments.get(2)?;
    let download_segment = path_segments.get(3)?;
    let tag_segment = path_segments.get(4)?;
    let asset_segment = path_segments.get(5)?;

    if release_segment != "releases" {
        return None;
    }
    if download_segment != "download" {
        return None;
    }

    let owner = path_segments.first()?;
    let repo = path_segments.get(1)?;
    let asset_url = format!(
        "https://github.com/{owner}/{repo}/releases/download/{tag_segment}/{asset_segment}"
    );
    let normalized_asset_url = if url.query().is_some() {
        url.as_str().to_owned()
    } else {
        asset_url
    };
    Some(normalized_asset_url)
}

fn collect_path_segments(url: &Url) -> Vec<String> {
    let segments = url.path_segments();
    let Some(segments) = segments else {
        return Vec::new();
    };

    segments
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn required_path_segment<'a>(
    path_segments: &'a [String],
    index: usize,
    label: &str,
) -> Result<&'a str, String> {
    let segment = path_segments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{label} is missing from external skill reference"))?;
    Ok(segment)
}

fn derive_display_name_from_url_path(url: &Url) -> String {
    let segments = collect_path_segments(url);
    let last_segment = segments
        .last()
        .map(String::as_str)
        .unwrap_or("external-skill");
    let trimmed_extension = trim_archive_extension(last_segment);
    trimmed_extension.to_owned()
}

fn trim_archive_extension(raw: &str) -> &str {
    if let Some(value) = raw.strip_suffix(".tar.gz") {
        return value;
    }
    if let Some(value) = raw.strip_suffix(".tgz") {
        return value;
    }
    raw
}

fn package_name_from_url_segments(segments: &[String]) -> Result<String, String> {
    if segments.is_empty() {
        return Err("npm package URL is missing the package name".to_owned());
    }

    let first = segments.first().map(String::as_str).unwrap_or_default();
    let second = segments.get(1).map(String::as_str);
    let package_name = if first.starts_with('@') {
        let second = second.ok_or_else(|| {
            "scoped npm package URL must contain both scope and package".to_owned()
        })?;
        format!("{first}/{second}")
    } else {
        first.to_owned()
    };
    Ok(package_name)
}

fn percent_decode_path(raw: &str) -> String {
    let replacements = [
        ("%40", "@"),
        ("%2f", "/"),
        ("%2F", "/"),
        ("%2e", "."),
        ("%2E", "."),
        ("%5f", "_"),
        ("%5F", "_"),
        ("%2d", "-"),
        ("%2D", "-"),
    ];

    let mut decoded = raw.to_owned();
    for (from, to) in replacements {
        decoded = decoded.replace(from, to);
    }
    decoded
}

fn encode_npm_package_name(raw: &str) -> String {
    raw.replace('/', "%2f")
}

fn is_github_repo_shorthand(raw: &str) -> bool {
    if raw.starts_with('@') {
        return false;
    }

    let mut parts = raw.split('/');
    let owner = parts.next();
    let repo = parts.next();
    let extra = parts.next();

    let Some(owner) = owner else {
        return false;
    };
    let Some(repo) = repo else {
        return false;
    };

    if extra.is_some() {
        return false;
    }

    is_simple_repo_token(owner) && is_simple_repo_token(repo)
}

fn is_npm_package_name(raw: &str) -> bool {
    if raw.contains("://") {
        return false;
    }

    if raw.contains(' ') {
        return false;
    }

    if raw.starts_with('@') {
        let mut parts = raw.split('/');
        let scope = parts.next();
        let package = parts.next();
        let extra = parts.next();

        let Some(scope) = scope else {
            return false;
        };
        let Some(package) = package else {
            return false;
        };

        if extra.is_some() {
            return false;
        }

        let scope_name = scope.trim_start_matches('@');
        return is_npm_token(scope_name) && is_npm_token(package);
    }

    if raw.contains('/') {
        return false;
    }

    is_npm_token(raw)
}

fn is_simple_repo_token(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }

    raw.chars().all(is_repo_token_char)
}

fn is_repo_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')
}

fn is_npm_token(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }

    raw.chars().all(is_npm_token_char)
}

fn is_npm_token_char(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
}

#[cfg(test)]
mod tests {
    use super::{
        ExternalSkillSourceKind, default_external_skill_search_sources,
        detect_external_skill_source_kind, is_github_repo_shorthand, is_npm_package_name,
        parse_external_skill_source_kind, resolve_external_skill_candidate,
        search_query_for_external_skill_source,
    };

    #[test]
    fn external_skills_source_kind_recognizes_github_reference() {
        let raw = "https://github.com/vercel-labs/agent-skills";
        let source_kind =
            detect_external_skill_source_kind(raw).expect("github reference should parse");
        assert_eq!(source_kind, ExternalSkillSourceKind::Github);
    }

    #[test]
    fn external_skills_source_kind_recognizes_skills_sh_reference() {
        let raw = "https://skills.sh/github/awesome-copilot/refactor-plan";
        let source_kind =
            detect_external_skill_source_kind(raw).expect("skills.sh reference should parse");
        assert_eq!(source_kind, ExternalSkillSourceKind::SkillsSh);
    }

    #[test]
    fn external_skills_source_kind_recognizes_clawhub_reference() {
        let raw = "https://clawhub.ai/skills/hybrid-deep-search";
        let source_kind =
            detect_external_skill_source_kind(raw).expect("clawhub reference should parse");
        assert_eq!(source_kind, ExternalSkillSourceKind::Clawhub);
    }

    #[test]
    fn external_skills_source_kind_recognizes_npm_package_name() {
        let raw = "@scope/skill-pack";
        let source_kind = detect_external_skill_source_kind(raw).expect("npm package should parse");
        assert_eq!(source_kind, ExternalSkillSourceKind::Npm);
    }

    #[test]
    fn external_skills_source_kind_rejects_unsupported_reference() {
        let raw = "find me a cool skill";
        let error =
            detect_external_skill_source_kind(raw).expect_err("unsupported reference should fail");
        assert!(error.contains("unsupported external skill reference"));
    }

    #[test]
    fn github_repo_shorthand_rejects_scoped_npm_names() {
        let raw = "@scope/package";
        assert!(!is_github_repo_shorthand(raw));
    }

    #[test]
    fn npm_package_name_rejects_github_style_owner_repo() {
        let raw = "owner/repo";
        assert!(!is_npm_package_name(raw));
    }

    #[test]
    fn direct_https_url_is_treated_as_direct_url_source() {
        let raw = "https://example.com/skill.tgz";
        let source_kind = detect_external_skill_source_kind(raw).expect("direct url should parse");
        assert_eq!(source_kind, ExternalSkillSourceKind::DirectUrl);
    }

    #[test]
    fn external_skills_resolver_normalizes_github_repo_candidate() {
        let raw = "vercel-labs/agent-skills";
        let candidate =
            resolve_external_skill_candidate(raw).expect("github shorthand should resolve");
        assert_eq!(candidate.source_kind, ExternalSkillSourceKind::Github);
        assert_eq!(
            candidate.canonical_reference,
            "github:vercel-labs/agent-skills"
        );
        assert_eq!(
            candidate.metadata_url.as_deref(),
            Some("https://api.github.com/repos/vercel-labs/agent-skills")
        );
        assert_eq!(candidate.endpoint_routes.len(), 2);
    }

    #[test]
    fn external_skills_resolver_normalizes_github_tree_candidate() {
        let raw = "https://github.com/vercel-labs/agent-skills/tree/main/skills/refactor";
        let candidate = resolve_external_skill_candidate(raw).expect("github tree should resolve");
        assert_eq!(candidate.source_kind, ExternalSkillSourceKind::Github);
        assert_eq!(candidate.artifact_routes.len(), 1);
        assert_eq!(
            candidate.artifact_routes[0].url,
            "https://codeload.github.com/vercel-labs/agent-skills/tar.gz/refs/heads/main"
        );
    }

    #[test]
    fn external_skills_resolver_normalizes_skills_sh_candidate() {
        let raw = "https://skills.sh/github/awesome-copilot/refactor-plan";
        let candidate =
            resolve_external_skill_candidate(raw).expect("skills.sh reference should resolve");
        assert_eq!(candidate.source_kind, ExternalSkillSourceKind::SkillsSh);
        assert_eq!(
            candidate.canonical_reference,
            "skills.sh:github/awesome-copilot/refactor-plan"
        );
        assert_eq!(candidate.endpoint_routes.len(), 1);
        assert_eq!(candidate.endpoint_routes[0].url, "https://skills.sh");
    }

    #[test]
    fn external_skills_resolver_normalizes_clawhub_candidate() {
        let raw = "https://clawhub.ai/skills/hybrid-deep-search";
        let candidate =
            resolve_external_skill_candidate(raw).expect("clawhub reference should resolve");
        assert_eq!(candidate.source_kind, ExternalSkillSourceKind::Clawhub);
        assert_eq!(candidate.endpoint_routes.len(), 2);
        assert_eq!(candidate.endpoint_routes[0].url, "https://clawhub.ai");
        assert_eq!(
            candidate.endpoint_routes[1].url,
            "https://mirror-cn.clawhub.ai"
        );
        assert_eq!(
            candidate.metadata_url.as_deref(),
            Some("https://clawhub.ai/.well-known/clawhub.json")
        );
    }

    #[test]
    fn external_skills_resolver_normalizes_npm_candidate() {
        let raw = "@scope/skill-pack";
        let candidate = resolve_external_skill_candidate(raw).expect("npm package should resolve");
        assert_eq!(candidate.source_kind, ExternalSkillSourceKind::Npm);
        assert_eq!(candidate.canonical_reference, "npm:@scope/skill-pack");
        assert_eq!(
            candidate.metadata_url.as_deref(),
            Some("https://registry.npmjs.org/@scope%2fskill-pack")
        );
    }

    #[test]
    fn external_skills_resolver_rejects_clawhub_io_reference() {
        let raw = "https://clawhub.io/skills/fake";
        let error =
            resolve_external_skill_candidate(raw).expect_err("clawhub.io should be rejected");
        assert!(error.contains("not a supported ClawHub domain"));
    }

    #[test]
    fn parse_external_skill_source_kind_accepts_known_source_ids() {
        let github = parse_external_skill_source_kind("github");
        let skills_sh = parse_external_skill_source_kind("skills-sh");
        let clawhub = parse_external_skill_source_kind("clawhub_ai");
        let npm = parse_external_skill_source_kind("npm");

        assert_eq!(github, Some(ExternalSkillSourceKind::Github));
        assert_eq!(skills_sh, Some(ExternalSkillSourceKind::SkillsSh));
        assert_eq!(clawhub, Some(ExternalSkillSourceKind::Clawhub));
        assert_eq!(npm, Some(ExternalSkillSourceKind::Npm));
    }

    #[test]
    fn default_external_skill_search_sources_prefer_dedicated_skill_ecosystems() {
        let sources = default_external_skill_search_sources();
        assert_eq!(sources[0], ExternalSkillSourceKind::SkillsSh);
        assert_eq!(sources[1], ExternalSkillSourceKind::Clawhub);
        assert_eq!(sources[2], ExternalSkillSourceKind::Github);
        assert_eq!(sources[3], ExternalSkillSourceKind::Npm);
    }

    #[test]
    fn search_query_for_external_skill_source_uses_source_aware_site_filters() {
        let github_query =
            search_query_for_external_skill_source(ExternalSkillSourceKind::Github, "refactor")
                .expect("github query should build");
        let skills_sh_query =
            search_query_for_external_skill_source(ExternalSkillSourceKind::SkillsSh, "refactor")
                .expect("skills.sh query should build");
        let clawhub_query =
            search_query_for_external_skill_source(ExternalSkillSourceKind::Clawhub, "refactor")
                .expect("clawhub query should build");

        assert!(github_query.contains("site:github.com"));
        assert!(skills_sh_query.contains("site:skills.sh"));
        assert!(clawhub_query.contains("site:clawhub.ai/skills"));
    }
}
