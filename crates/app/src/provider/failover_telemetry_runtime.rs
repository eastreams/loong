#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use loong_kernel::AuditEventKind;

use crate::config::ProviderConfig;

use super::failover::ProviderFailoverSnapshot;
use super::runtime_binding::ProviderRuntimeBinding;

#[derive(Debug, Clone)]
struct ProviderFailoverEvent {
    provider_id: String,
    reason: String,
    stage: String,
    model: String,
    attempt: usize,
    max_attempts: usize,
    status_code: Option<u16>,
    try_next_model: bool,
    auto_model_mode: bool,
    candidate_index: usize,
    candidate_count: usize,
    exhausted: bool,
}

#[derive(Debug, Default)]
struct ProviderFailoverMetrics {
    total_events: usize,
    continued_events: usize,
    exhausted_events: usize,
    by_reason: HashMap<String, usize>,
    by_stage: HashMap<String, usize>,
    by_provider: HashMap<String, usize>,
}

impl ProviderFailoverMetrics {
    fn record(&mut self, event: &ProviderFailoverEvent) {
        self.total_events = self.total_events.saturating_add(1);
        if event.exhausted {
            self.exhausted_events = self.exhausted_events.saturating_add(1);
        } else {
            self.continued_events = self.continued_events.saturating_add(1);
        }
        *self.by_reason.entry(event.reason.clone()).or_insert(0) += 1;
        *self.by_stage.entry(event.stage.clone()).or_insert(0) += 1;
        *self
            .by_provider
            .entry(event.provider_id.clone())
            .or_insert(0) += 1;
    }

    #[cfg(test)]
    fn snapshot(&self) -> ProviderFailoverMetricsSnapshot {
        ProviderFailoverMetricsSnapshot {
            total_events: self.total_events,
            continued_events: self.continued_events,
            exhausted_events: self.exhausted_events,
            by_reason: self
                .by_reason
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect::<BTreeMap<_, _>>(),
            by_stage: self
                .by_stage
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect::<BTreeMap<_, _>>(),
            by_provider: self
                .by_provider
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect::<BTreeMap<_, _>>(),
        }
    }
}

static PROVIDER_FAILOVER_METRICS: OnceLock<Mutex<ProviderFailoverMetrics>> = OnceLock::new();

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ProviderFailoverMetricsSnapshot {
    pub(super) total_events: usize,
    pub(super) continued_events: usize,
    pub(super) exhausted_events: usize,
    pub(super) by_reason: BTreeMap<String, usize>,
    pub(super) by_stage: BTreeMap<String, usize>,
    pub(super) by_provider: BTreeMap<String, usize>,
}

fn with_provider_failover_metrics<R>(run: impl FnOnce(&mut ProviderFailoverMetrics) -> R) -> R {
    let metrics =
        PROVIDER_FAILOVER_METRICS.get_or_init(|| Mutex::new(ProviderFailoverMetrics::default()));
    let mut guard = match metrics.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    run(&mut guard)
}

fn record_provider_failover_metrics(event: &ProviderFailoverEvent) {
    with_provider_failover_metrics(|metrics| metrics.record(event));
}

#[cfg(test)]
pub(super) fn provider_failover_metrics_snapshot() -> ProviderFailoverMetricsSnapshot {
    with_provider_failover_metrics(|metrics| metrics.snapshot())
}

fn build_provider_failover_event(
    provider: &ProviderConfig,
    snapshot: &ProviderFailoverSnapshot,
    try_next_model: bool,
    auto_model_mode: bool,
    candidate_index: usize,
    candidate_count: usize,
    exhausted: bool,
) -> ProviderFailoverEvent {
    ProviderFailoverEvent {
        provider_id: provider.kind.profile().id.to_owned(),
        reason: snapshot.reason.as_str().to_owned(),
        stage: snapshot.stage.as_str().to_owned(),
        model: snapshot.model.clone(),
        attempt: snapshot.attempt,
        max_attempts: snapshot.max_attempts,
        status_code: snapshot.status_code,
        try_next_model,
        auto_model_mode,
        candidate_index,
        candidate_count,
        exhausted,
    }
}

pub(super) fn record_provider_failover_audit_event(
    binding: ProviderRuntimeBinding<'_>,
    provider: &ProviderConfig,
    snapshot: &ProviderFailoverSnapshot,
    try_next_model: bool,
    auto_model_mode: bool,
    candidate_index: usize,
    candidate_count: usize,
    exhausted: bool,
) {
    let event = build_provider_failover_event(
        provider,
        snapshot,
        try_next_model,
        auto_model_mode,
        candidate_index,
        candidate_count,
        exhausted,
    );
    record_provider_failover_metrics(&event);

    let Some(ctx) = binding.kernel_context() else {
        return;
    };
    let _ = ctx.kernel.record_audit_event(
        Some(ctx.agent_id()),
        AuditEventKind::ProviderFailover {
            pack_id: ctx.pack_id().to_owned(),
            provider_id: event.provider_id,
            reason: event.reason,
            stage: event.stage,
            model: event.model,
            attempt: event.attempt,
            max_attempts: event.max_attempts,
            status_code: event.status_code,
            try_next_model: event.try_next_model,
            auto_model_mode: event.auto_model_mode,
            candidate_index: event.candidate_index,
            candidate_count: event.candidate_count,
        },
    );
}
