use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::failover::ProviderFailoverReason;
use super::profile_health_runtime::ProviderProfileStatePolicy;

#[derive(Debug, Clone)]
pub(super) struct ProviderProfileStateEntry {
    pub(super) reason: ProviderFailoverReason,
    pub(super) failure_count: u32,
    pub(super) cooldown_until: Option<Instant>,
    pub(super) disabled_until: Option<Instant>,
    pub(super) last_used_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ProviderProfileHealthSnapshot {
    pub(super) unusable_until: Option<Instant>,
    pub(super) last_used_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderProfileHealthMode {
    EnforceUnusableWindows,
    ObserveOnly,
}

#[derive(Debug, Default)]
pub(super) struct ProviderProfileStateStore {
    pub(super) entries: HashMap<String, ProviderProfileStateEntry>,
    pub(super) order: VecDeque<String>,
    pub(super) revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ProviderProfileStateSnapshot {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) revision: u64,
    pub(super) generated_at_unix_ms: u64,
    pub(super) order: Vec<String>,
    pub(super) entries: Vec<ProviderProfileStateSnapshotEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ProviderProfileStateSnapshotEntry {
    pub(super) key: String,
    pub(super) reason: String,
    pub(super) failure_count: u32,
    pub(super) cooldown_remaining_ms: Option<u64>,
    pub(super) disabled_remaining_ms: Option<u64>,
    pub(super) last_used_age_ms: Option<u64>,
}

pub(super) const PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION: u32 = 1;

impl ProviderProfileStateStore {
    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub(super) fn health_snapshot(
        &mut self,
        key: &str,
        now: Instant,
    ) -> ProviderProfileHealthSnapshot {
        self.prune_expired(now);
        let Some(entry) = self.entries.get(key) else {
            return ProviderProfileHealthSnapshot::default();
        };
        let cooldown_until = entry.cooldown_until.filter(|until| *until > now);
        let disabled_until = entry.disabled_until.filter(|until| *until > now);
        ProviderProfileHealthSnapshot {
            unusable_until: match (cooldown_until, disabled_until) {
                (Some(cooldown), Some(disabled)) => Some(cooldown.max(disabled)),
                (Some(cooldown), None) => Some(cooldown),
                (None, Some(disabled)) => Some(disabled),
                (None, None) => None,
            },
            last_used_at: entry.last_used_at,
        }
    }

    pub(super) fn mark_success(&mut self, key: String, now: Instant, max_entries: usize) {
        self.prune_expired(now);
        let entry = self
            .entries
            .entry(key.clone())
            .or_insert(ProviderProfileStateEntry {
                reason: ProviderFailoverReason::RequestRejected,
                failure_count: 0,
                cooldown_until: None,
                disabled_until: None,
                last_used_at: None,
            });
        entry.failure_count = 0;
        entry.cooldown_until = None;
        entry.disabled_until = None;
        entry.last_used_at = Some(now);
        self.touch_order(key, max_entries);
        self.bump_revision();
    }

    pub(super) fn mark_failure(
        &mut self,
        key: String,
        reason: ProviderFailoverReason,
        now: Instant,
        policy: &ProviderProfileStatePolicy,
    ) {
        self.prune_expired(now);
        let entry = self
            .entries
            .entry(key.clone())
            .or_insert(ProviderProfileStateEntry {
                reason,
                failure_count: 0,
                cooldown_until: None,
                disabled_until: None,
                last_used_at: None,
            });
        entry.reason = reason;
        entry.failure_count = entry.failure_count.saturating_add(1);

        match policy.health_mode {
            ProviderProfileHealthMode::EnforceUnusableWindows => {
                if matches!(reason, ProviderFailoverReason::AuthRejected) {
                    entry.disabled_until = now.checked_add(policy.auth_reject_disable);
                    entry.cooldown_until = None;
                } else {
                    let bounded_max_cooldown = policy.max_cooldown.max(policy.cooldown);
                    let exponent = entry.failure_count.saturating_sub(1).min(20);
                    let multiplier = 1u32 << exponent;
                    let effective_cooldown = policy
                        .cooldown
                        .saturating_mul(multiplier)
                        .min(bounded_max_cooldown);
                    entry.cooldown_until = now.checked_add(effective_cooldown);
                }
            }
            ProviderProfileHealthMode::ObserveOnly => {
                // Observe-only mode keeps failure counters but avoids hard profile suppression.
                entry.cooldown_until = None;
                entry.disabled_until = None;
                entry.last_used_at = Some(now);
            }
        }

        self.touch_order(key, policy.max_entries);
        self.bump_revision();
    }

    fn touch_order(&mut self, key: String, max_entries: usize) {
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key);
        self.prune_capacity(max_entries);
    }

    fn prune_expired(&mut self, now: Instant) {
        self.entries.retain(|_, entry| {
            let cooldown_active = entry.cooldown_until.is_some_and(|until| until > now);
            let disabled_active = entry.disabled_until.is_some_and(|until| until > now);
            cooldown_active || disabled_active || entry.last_used_at.is_some()
        });
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

    pub(super) fn to_snapshot(&self, now: Instant) -> ProviderProfileStateSnapshot {
        let mut entries = Vec::new();
        for (key, entry) in &self.entries {
            let cooldown_remaining_ms = remaining_duration_ms(entry.cooldown_until, now);
            let disabled_remaining_ms = remaining_duration_ms(entry.disabled_until, now);
            if cooldown_remaining_ms.is_none()
                && disabled_remaining_ms.is_none()
                && entry.last_used_at.is_none()
            {
                continue;
            }
            entries.push(ProviderProfileStateSnapshotEntry {
                key: key.clone(),
                reason: entry.reason.as_str().to_owned(),
                failure_count: entry.failure_count,
                cooldown_remaining_ms,
                disabled_remaining_ms,
                last_used_age_ms: elapsed_duration_ms(entry.last_used_at, now),
            });
        }
        entries.sort_unstable_by(|left, right| left.key.cmp(&right.key));

        let mut order = Vec::new();
        for key in &self.order {
            if entries.iter().any(|entry| &entry.key == key) {
                order.push(key.clone());
            }
        }
        for entry in &entries {
            if !order.iter().any(|key| key == &entry.key) {
                order.push(entry.key.clone());
            }
        }

        ProviderProfileStateSnapshot {
            version: PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
            revision: self.revision,
            generated_at_unix_ms: current_unix_timestamp_ms(),
            order,
            entries,
        }
    }

    pub(super) fn from_snapshot(snapshot: ProviderProfileStateSnapshot, now: Instant) -> Self {
        if snapshot.version != PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION {
            return Self::default();
        }

        let mut entries = HashMap::new();
        for record in snapshot.entries {
            let Some(reason) = ProviderFailoverReason::from_str(record.reason.as_str()) else {
                continue;
            };
            let cooldown_until = record
                .cooldown_remaining_ms
                .and_then(|ms| now.checked_add(Duration::from_millis(ms)));
            let disabled_until = record
                .disabled_remaining_ms
                .and_then(|ms| now.checked_add(Duration::from_millis(ms)));
            let last_used_at = record
                .last_used_age_ms
                .and_then(|ms| now.checked_sub(Duration::from_millis(ms)));
            entries.insert(
                record.key,
                ProviderProfileStateEntry {
                    reason,
                    failure_count: record.failure_count,
                    cooldown_until,
                    disabled_until,
                    last_used_at,
                },
            );
        }

        let mut order = VecDeque::new();
        for key in snapshot.order {
            if entries.contains_key(&key) && !order.iter().any(|existing| existing == &key) {
                order.push_back(key);
            }
        }
        for key in entries.keys() {
            if !order.iter().any(|existing| existing == key) {
                order.push_back(key.clone());
            }
        }

        let mut store = Self {
            entries,
            order,
            revision: snapshot.revision,
        };
        store.prune_expired(now);
        store
    }
}

fn remaining_duration_ms(until: Option<Instant>, now: Instant) -> Option<u64> {
    until
        .filter(|deadline| *deadline > now)
        .map(|deadline| deadline.saturating_duration_since(now).as_millis() as u64)
}

fn elapsed_duration_ms(since: Option<Instant>, now: Instant) -> Option<u64> {
    since.map(|instant| now.saturating_duration_since(instant).as_millis() as u64)
}

pub(super) fn current_unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
