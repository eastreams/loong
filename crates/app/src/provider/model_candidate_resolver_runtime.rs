use std::time::Duration;

use crate::{
    CliResult,
    config::{LoongClawConfig, ProviderConfig},
};

use super::catalog_executor::{ModelCatalogRequestRuntime, fetch_available_models_with_policy};
use super::catalog_runtime::{
    ModelCatalogCacheLookup, fetch_model_catalog_singleflight, load_cached_model_catalog,
    store_model_catalog,
};
use super::model_candidate_cooldown_runtime::{
    ModelCandidateCooldownPolicy, prioritize_model_candidates_by_cooldown,
};
use super::policy;
use super::provider_keyspace::build_model_catalog_cache_key;

pub(super) async fn resolve_request_models(
    config: &LoongClawConfig,
    headers: &reqwest::header::HeaderMap,
    request_policy: &policy::ProviderRequestPolicy,
    model_candidate_cooldown_policy: Option<&ModelCandidateCooldownPolicy>,
    authorization_header: Option<&str>,
) -> CliResult<Vec<String>> {
    if let Some(model) = config.provider.resolved_model() {
        return Ok(vec![model]);
    }
    let cache_ttl_ms = config.provider.resolved_model_catalog_cache_ttl_ms();
    let stale_if_error_ms = config.provider.resolved_model_catalog_stale_if_error_ms();
    let cache_max_entries = config.provider.resolved_model_catalog_cache_max_entries();
    let models_endpoint = config.provider.models_endpoint();
    let cache_key = (cache_ttl_ms > 0)
        .then(|| build_model_catalog_cache_key(&models_endpoint, headers, authorization_header));
    let mut stale_models = None;

    if let Some(cache_key) = cache_key.as_deref() {
        if let Some(lookup) = load_cached_model_catalog(cache_key) {
            let cached_models = match lookup {
                ModelCatalogCacheLookup::Fresh(models) => {
                    let ordered = rank_model_candidates(&config.provider, &models);
                    if !ordered.is_empty() {
                        return Ok(prioritize_model_candidates_by_cooldown(
                            ordered,
                            model_candidate_cooldown_policy,
                        ));
                    }
                    models
                }
                ModelCatalogCacheLookup::Stale(models) => {
                    stale_models = Some(models.clone());
                    models
                }
            };
            if stale_models.is_none() {
                stale_models = Some(cached_models);
            }
        }
    }

    let fetch_result = if let Some(cache_key) = cache_key.as_deref() {
        fetch_model_catalog_singleflight(cache_key, || async {
            fetch_available_models_with_policy(ModelCatalogRequestRuntime {
                provider: &config.provider,
                headers,
                request_policy,
                authorization_header,
            })
            .await
        })
        .await
    } else {
        fetch_available_models_with_policy(ModelCatalogRequestRuntime {
            provider: &config.provider,
            headers,
            request_policy,
            authorization_header,
        })
        .await
    };

    let available = match fetch_result {
        Ok(models) => models,
        Err(error) => {
            if let Some(stale_models) = stale_models {
                let ordered = rank_model_candidates(&config.provider, &stale_models);
                if !ordered.is_empty() {
                    return Ok(prioritize_model_candidates_by_cooldown(
                        ordered,
                        model_candidate_cooldown_policy,
                    ));
                }
            }
            return Err(error);
        }
    };

    if let Some(cache_key) = cache_key {
        store_model_catalog(
            cache_key,
            available.clone(),
            Duration::from_millis(cache_ttl_ms),
            Duration::from_millis(stale_if_error_ms),
            cache_max_entries,
        );
    }
    let ordered = rank_model_candidates(&config.provider, &available);
    if ordered.is_empty() {
        return Err("provider model-list is empty; set provider.model explicitly".to_owned());
    }
    Ok(prioritize_model_candidates_by_cooldown(
        ordered,
        model_candidate_cooldown_policy,
    ))
}

pub(super) fn rank_model_candidates(
    provider: &ProviderConfig,
    available: &[String],
) -> Vec<String> {
    let mut ordered = Vec::new();
    for raw in &provider.preferred_models {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(matched) = available.iter().find(|model| *model == trimmed) {
            push_unique_model(&mut ordered, matched);
            continue;
        }
        if let Some(matched) = available
            .iter()
            .find(|model| model.eq_ignore_ascii_case(trimmed))
        {
            push_unique_model(&mut ordered, matched);
        }
    }

    for model in available {
        push_unique_model(&mut ordered, model);
    }
    ordered
}

fn push_unique_model(out: &mut Vec<String>, model: &str) {
    if out.iter().any(|existing| existing == model) {
        return;
    }
    out.push(model.to_owned());
}
