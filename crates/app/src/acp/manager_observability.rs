use std::collections::BTreeMap;

use super::*;

impl AcpSessionManager {
    pub async fn observability_snapshot(
        &self,
        config: &LoongConfig,
    ) -> CliResult<AcpManagerObservabilitySnapshot> {
        self.cleanup_idle_sessions(config).await?;

        let sessions = self.store.list()?;
        let active_sessions = sessions.len();
        let mut bound_sessions = 0usize;
        let mut unbound_sessions = 0usize;
        let mut activation_origin_counts = BTreeMap::new();
        let mut backend_counts = BTreeMap::new();
        for metadata in &sessions {
            if metadata.binding.is_some() {
                bound_sessions = bound_sessions.saturating_add(1);
            } else {
                unbound_sessions = unbound_sessions.saturating_add(1);
            }
            if let Some(origin) = metadata.activation_origin {
                bump_usize_count(&mut activation_origin_counts, origin.as_str());
            }
            bump_usize_count(&mut backend_counts, metadata.backend_id.as_str());
        }
        let (actor_active, actor_queue_depth, actor_waiting) = {
            let guard = self
                .actor_ref_counts
                .read()
                .map_err(|_error| "ACP actor reference registry lock poisoned".to_owned())?;
            let queue_depth = guard.values().copied().sum();
            let waiting = guard
                .values()
                .copied()
                .map(|count| count.saturating_sub(1))
                .sum();
            (guard.len(), queue_depth, waiting)
        };
        let queue_depth = {
            let guard = self
                .pending_turns
                .read()
                .map_err(|_error| "ACP pending turn registry lock poisoned".to_owned())?;
            guard.values().copied().sum()
        };
        let active = self
            .active_turns
            .read()
            .map_err(|_error| "ACP active turn registry lock poisoned".to_owned())?
            .len();
        let latency = *self
            .turn_latency_stats
            .read()
            .map_err(|_error| "ACP turn latency registry lock poisoned".to_owned())?;
        let total_turns = latency.completed + latency.failed;
        let average_latency_ms = if total_turns > 0 {
            latency.total_ms / total_turns
        } else {
            0
        };
        let errors_by_code = self
            .error_counts_by_code
            .read()
            .map_err(|_error| "ACP error registry lock poisoned".to_owned())?
            .clone();
        let evicted_total = *self
            .evicted_runtime_count
            .read()
            .map_err(|_error| "ACP eviction counter lock poisoned".to_owned())?;
        let last_evicted_at_ms = *self
            .last_evicted_at_ms
            .read()
            .map_err(|_error| "ACP last eviction lock poisoned".to_owned())?;

        Ok(AcpManagerObservabilitySnapshot {
            runtime_cache: AcpManagerRuntimeCacheSnapshot {
                active_sessions,
                idle_ttl_ms: config.acp.session_idle_ttl_ms(),
                evicted_total,
                last_evicted_at_ms,
            },
            sessions: AcpManagerSessionSnapshot {
                bound: bound_sessions,
                unbound: unbound_sessions,
                activation_origin_counts,
                backend_counts,
            },
            actors: AcpManagerActorSnapshot {
                active: actor_active,
                queue_depth: actor_queue_depth,
                waiting: actor_waiting,
            },
            turns: AcpManagerTurnSnapshot {
                active,
                queue_depth,
                completed: latency.completed,
                failed: latency.failed,
                average_latency_ms,
                max_latency_ms: latency.max_ms,
            },
            errors_by_code,
        })
    }

    pub async fn doctor(
        &self,
        config: &LoongConfig,
        backend_id: Option<&str>,
    ) -> CliResult<AcpDoctorReport> {
        let selected = backend_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| resolve_acp_backend_selection(config).id);
        let backend = resolve_acp_backend(Some(selected.as_str()))?;

        if let Some(report) = backend.doctor(config).await? {
            return Ok(report);
        }

        Ok(AcpDoctorReport {
            healthy: true,
            diagnostics: BTreeMap::from([
                ("backend".to_owned(), backend.id().to_owned()),
                ("status".to_owned(), "no_doctor".to_owned()),
            ]),
        })
    }
}
