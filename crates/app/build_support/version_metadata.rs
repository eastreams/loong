#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildMetadata {
    pub release_build: bool,
    pub channel: Option<String>,
    pub short_sha: Option<String>,
}

pub fn resolve_build_metadata(
    release_build_env: Option<&str>,
    explicit_channel_env: Option<&str>,
    explicit_sha_env: Option<&str>,
    git_branch: Option<&str>,
    git_sha: Option<&str>,
) -> BuildMetadata {
    let release_build = release_build_env.is_some_and(is_truthy_env_value);
    if release_build {
        return BuildMetadata {
            release_build: true,
            channel: None,
            short_sha: None,
        };
    }

    let channel = normalize_metadata_value(explicit_channel_env)
        .or_else(|| normalize_metadata_value(git_branch));
    let short_sha = normalize_metadata_value(explicit_sha_env)
        .or_else(|| normalize_metadata_value(git_sha))
        .map(short_sha);

    BuildMetadata {
        release_build: false,
        channel,
        short_sha,
    }
}

pub fn git_rerun_hint_targets(symbolic_head_ref: Option<&str>) -> Vec<String> {
    let mut targets = vec!["HEAD".to_owned()];
    if let Some(head_ref) = normalize_metadata_value(symbolic_head_ref) {
        targets.push(head_ref);
    }
    targets.push("packed-refs".to_owned());
    targets
}

fn normalize_metadata_value(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn short_sha(raw: String) -> String {
    raw.chars().take(7).collect()
}

fn is_truthy_env_value(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}
