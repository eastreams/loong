use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::{ProviderFailoverReason, rate_limit::RateLimitObservation};

#[derive(Debug, Clone)]
pub(super) struct ModelCandidateCooldownPolicy {
    pub(super) namespace: String,
    pub(super) cooldown: Duration,
    pub(super) max_cooldown: Duration,
    pub(super) max_entries: usize,
}

#[derive(Debug, Clone)]
pub(super) struct ModelCandidateCooldownEntry {
    pub(super) reason: ProviderFailoverReason,
    pub(super) failure_count: u32,
    pub(super) expires_at: Instant,
}

#[derive(Debug, Default)]
pub(super) struct ModelCandidateCooldownCache {
    entries: HashMap<String, ModelCandidateCooldownEntry>,
    order: VecDeque<String>,
}

impl ModelCandidateCooldownCache {
    pub(super) fn lookup_active(
        &mut self,
        key: &str,
        now: Instant,
    ) -> Option<&ModelCandidateCooldownEntry> {
        self.prune_expired(now);
        self.entries.get(key).filter(|entry| entry.expires_at > now)
    }

    pub(super) fn put(
        &mut self,
        key: String,
        reason: ProviderFailoverReason,
        now: Instant,
        base_cooldown: Duration,
        max_cooldown: Duration,
        max_entries: usize,
    ) {
        if base_cooldown.is_zero() {
            return;
        }
        self.prune_expired(now);
        let bounded_max_cooldown = max_cooldown.max(base_cooldown);
        let failure_count = self
            .entries
            .get(key.as_str())
            .map_or(1, |entry| entry.failure_count.saturating_add(1));
        let exponent = failure_count.saturating_sub(1).min(20);
        let multiplier = 1u32 << exponent;
        let effective_cooldown = base_cooldown
            .saturating_mul(multiplier)
            .min(bounded_max_cooldown);
        let Some(expires_at) = now.checked_add(effective_cooldown) else {
            return;
        };
        self.entries.insert(
            key.clone(),
            ModelCandidateCooldownEntry {
                reason,
                failure_count,
                expires_at,
            },
        );
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key);
        self.prune_capacity(max_entries);
    }

    fn prune_expired(&mut self, now: Instant) {
        self.entries.retain(|_, entry| entry.expires_at > now);
        self.order.retain(|key| self.entries.contains_key(key));
    }

    fn prune_capacity(&mut self, max_entries: usize) {
        while self.entries.len() > max_entries {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&evicted);
        }
    }
}

fn with_model_candidate_cooldowns<R>(run: impl FnOnce(&mut ModelCandidateCooldownCache) -> R) -> R {
    let cache = MODEL_CANDIDATE_COOLDOWNS
        .get_or_init(|| Mutex::new(ModelCandidateCooldownCache::default()));
    let mut guard = match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    run(&mut guard)
}

pub(super) fn register_model_candidate_cooldown(
    policy: &ModelCandidateCooldownPolicy,
    model: &str,
    reason: ProviderFailoverReason,
    rate_limit: Option<&RateLimitObservation>,
) {
    if !matches!(
        reason,
        ProviderFailoverReason::ModelMismatch
            | ProviderFailoverReason::PayloadIncompatible
            | ProviderFailoverReason::RateLimited
            | ProviderFailoverReason::ProviderOverloaded
    ) {
        return;
    }
    let base_cooldown = resolve_model_candidate_cooldown_duration(policy, rate_limit);
    let key = build_model_candidate_cooldown_key(policy.namespace.as_str(), model);
    with_model_candidate_cooldowns(|cache| {
        cache.put(
            key,
            reason,
            Instant::now(),
            base_cooldown,
            policy.max_cooldown,
            policy.max_entries,
        )
    });
}

pub(super) fn resolve_model_candidate_cooldown_duration(
    policy: &ModelCandidateCooldownPolicy,
    rate_limit: Option<&RateLimitObservation>,
) -> Duration {
    rate_limit
        .and_then(RateLimitObservation::cooldown_hint)
        .unwrap_or(policy.cooldown)
}

pub(super) fn prioritize_model_candidates_by_cooldown(
    mut models: Vec<String>,
    policy: Option<&ModelCandidateCooldownPolicy>,
) -> Vec<String> {
    let Some(policy) = policy else {
        return models;
    };
    if models.len() <= 1 {
        return models;
    }

    let now = Instant::now();
    let mut ready = Vec::with_capacity(models.len());
    let mut cooling = Vec::new();
    for model in models.drain(..) {
        let key = build_model_candidate_cooldown_key(policy.namespace.as_str(), model.as_str());
        let active_reason = with_model_candidate_cooldowns(|cache| {
            cache
                .lookup_active(key.as_str(), now)
                .map(|entry| entry.reason)
        });
        if active_reason.is_some() {
            cooling.push(model);
        } else {
            ready.push(model);
        }
    }

    if ready.is_empty() {
        return cooling;
    }
    ready.extend(cooling);
    ready
}

fn build_model_candidate_cooldown_key(namespace: &str, model: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    namespace.trim().hash(&mut hasher);
    model.trim().to_ascii_lowercase().hash(&mut hasher);
    format!("{namespace}::model::{:016x}", hasher.finish())
}

static MODEL_CANDIDATE_COOLDOWNS: OnceLock<Mutex<ModelCandidateCooldownCache>> = OnceLock::new();
