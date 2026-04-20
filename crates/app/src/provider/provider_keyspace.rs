use std::hash::{Hash, Hasher};

pub(super) fn build_provider_cache_key(
    prefix: &str,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    auth_header: Option<&str>,
) -> String {
    let mut header_pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    header_pairs.sort_unstable();
    let header_sig = header_pairs
        .into_iter()
        .map(|(name, value)| format!("{name}:{value}"))
        .collect::<Vec<_>>()
        .join("|");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    endpoint.trim().hash(&mut hasher);
    auth_header.unwrap_or_default().trim().hash(&mut hasher);
    header_sig.hash(&mut hasher);
    format!("{prefix}::{:016x}", hasher.finish())
}

pub(super) fn build_model_catalog_cache_key(
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    auth_header: Option<&str>,
) -> String {
    build_provider_cache_key("provider-model-catalog", endpoint, headers, auth_header)
}

pub(super) fn build_model_candidate_cooldown_namespace(
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    auth_header: Option<&str>,
) -> String {
    build_provider_cache_key(
        "provider-model-candidate-cooldown",
        endpoint,
        headers,
        auth_header,
    )
}

pub(super) fn build_provider_profile_state_namespace(
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
) -> String {
    build_provider_cache_key("provider-profile-state", endpoint, headers, None)
}

pub(super) fn build_provider_profile_state_key(namespace: &str, profile_id: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    namespace.trim().hash(&mut hasher);
    profile_id.trim().to_ascii_lowercase().hash(&mut hasher);
    format!("{namespace}::profile::{:016x}", hasher.finish())
}

pub(super) fn build_provider_auth_profile_id(prefix: &str, secret: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prefix.hash(&mut hasher);
    secret.hash(&mut hasher);
    format!("{prefix}:{:016x}", hasher.finish())
}
