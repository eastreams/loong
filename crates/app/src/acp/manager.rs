use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::CliResult;
use crate::config::LoongClawConfig;

#[path = "manager_actor.rs"]
mod actor;
#[path = "manager_sessions.rs"]
mod sessions;
#[path = "manager_support.rs"]
mod support;

use self::support::*;
pub use self::support::{
    AcpManagerActorSnapshot, AcpManagerObservabilitySnapshot, AcpManagerRuntimeCacheSnapshot,
    AcpManagerSessionSnapshot, AcpManagerTurnSnapshot,
};
use super::backend::{
    ACP_SESSION_METADATA_ACTIVATION_ORIGIN, ACP_TURN_METADATA_TRACE_ID, AcpAbortController,
    AcpConfigPatch, AcpDoctorReport, AcpRoutingOrigin, AcpSessionBootstrap, AcpSessionHandle,
    AcpSessionMetadata, AcpSessionMode, AcpSessionState, AcpSessionStatus, AcpTurnEventSink,
    AcpTurnRequest, AcpTurnResult, BufferedAcpTurnEventSink, CompositeAcpTurnEventSink,
};
use super::binding::AcpSessionBindingScope;
use super::merge_turn_events;
use super::registry::resolve_acp_backend;
use super::runtime::resolve_acp_backend_selection;
use super::store::{AcpSessionStore, InMemoryAcpSessionStore};

#[derive(Debug)]
struct ActiveTurnState {
    handle: AcpSessionHandle,
    abort_controller: AcpAbortController,
}

pub struct AcpSessionManager {
    store: Arc<dyn AcpSessionStore>,
    active_turns: RwLock<BTreeMap<String, Arc<ActiveTurnState>>>,
    session_actor_locks: Arc<RwLock<BTreeMap<String, Arc<AsyncMutex<()>>>>>,
    actor_ref_counts: Arc<RwLock<BTreeMap<String, usize>>>,
    pending_turns: Arc<RwLock<BTreeMap<String, usize>>>,
    turn_latency_stats: RwLock<TurnLatencyStats>,
    error_counts_by_code: RwLock<BTreeMap<String, usize>>,
    evicted_runtime_count: RwLock<u64>,
    last_evicted_at_ms: RwLock<Option<u64>>,
}

impl Default for AcpSessionManager {
    fn default() -> Self {
        Self::new(Arc::new(InMemoryAcpSessionStore::default()))
    }
}

impl AcpSessionManager {
    pub fn new(store: Arc<dyn AcpSessionStore>) -> Self {
        Self {
            store,
            active_turns: RwLock::new(BTreeMap::new()),
            session_actor_locks: Arc::new(RwLock::new(BTreeMap::new())),
            actor_ref_counts: Arc::new(RwLock::new(BTreeMap::new())),
            pending_turns: Arc::new(RwLock::new(BTreeMap::new())),
            turn_latency_stats: RwLock::new(TurnLatencyStats::default()),
            error_counts_by_code: RwLock::new(BTreeMap::new()),
            evicted_runtime_count: RwLock::new(0),
            last_evicted_at_ms: RwLock::new(None),
        }
    }

    pub async fn ensure_session(
        &self,
        config: &LoongClawConfig,
        bootstrap: &AcpSessionBootstrap,
    ) -> CliResult<AcpSessionMetadata> {
        self.cleanup_idle_sessions(config).await?;

        let selection = resolve_acp_backend_selection(config);
        let binding_scope = AcpSessionBindingScope::from_bootstrap(bootstrap);
        let has_conversation_id = bootstrap.conversation_id.is_some();
        let redacted_conversation_id =
            redact_identifier_for_log(bootstrap.conversation_id.as_deref());
        let redacted_binding_scope = redact_binding_scope_for_log(binding_scope.as_ref());
        tracing::debug!(
            target: "loongclaw.acp",
            backend_id = %selection.id,
            has_conversation_id,
            mode = ?bootstrap.mode,
            conversation_id = ?redacted_conversation_id,
            binding = ?redacted_binding_scope,
            "ensuring ACP session"
        );
        if let Some(existing) =
            self.resolve_existing_session(config, selection.id.as_str(), bootstrap)?
        {
            tracing::debug!(
                target: "loongclaw.acp",
                backend_id = %existing.backend_id,
                state = ?existing.state,
                "reused ACP session"
            );
            return Ok(existing);
        }

        self.enforce_max_concurrent_sessions(config, bootstrap.session_key.as_str())?;

        let backend = resolve_acp_backend(Some(selection.id.as_str()))?;
        let handle = backend.ensure_session(config, bootstrap).await?;
        let mut metadata = handle.into_metadata(
            normalized_conversation_id(bootstrap.conversation_id.as_deref()),
            binding_scope,
            bootstrap.mode,
            AcpSessionState::Ready,
        );
        metadata.activation_origin = bootstrap
            .metadata
            .get(ACP_SESSION_METADATA_ACTIVATION_ORIGIN)
            .and_then(|value| AcpRoutingOrigin::parse(value));
        self.store.upsert(metadata.clone())?;
        tracing::debug!(
            target: "loongclaw.acp",
            backend_id = %metadata.backend_id,
            activation_origin = ?metadata.activation_origin.map(AcpRoutingOrigin::as_str),
            "created ACP session"
        );
        Ok(metadata)
    }

    pub async fn run_turn(
        &self,
        config: &LoongClawConfig,
        bootstrap: &AcpSessionBootstrap,
        request: &AcpTurnRequest,
    ) -> CliResult<AcpTurnResult> {
        self.run_turn_with_sink(config, bootstrap, request, None)
            .await
    }

    pub async fn run_turn_with_sink(
        &self,
        config: &LoongClawConfig,
        bootstrap: &AcpSessionBootstrap,
        request: &AcpTurnRequest,
        sink: Option<&dyn AcpTurnEventSink>,
    ) -> CliResult<AcpTurnResult> {
        let end_to_end_started_at = std::time::Instant::now();
        let actor_key = actor_key_for_bootstrap(bootstrap);
        let _turn_queue_guard = self.acquire_turn_queue_guard(actor_key.clone()).await?;
        self.cleanup_idle_sessions(config).await?;

        let mut metadata = self.ensure_session(config, bootstrap).await?;
        let trace_id = request
            .metadata
            .get(ACP_TURN_METADATA_TRACE_ID)
            .map(String::as_str);
        let redacted_trace_id = redact_identifier_for_log(trace_id);
        let has_trace_id = redacted_trace_id.is_some();
        tracing::debug!(
            target: "loongclaw.acp",
            backend_id = %metadata.backend_id,
            has_trace_id,
            input_len = request.input.chars().count(),
            sink_enabled = sink.is_some(),
            "starting ACP turn"
        );
        let backend = resolve_acp_backend(Some(metadata.backend_id.as_str()))?;
        metadata.state = AcpSessionState::Busy;
        metadata.clear_error();
        metadata.touch();
        self.store.upsert(metadata.clone())?;

        let handle = metadata.to_handle();
        let active_turn = Arc::new(ActiveTurnState {
            handle: handle.clone(),
            abort_controller: AcpAbortController::new(),
        });
        self.register_active_turn(actor_key.as_str(), active_turn.clone())?;
        let turn_started_ms = now_ms();
        let execution_started_at = std::time::Instant::now();
        let buffered_sink = BufferedAcpTurnEventSink::default();
        let result = match sink {
            Some(external_sink) => {
                let composite = CompositeAcpTurnEventSink {
                    primary: external_sink,
                    secondary: &buffered_sink,
                };
                backend
                    .run_turn_with_sink(
                        config,
                        &handle,
                        request,
                        Some(active_turn.abort_controller.signal()),
                        Some(&composite),
                    )
                    .await
            }
            None => {
                backend
                    .run_turn_with_sink(
                        config,
                        &handle,
                        request,
                        Some(active_turn.abort_controller.signal()),
                        Some(&buffered_sink),
                    )
                    .await
            }
        };

        self.clear_active_turn(actor_key.as_str())?;
        let end_to_end_duration_ms = end_to_end_started_at.elapsed().as_millis();
        let execution_duration_ms = execution_started_at.elapsed().as_millis();
        let queue_wait_ms = end_to_end_duration_ms.saturating_sub(execution_duration_ms);
        match result {
            Ok(mut result) => {
                self.record_turn_completion(turn_started_ms, true)?;
                let streamed_events = buffered_sink.snapshot()?;
                let reported_event_count = result.events.len();
                let streamed_event_count = streamed_events.len();
                result.events = merge_turn_events(&result.events, &streamed_events);
                metadata.state = result.state;
                metadata.clear_error();
                metadata.touch();
                self.store.upsert(metadata)?;
                tracing::debug!(
                    target: "loongclaw.acp",
                    backend_id = %handle.backend_id,
                    has_trace_id,
                    state = ?result.state,
                    stop_reason = ?result.stop_reason,
                    reported_event_count,
                    streamed_event_count,
                    merged_event_count = result.events.len(),
                    end_to_end_duration_ms,
                    execution_duration_ms,
                    queue_wait_ms,
                    "ACP turn completed"
                );
                Ok(result)
            }
            Err(error) => {
                self.record_turn_completion(turn_started_ms, false)?;
                self.record_error(error.as_str())?;
                metadata.state = AcpSessionState::Error;
                metadata.set_error(error.clone());
                self.store.upsert(metadata)?;
                tracing::warn!(
                    target: "loongclaw.acp",
                    backend_id = %handle.backend_id,
                    trace_id = ?redacted_trace_id,
                    has_trace_id,
                    end_to_end_duration_ms,
                    execution_duration_ms,
                    queue_wait_ms,
                    error = %crate::observability::summarize_error(error.as_str()),
                    "ACP turn failed"
                );
                Err(error)
            }
        }
    }

    pub async fn get_status(
        &self,
        config: &LoongClawConfig,
        session_key: &str,
    ) -> CliResult<AcpSessionStatus> {
        let registered = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let active_turn = self.is_active_turn_for_metadata(&registered)?;
        let pending_turns = self.pending_turn_count_for_metadata(&registered)?;
        if active_turn || pending_turns > 0 {
            return Ok(self.fallback_status(&registered, active_turn, pending_turns));
        }
        let actor_key = actor_key_for_metadata(&registered);
        let _actor_guard = self.acquire_session_actor_guard(actor_key).await?;
        self.cleanup_idle_sessions(config).await?;

        let mut metadata = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let active_turn = self.is_active_turn_for_metadata(&metadata)?;
        let pending_turns = self.pending_turn_count_for_metadata(&metadata)?;
        if active_turn || pending_turns > 0 {
            return Ok(self.fallback_status(&metadata, active_turn, pending_turns));
        }
        let backend = resolve_acp_backend(Some(metadata.backend_id.as_str()))?;

        match backend.get_status(config, &metadata.to_handle()).await {
            Ok(Some(mut status)) => {
                if status.backend_id.trim().is_empty() {
                    status.backend_id = metadata.backend_id.clone();
                }
                if status.conversation_id.is_none() {
                    status.conversation_id = metadata.conversation_id.clone();
                }
                if status.binding.is_none() {
                    status.binding = metadata.binding.clone();
                }
                if status.activation_origin.is_none() {
                    status.activation_origin = metadata.activation_origin;
                }
                if status.mode.is_none() {
                    status.mode = metadata.mode;
                }
                if status.last_activity_ms == 0 {
                    status.last_activity_ms = metadata.last_activity_ms;
                }
                if status.last_error.is_none() {
                    status.last_error = metadata.last_error.clone();
                }
                if active_turn || pending_turns > 0 {
                    status.pending_turns = status
                        .pending_turns
                        .max(pending_turns)
                        .max(usize::from(active_turn));
                    status.state =
                        projected_status_state(status.state, active_turn, status.pending_turns);
                }
                if active_turn && status.active_turn_id.is_none() {
                    status.active_turn_id = Some(metadata.runtime_session_name.clone());
                }

                metadata.state = status.state;
                metadata.mode = status.mode;
                metadata.last_activity_ms = status.last_activity_ms.max(now_ms());
                metadata.last_error = status.last_error.clone();
                self.store.upsert(metadata)?;
                Ok(status)
            }
            Ok(None) => Ok(self.fallback_status(&metadata, active_turn, pending_turns)),
            Err(error) => {
                self.record_error(error.as_str())?;
                metadata.state = AcpSessionState::Error;
                metadata.set_error(error.clone());
                self.store.upsert(metadata)?;
                Err(error)
            }
        }
    }

    pub async fn set_mode(
        &self,
        config: &LoongClawConfig,
        session_key: &str,
        mode: AcpSessionMode,
    ) -> CliResult<()> {
        let registered = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let actor_key = actor_key_for_metadata(&registered);
        let _actor_guard = self.acquire_session_actor_guard(actor_key).await?;
        self.cleanup_idle_sessions(config).await?;

        let mut metadata = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let backend = resolve_acp_backend(Some(metadata.backend_id.as_str()))?;
        match backend.set_mode(config, &metadata.to_handle(), mode).await {
            Ok(()) => {
                metadata.mode = Some(mode);
                metadata.clear_error();
                metadata.touch();
                self.store.upsert(metadata)
            }
            Err(error) => {
                self.record_error(error.as_str())?;
                metadata.state = AcpSessionState::Error;
                metadata.set_error(error.clone());
                self.store.upsert(metadata)?;
                Err(error)
            }
        }
    }

    pub async fn set_config_option(
        &self,
        config: &LoongClawConfig,
        session_key: &str,
        patch: &AcpConfigPatch,
    ) -> CliResult<()> {
        let registered = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let actor_key = actor_key_for_metadata(&registered);
        let _actor_guard = self.acquire_session_actor_guard(actor_key).await?;
        self.cleanup_idle_sessions(config).await?;

        let mut metadata = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let backend = resolve_acp_backend(Some(metadata.backend_id.as_str()))?;
        match backend
            .set_config_option(config, &metadata.to_handle(), patch)
            .await
        {
            Ok(()) => {
                metadata.clear_error();
                metadata.touch();
                self.store.upsert(metadata)
            }
            Err(error) => {
                self.record_error(error.as_str())?;
                metadata.state = AcpSessionState::Error;
                metadata.set_error(error.clone());
                self.store.upsert(metadata)?;
                Err(error)
            }
        }
    }

    pub async fn cancel(&self, config: &LoongClawConfig, session_key: &str) -> CliResult<()> {
        let registered = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let actor_key = actor_key_for_metadata(&registered);

        if let Some(active_turn) = self.active_turn(actor_key.as_str())? {
            return self
                .request_active_turn_cancellation(config, registered, active_turn)
                .await;
        }

        let _actor_guard = self.acquire_session_actor_guard(actor_key).await?;
        self.cleanup_idle_sessions(config).await?;

        let mut metadata = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let backend = resolve_acp_backend(Some(metadata.backend_id.as_str()))?;
        match backend.cancel(config, &metadata.to_handle()).await {
            Ok(()) => {
                metadata.state = AcpSessionState::Cancelling;
                metadata.clear_error();
                metadata.touch();
                self.store.upsert(metadata)
            }
            Err(error) => {
                self.record_error(error.as_str())?;
                metadata.state = AcpSessionState::Error;
                metadata.set_error(error.clone());
                self.store.upsert(metadata)?;
                Err(error)
            }
        }
    }

    pub async fn close(&self, config: &LoongClawConfig, session_key: &str) -> CliResult<()> {
        let registered = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let actor_key = actor_key_for_metadata(&registered);

        if let Some(active_turn) = self.active_turn(actor_key.as_str())? {
            self.request_active_turn_cancellation(config, registered, active_turn)
                .await?;
        }

        let _actor_guard = self.acquire_session_actor_guard(actor_key).await?;
        self.cleanup_idle_sessions(config).await?;

        let metadata = self
            .store
            .get(session_key)?
            .ok_or_else(|| format!("ACP session `{session_key}` is not registered"))?;
        let backend = resolve_acp_backend(Some(metadata.backend_id.as_str()))?;
        backend.close(config, &metadata.to_handle()).await?;
        self.clear_active_turn(actor_key_for_metadata(&metadata).as_str())?;
        self.store.remove(session_key)
    }

    pub async fn observability_snapshot(
        &self,
        config: &LoongClawConfig,
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
        config: &LoongClawConfig,
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

    pub fn list_sessions(&self) -> CliResult<Vec<AcpSessionMetadata>> {
        self.store.list()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::{Value, json};
    use tokio::sync::Notify;

    use super::super::{
        AcpAbortSignal, AcpBackendMetadata, AcpCapability, AcpRuntimeBackend, AcpSessionBootstrap,
        AcpSessionHandle, AcpSessionMode, AcpSessionState, AcpSessionStatus, AcpSessionStore,
        AcpTurnRequest, AcpTurnResult, AcpTurnStopReason, register_acp_backend,
    };
    use crate::CliResult;
    use crate::config::{AcpConfig, LoongClawConfig};

    #[cfg(feature = "memory-sqlite")]
    use super::super::AcpSqliteSessionStore;
    use super::{AcpSessionManager, redact_binding_scope_for_log, redact_identifier_for_log};

    #[derive(Default)]
    struct CountingState {
        ensure_calls: usize,
        turn_calls: usize,
        cancel_calls: usize,
        close_calls: usize,
    }

    fn install_manager_backends(
        counting_id: &'static str,
        alt_id: &'static str,
    ) -> Arc<Mutex<CountingState>> {
        let shared = Arc::new(Mutex::new(CountingState::default()));

        register_acp_backend(counting_id, {
            let state = shared.clone();
            move || {
                Box::new(CountingBackend {
                    id: counting_id,
                    state: state.clone(),
                })
            }
        })
        .expect("register counting ACP backend");
        register_acp_backend(alt_id, move || Box::new(AlternateBackend { id: alt_id }))
            .expect("register alternate ACP backend");
        shared
    }

    #[test]
    fn redact_identifier_for_log_hashes_non_empty_values() {
        let left = redact_identifier_for_log(Some("  conversation-42 "));
        let right = redact_identifier_for_log(Some("conversation-42"));
        let different = redact_identifier_for_log(Some("conversation-43"));

        assert_eq!(left, right);
        assert_ne!(left, different);
        assert!(
            left.as_deref()
                .is_some_and(|value| value.starts_with("sha256:"))
        );
        assert_eq!(redact_identifier_for_log(Some("   ")), None);
    }

    #[test]
    fn redact_binding_scope_for_log_redacts_sensitive_identifiers() {
        let binding = super::super::AcpSessionBindingScope {
            route_session_id: "telegram:acct:42".to_owned(),
            channel_id: Some("telegram".to_owned()),
            account_id: Some("acct".to_owned()),
            conversation_id: Some("42".to_owned()),
            participant_id: None,
            thread_id: Some("thread-1".to_owned()),
        };

        let redacted = redact_binding_scope_for_log(Some(&binding))
            .expect("binding log redaction should return a value");

        assert_eq!(redacted.channel_id.as_deref(), Some("telegram"));
        assert_ne!(redacted.route_session_id, binding.route_session_id);
        assert_ne!(redacted.account_id, binding.account_id);
        assert_ne!(redacted.conversation_id, binding.conversation_id);
        assert_ne!(redacted.thread_id, binding.thread_id);
    }

    struct CountingBackend {
        id: &'static str,
        state: Arc<Mutex<CountingState>>,
    }

    struct AlternateBackend {
        id: &'static str,
    }

    struct FailingBackend {
        id: &'static str,
    }

    struct CloseFailureBackend {
        id: &'static str,
        state: Arc<CloseFailureState>,
    }

    struct QueuedTurnBackend {
        id: &'static str,
        state: Arc<QueuedTurnState>,
    }

    struct SerializedControlBackend {
        id: &'static str,
        state: Arc<SerializedControlState>,
    }

    struct SlowControlBackend {
        id: &'static str,
        state: Arc<SlowControlState>,
    }

    struct AbortableTurnBackend {
        id: &'static str,
        state: Arc<AbortableTurnState>,
    }

    struct QueuedTurnState {
        first_turn_entered: Notify,
        release_first_turn: Notify,
        active_turns: AtomicUsize,
        max_active_turns: AtomicUsize,
        inputs: Mutex<Vec<String>>,
    }

    struct SerializedControlState {
        first_turn_entered: Notify,
        release_first_turn: Notify,
        set_mode_calls: AtomicUsize,
        status_calls: AtomicUsize,
        events: Mutex<Vec<String>>,
    }

    struct SlowControlState {
        set_mode_entered: Notify,
        release_set_mode: Notify,
        set_mode_calls: AtomicUsize,
    }

    struct AbortableTurnState {
        turn_entered: Notify,
        abort_entered: Notify,
        release_abort_completion: Notify,
        hold_after_abort: AtomicBool,
        cancel_calls: AtomicUsize,
        close_calls: AtomicUsize,
        abort_observed: AtomicUsize,
    }

    #[derive(Default)]
    struct CloseFailureState {
        close_calls: AtomicUsize,
    }

    #[derive(Default)]
    struct StreamingSinkState {
        sink_calls: AtomicUsize,
    }

    impl Default for QueuedTurnState {
        fn default() -> Self {
            Self {
                first_turn_entered: Notify::new(),
                release_first_turn: Notify::new(),
                active_turns: AtomicUsize::new(0),
                max_active_turns: AtomicUsize::new(0),
                inputs: Mutex::new(Vec::new()),
            }
        }
    }

    impl Default for SerializedControlState {
        fn default() -> Self {
            Self {
                first_turn_entered: Notify::new(),
                release_first_turn: Notify::new(),
                set_mode_calls: AtomicUsize::new(0),
                status_calls: AtomicUsize::new(0),
                events: Mutex::new(Vec::new()),
            }
        }
    }

    impl Default for SlowControlState {
        fn default() -> Self {
            Self {
                set_mode_entered: Notify::new(),
                release_set_mode: Notify::new(),
                set_mode_calls: AtomicUsize::new(0),
            }
        }
    }

    impl Default for AbortableTurnState {
        fn default() -> Self {
            Self {
                turn_entered: Notify::new(),
                abort_entered: Notify::new(),
                release_abort_completion: Notify::new(),
                hold_after_abort: AtomicBool::new(false),
                cancel_calls: AtomicUsize::new(0),
                close_calls: AtomicUsize::new(0),
                abort_observed: AtomicUsize::new(0),
            }
        }
    }

    async fn wait_for_actor_counts(
        manager: &AcpSessionManager,
        actor_key: &str,
        expected_ref_count: usize,
        expected_pending_turns: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ref_count = manager
                    .actor_ref_count(actor_key)
                    .expect("actor ref count should read");
                let pending_turns = manager
                    .pending_turn_count(actor_key)
                    .expect("pending turn count should read");
                if ref_count == expected_ref_count && pending_turns == expected_pending_turns {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actor counts should converge");
    }

    struct StreamingSinkBackend {
        id: &'static str,
        state: Arc<StreamingSinkState>,
    }

    #[derive(Default)]
    struct RecordingTurnEventSink {
        events: Mutex<Vec<Value>>,
    }

    impl super::super::AcpTurnEventSink for RecordingTurnEventSink {
        fn on_event(&self, event: &Value) -> CliResult<()> {
            self.events
                .lock()
                .expect("recording turn event sink")
                .push(event.clone());
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for CountingBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [
                    AcpCapability::SessionLifecycle,
                    AcpCapability::TurnExecution,
                    AcpCapability::StatusInspection,
                ],
                "Counting ACP backend for manager tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            self.state.lock().expect("counting state").ensure_calls += 1;
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("runtime-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("backend-{}", request.session_key)),
                agent_session_id: Some(format!("agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            self.state.lock().expect("counting state").turn_calls += 1;
            Ok(AcpTurnResult {
                output_text: format!("echo: {}", request.input),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            self.state.lock().expect("counting state").cancel_calls += 1;
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            self.state.lock().expect("counting state").close_calls += 1;
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for AlternateBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(self.id(), [AcpCapability::SessionLifecycle], "Alt backend")
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("alt-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: None,
                agent_session_id: None,
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Ok(AcpTurnResult {
                output_text: request.input.clone(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for FailingBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("fail-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: None,
                agent_session_id: None,
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            _request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Err("synthetic ACP turn failure".to_owned())
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for CloseFailureBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [AcpCapability::SessionLifecycle],
                "Close failure ACP backend for manager tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("close-failure-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: None,
                agent_session_id: None,
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Ok(AcpTurnResult {
                output_text: request.input.clone(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            self.state.close_calls.fetch_add(1, Ordering::SeqCst);
            Err("synthetic ACP close failure".to_owned())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for QueuedTurnBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [
                    AcpCapability::SessionLifecycle,
                    AcpCapability::TurnExecution,
                    AcpCapability::StatusInspection,
                ],
                "Queued ACP backend for manager tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("queued-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("queued-backend-{}", request.session_key)),
                agent_session_id: Some(format!("queued-agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            self.state
                .inputs
                .lock()
                .expect("queued turn inputs")
                .push(request.input.clone());
            let active_turns = self.state.active_turns.fetch_add(1, Ordering::SeqCst) + 1;
            self.state
                .max_active_turns
                .fetch_max(active_turns, Ordering::SeqCst);

            if request.input == "first" {
                self.state.first_turn_entered.notify_waiters();
                self.state.release_first_turn.notified().await;
            }

            self.state.active_turns.fetch_sub(1, Ordering::SeqCst);
            Ok(AcpTurnResult {
                output_text: format!("queued: {}", request.input),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for SerializedControlBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [
                    AcpCapability::SessionLifecycle,
                    AcpCapability::TurnExecution,
                    AcpCapability::StatusInspection,
                    AcpCapability::ModeSwitching,
                ],
                "Serialized control backend for manager tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("serialized-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("serialized-backend-{}", request.session_key)),
                agent_session_id: Some(format!("serialized-agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            self.state
                .events
                .lock()
                .expect("serialized control events")
                .push(format!("turn:{}", request.input));
            if request.input == "first" {
                self.state.first_turn_entered.notify_waiters();
                self.state.release_first_turn.notified().await;
            }
            Ok(AcpTurnResult {
                output_text: format!("serialized: {}", request.input),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn set_mode(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            mode: AcpSessionMode,
        ) -> CliResult<()> {
            self.state.set_mode_calls.fetch_add(1, Ordering::SeqCst);
            let mode_label = match mode {
                AcpSessionMode::Interactive => "interactive",
                AcpSessionMode::Background => "background",
                AcpSessionMode::Review => "review",
            };
            self.state
                .events
                .lock()
                .expect("serialized control events")
                .push(format!("set-mode:{mode_label}"));
            Ok(())
        }

        async fn get_status(
            &self,
            _config: &LoongClawConfig,
            session: &AcpSessionHandle,
        ) -> CliResult<Option<AcpSessionStatus>> {
            self.state.status_calls.fetch_add(1, Ordering::SeqCst);
            self.state
                .events
                .lock()
                .expect("serialized control events")
                .push("status".to_owned());
            Ok(Some(AcpSessionStatus {
                session_key: session.session_key.clone(),
                backend_id: self.id().to_owned(),
                conversation_id: None,
                binding: session.binding.clone(),
                activation_origin: None,
                state: AcpSessionState::Ready,
                mode: None,
                pending_turns: 0,
                active_turn_id: None,
                last_activity_ms: super::now_ms(),
                last_error: None,
            }))
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for SlowControlBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [
                    AcpCapability::SessionLifecycle,
                    AcpCapability::ModeSwitching,
                ],
                "Slow control backend for manager cleanup tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("slow-control-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("slow-control-backend-{}", request.session_key)),
                agent_session_id: Some(format!("slow-control-agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Ok(AcpTurnResult {
                output_text: request.input.clone(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn set_mode(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            _mode: AcpSessionMode,
        ) -> CliResult<()> {
            self.state.set_mode_calls.fetch_add(1, Ordering::SeqCst);
            self.state.set_mode_entered.notify_waiters();
            self.state.release_set_mode.notified().await;
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for AbortableTurnBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [
                    AcpCapability::SessionLifecycle,
                    AcpCapability::TurnExecution,
                    AcpCapability::TurnEventStreaming,
                    AcpCapability::Cancellation,
                ],
                "Abortable ACP backend for manager tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("abortable-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("abortable-backend-{}", request.session_key)),
                agent_session_id: Some(format!("abortable-agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Ok(AcpTurnResult {
                output_text: request.input.clone(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn run_turn_with_sink(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            _request: &AcpTurnRequest,
            abort: Option<AcpAbortSignal>,
            _sink: Option<&dyn super::super::AcpTurnEventSink>,
        ) -> CliResult<AcpTurnResult> {
            self.state.turn_entered.notify_waiters();
            let Some(mut abort) = abort else {
                return Err("abortable manager test backend requires abort signal".to_owned());
            };
            abort.cancelled().await;
            self.state.abort_observed.fetch_add(1, Ordering::SeqCst);
            self.state.abort_entered.notify_waiters();
            if self.state.hold_after_abort.load(Ordering::SeqCst) {
                self.state.release_abort_completion.notified().await;
            }
            Ok(AcpTurnResult {
                output_text: String::new(),
                state: AcpSessionState::Ready,
                usage: None,
                events: vec![json!({
                    "type": "done",
                    "stopReason": "cancelled",
                })],
                stop_reason: Some(AcpTurnStopReason::Cancelled),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            self.state.cancel_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            self.state.close_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl AcpRuntimeBackend for StreamingSinkBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> AcpBackendMetadata {
            AcpBackendMetadata::new(
                self.id(),
                [
                    AcpCapability::SessionLifecycle,
                    AcpCapability::TurnExecution,
                    AcpCapability::TurnEventStreaming,
                ],
                "Streaming sink backend for manager tests",
            )
        }

        async fn ensure_session(
            &self,
            _config: &LoongClawConfig,
            request: &AcpSessionBootstrap,
        ) -> CliResult<AcpSessionHandle> {
            Ok(AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("streaming-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("streaming-backend-{}", request.session_key)),
                agent_session_id: Some(format!("streaming-agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        }

        async fn run_turn(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
        ) -> CliResult<AcpTurnResult> {
            Ok(AcpTurnResult {
                output_text: request.input.clone(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn run_turn_with_sink(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
            request: &AcpTurnRequest,
            _abort: Option<AcpAbortSignal>,
            sink: Option<&dyn super::super::AcpTurnEventSink>,
        ) -> CliResult<AcpTurnResult> {
            if let Some(sink) = sink {
                sink.on_event(&json!({
                    "type": "text",
                    "content": format!("chunk:{}", request.input),
                }))?;
                sink.on_event(&json!({
                    "type": "done",
                    "stopReason": "completed",
                }))?;
            }
            self.state.sink_calls.fetch_add(1, Ordering::SeqCst);
            Ok(AcpTurnResult {
                output_text: format!("streamed: {}", request.input),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            })
        }

        async fn cancel(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn close(
            &self,
            _config: &LoongClawConfig,
            _session: &AcpSessionHandle,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn ensure_session_reuses_existing_metadata_without_respawning_backend() {
        let counts = install_manager_backends("manager-counting-reuse", "manager-alt-reuse");
        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-reuse".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-reuse".to_owned(),
            conversation_id: Some("conv-1".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let first = manager
            .ensure_session(&config, &bootstrap)
            .await
            .expect("first ensure");
        let second = manager
            .ensure_session(&config, &bootstrap)
            .await
            .expect("second ensure");

        assert_eq!(first.session_key, second.session_key);
        assert_eq!(first.backend_id, "manager-counting-reuse");
        assert_eq!(manager.list_sessions().expect("list sessions").len(), 1);
        assert_eq!(
            counts.lock().expect("counting state").ensure_calls,
            1,
            "existing metadata should be reused instead of respawning backend session"
        );
    }

    #[tokio::test]
    async fn ensure_session_reuses_bound_conversation_when_bindings_enabled() {
        let counts = install_manager_backends(
            "manager-counting-binding-reuse",
            "manager-alt-binding-reuse",
        );
        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-binding-reuse".to_owned()),
                bindings_enabled: true,
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let first_bootstrap = AcpSessionBootstrap {
            session_key: "session-bound-a".to_owned(),
            conversation_id: Some("telegram:1001".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let second_bootstrap = AcpSessionBootstrap {
            session_key: "session-bound-b".to_owned(),
            conversation_id: Some("telegram:1001".to_owned()),
            binding: None,
            ..first_bootstrap.clone()
        };

        let first = manager
            .ensure_session(&config, &first_bootstrap)
            .await
            .expect("first bound ensure");
        let second = manager
            .ensure_session(&config, &second_bootstrap)
            .await
            .expect("second bound ensure");

        assert_eq!(first.session_key, "session-bound-a");
        assert_eq!(second.session_key, "session-bound-a");
        assert_eq!(second.conversation_id.as_deref(), Some("telegram:1001"));
        assert_eq!(manager.list_sessions().expect("list sessions").len(), 1);
        assert_eq!(counts.lock().expect("counting state").ensure_calls, 1);
    }

    #[tokio::test]
    async fn ensure_session_reuses_bound_route_scope_when_bindings_enabled() {
        let counts = install_manager_backends(
            "manager-counting-route-binding-reuse",
            "manager-alt-route-binding-reuse",
        );
        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-route-binding-reuse".to_owned()),
                bindings_enabled: true,
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let binding_metadata = BTreeMap::from([
            (
                "route_session_id".to_owned(),
                "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
            ),
            ("channel".to_owned(), "feishu".to_owned()),
            ("channel_account_id".to_owned(), "lark-prod".to_owned()),
            ("channel_conversation_id".to_owned(), "oc_123".to_owned()),
            ("channel_thread_id".to_owned(), "om_thread_1".to_owned()),
        ]);
        let binding = Some(super::super::AcpSessionBindingScope {
            route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
            channel_id: Some("feishu".to_owned()),
            account_id: Some("lark-prod".to_owned()),
            conversation_id: Some("oc_123".to_owned()),
            participant_id: None,
            thread_id: Some("om_thread_1".to_owned()),
        });
        let first_bootstrap = AcpSessionBootstrap {
            session_key: "session-route-bound-a".to_owned(),
            conversation_id: Some("opaque-session-a".to_owned()),
            binding: binding.clone(),
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: binding_metadata.clone(),
        };
        let second_bootstrap = AcpSessionBootstrap {
            session_key: "session-route-bound-b".to_owned(),
            conversation_id: Some("opaque-session-b".to_owned()),
            binding,
            metadata: binding_metadata,
            ..first_bootstrap.clone()
        };

        let first = manager
            .ensure_session(&config, &first_bootstrap)
            .await
            .expect("first bound ensure");
        let second = manager
            .ensure_session(&config, &second_bootstrap)
            .await
            .expect("second bound ensure");

        assert_eq!(first.session_key, "session-route-bound-a");
        assert_eq!(second.session_key, "session-route-bound-a");
        assert_eq!(
            second.conversation_id.as_deref(),
            Some("opaque-session-a"),
            "legacy conversation id should remain stable once a structured binding exists"
        );
        assert_eq!(
            second
                .binding
                .as_ref()
                .map(|binding| binding.route_session_id.as_str()),
            Some("feishu:lark-prod:oc_123:om_thread_1")
        );
        assert_eq!(manager.list_sessions().expect("list sessions").len(), 1);
        assert_eq!(counts.lock().expect("counting state").ensure_calls, 1);
    }

    #[tokio::test]
    async fn run_turn_updates_persisted_session_state() {
        let counts = install_manager_backends("manager-counting-turn", "manager-alt-turn");
        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-turn".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-turn".to_owned(),
            conversation_id: Some("conv-2".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: Some("hello".to_owned()),
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: vec!["fs".to_owned()],
            metadata: BTreeMap::new(),
        };
        let request = AcpTurnRequest {
            session_key: "session-turn".to_owned(),
            input: "ping".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let result = manager
            .run_turn(&config, &bootstrap, &request)
            .await
            .expect("run turn");
        let sessions = manager.list_sessions().expect("list sessions");
        let session = sessions
            .iter()
            .find(|entry| entry.session_key == "session-turn")
            .expect("stored session metadata");

        assert_eq!(result.output_text, "echo: ping");
        assert_eq!(session.state, AcpSessionState::Ready);
        assert_eq!(session.mode, Some(AcpSessionMode::Interactive));
        assert!(session.last_error.is_none());
        assert_eq!(counts.lock().expect("counting state").turn_calls, 1);
    }

    #[tokio::test]
    async fn run_turn_persists_last_error_for_failed_backend() {
        register_acp_backend("manager-failing-turn", || {
            Box::new(FailingBackend {
                id: "manager-failing-turn",
            })
        })
        .expect("register failing backend");

        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-failing-turn".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-error".to_owned(),
            conversation_id: Some("conv-error".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let request = AcpTurnRequest {
            session_key: "session-error".to_owned(),
            input: "boom".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let error = manager
            .run_turn(&config, &bootstrap, &request)
            .await
            .expect_err("failing backend should bubble error");
        let session = manager
            .list_sessions()
            .expect("list sessions")
            .into_iter()
            .find(|entry| entry.session_key == "session-error")
            .expect("persisted error session");

        assert!(
            error.contains("synthetic ACP turn failure"),
            "error: {error}"
        );
        assert_eq!(session.state, AcpSessionState::Error);
        assert_eq!(
            session.last_error.as_deref(),
            Some("synthetic ACP turn failure")
        );
    }

    #[tokio::test]
    async fn observability_snapshot_tracks_success_failure_and_error_counts() {
        let counts = install_manager_backends("manager-counting-observe", "manager-alt-observe");
        register_acp_backend("manager-failing-observe", || {
            Box::new(FailingBackend {
                id: "manager-failing-observe",
            })
        })
        .expect("register failing observe backend");

        let manager = AcpSessionManager::default();
        let success_config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-observe".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let failure_config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-failing-observe".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        manager
            .run_turn(
                &success_config,
                &AcpSessionBootstrap {
                    session_key: "session-observe-ok".to_owned(),
                    conversation_id: Some("conv-observe-ok".to_owned()),
                    binding: None,
                    working_directory: None,
                    initial_prompt: None,
                    mode: Some(AcpSessionMode::Interactive),
                    mcp_servers: Vec::new(),
                    metadata: BTreeMap::new(),
                },
                &AcpTurnRequest {
                    session_key: "session-observe-ok".to_owned(),
                    input: "ok".to_owned(),
                    working_directory: None,
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect("successful observed turn");

        let error = manager
            .run_turn(
                &failure_config,
                &AcpSessionBootstrap {
                    session_key: "session-observe-fail".to_owned(),
                    conversation_id: Some("conv-observe-fail".to_owned()),
                    binding: None,
                    working_directory: None,
                    initial_prompt: None,
                    mode: Some(AcpSessionMode::Interactive),
                    mcp_servers: Vec::new(),
                    metadata: BTreeMap::new(),
                },
                &AcpTurnRequest {
                    session_key: "session-observe-fail".to_owned(),
                    input: "fail".to_owned(),
                    working_directory: None,
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect_err("failing observed turn should bubble error");
        let snapshot = manager
            .observability_snapshot(&success_config)
            .await
            .expect("observability snapshot");

        assert!(
            error.contains("synthetic ACP turn failure"),
            "error: {error}"
        );
        assert_eq!(snapshot.runtime_cache.active_sessions, 2);
        assert_eq!(
            snapshot.runtime_cache.idle_ttl_ms,
            success_config.acp.session_idle_ttl_ms()
        );
        assert_eq!(snapshot.actors.active, 0);
        assert_eq!(snapshot.actors.queue_depth, 0);
        assert_eq!(snapshot.actors.waiting, 0);
        assert_eq!(snapshot.turns.active, 0);
        assert_eq!(snapshot.turns.queue_depth, 0);
        assert_eq!(snapshot.turns.completed, 1);
        assert_eq!(snapshot.turns.failed, 1);
        assert!(snapshot.turns.max_latency_ms >= snapshot.turns.average_latency_ms);
        assert_eq!(
            snapshot
                .errors_by_code
                .get("synthetic ACP turn failure")
                .copied(),
            Some(1)
        );
        assert_eq!(counts.lock().expect("counting state").turn_calls, 1);
    }

    #[tokio::test]
    async fn run_turn_serializes_concurrent_turns_for_same_session() {
        let shared = Arc::new(QueuedTurnState::default());
        register_acp_backend("manager-queued-turn", {
            let shared = shared.clone();
            move || {
                Box::new(QueuedTurnBackend {
                    id: "manager-queued-turn",
                    state: shared.clone(),
                })
            }
        })
        .expect("register queued backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-queued-turn".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-queued".to_owned(),
            conversation_id: Some("conv-queued".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let first_request = AcpTurnRequest {
            session_key: "session-queued".to_owned(),
            input: "first".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };
        let second_request = AcpTurnRequest {
            session_key: "session-queued".to_owned(),
            input: "second".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let first_task = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(config.as_ref(), &bootstrap, &first_request)
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.first_turn_entered.notified())
            .await
            .expect("first turn should reach backend");

        let second_task = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(config.as_ref(), &bootstrap, &second_request)
                    .await
            })
        };

        let status = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let status = manager
                    .get_status(config.as_ref(), "session-queued")
                    .await
                    .expect("status should succeed while first turn is active");
                if status.pending_turns >= 2 {
                    return status;
                }
                assert!(
                    !second_task.is_finished(),
                    "second turn should remain queued until the first turn completes"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued turn status should become observable");
        let snapshot = manager
            .observability_snapshot(config.as_ref())
            .await
            .expect("observability snapshot while turn queue is active");

        assert_eq!(status.state, AcpSessionState::Busy);
        assert_eq!(status.pending_turns, 2);
        assert_eq!(snapshot.turns.active, 1);
        assert_eq!(snapshot.turns.queue_depth, 2);
        assert_eq!(
            shared.inputs.lock().expect("queued inputs").as_slice(),
            ["first"],
            "second turn should not enter backend execution before the first finishes"
        );

        shared.release_first_turn.notify_waiters();

        let first = first_task
            .await
            .expect("first join should succeed")
            .expect("first turn should succeed");
        let second = second_task
            .await
            .expect("second join should succeed")
            .expect("second turn should succeed");
        let inputs = shared.inputs.lock().expect("queued inputs").clone();

        assert_eq!(first.output_text, "queued: first");
        assert_eq!(second.output_text, "queued: second");
        assert_eq!(inputs, vec!["first".to_owned(), "second".to_owned()]);
        assert_eq!(shared.max_active_turns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancel_preempts_active_turn_and_returns_session_to_ready() {
        let shared = Arc::new(AbortableTurnState::default());
        register_acp_backend("manager-abortable-turn", {
            let shared = shared.clone();
            move || {
                Box::new(AbortableTurnBackend {
                    id: "manager-abortable-turn",
                    state: shared.clone(),
                })
            }
        })
        .expect("register abortable backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-abortable-turn".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-abortable".to_owned(),
            conversation_id: Some("conv-abortable".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let request = AcpTurnRequest {
            session_key: "session-abortable".to_owned(),
            input: "long-running".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let run_task = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(config.as_ref(), &bootstrap, &request)
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.turn_entered.notified())
            .await
            .expect("abortable turn should enter backend");

        manager
            .cancel(config.as_ref(), bootstrap.session_key.as_str())
            .await
            .expect("cancel should succeed while turn is active");

        let result = tokio::time::timeout(Duration::from_secs(1), async {
            run_task
                .await
                .expect("abortable turn join should succeed")
                .expect("abortable turn should resolve as cancelled success")
        })
        .await
        .expect("cancelled turn should finish promptly");

        let session = manager
            .list_sessions()
            .expect("list sessions")
            .into_iter()
            .find(|entry| entry.session_key == "session-abortable")
            .expect("persisted abortable session");

        assert_eq!(result.state, AcpSessionState::Ready);
        assert_eq!(result.stop_reason, Some(AcpTurnStopReason::Cancelled));
        assert_eq!(
            shared.cancel_calls.load(Ordering::SeqCst),
            1,
            "manager should issue backend cancel immediately for active turn"
        );
        assert_eq!(
            shared.abort_observed.load(Ordering::SeqCst),
            1,
            "manager should trip active-turn abort signal"
        );
        assert_eq!(session.state, AcpSessionState::Ready);
        assert!(session.last_error.is_none());
    }

    #[tokio::test]
    async fn cancel_reports_cancelling_state_while_active_turn_is_draining() {
        let shared = Arc::new(AbortableTurnState::default());
        shared.hold_after_abort.store(true, Ordering::SeqCst);
        register_acp_backend("manager-abortable-turn-status", {
            let shared = shared.clone();
            move || {
                Box::new(AbortableTurnBackend {
                    id: "manager-abortable-turn-status",
                    state: shared.clone(),
                })
            }
        })
        .expect("register abortable status backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-abortable-turn-status".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-abortable-status".to_owned(),
            conversation_id: Some("conv-abortable-status".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let request = AcpTurnRequest {
            session_key: "session-abortable-status".to_owned(),
            input: "long-running".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let run_task = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(config.as_ref(), &bootstrap, &request)
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.turn_entered.notified())
            .await
            .expect("abortable status turn should enter backend");

        manager
            .cancel(config.as_ref(), bootstrap.session_key.as_str())
            .await
            .expect("cancel should start while turn is active");

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if shared.abort_observed.load(Ordering::SeqCst) > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancel should trigger active turn abort");

        let status = manager
            .get_status(config.as_ref(), bootstrap.session_key.as_str())
            .await
            .expect("status should project cancelling repair state");

        assert_eq!(status.state, AcpSessionState::Cancelling);
        assert_eq!(status.pending_turns, 1);
        assert_eq!(
            status.active_turn_id.as_deref(),
            Some("abortable-session-abortable-status")
        );

        shared.release_abort_completion.notify_waiters();

        let result = tokio::time::timeout(Duration::from_secs(1), async {
            run_task
                .await
                .expect("abortable status join should succeed")
                .expect("abortable status turn should resolve as cancelled success")
        })
        .await
        .expect("cancelled status turn should finish promptly");
        let session = manager
            .list_sessions()
            .expect("list sessions")
            .into_iter()
            .find(|entry| entry.session_key == "session-abortable-status")
            .expect("persisted abortable status session");

        assert_eq!(result.state, AcpSessionState::Ready);
        assert_eq!(result.stop_reason, Some(AcpTurnStopReason::Cancelled));
        assert_eq!(session.state, AcpSessionState::Ready);
        assert!(session.last_error.is_none());
    }

    #[tokio::test]
    async fn close_preempts_active_turn_reports_cancelling_and_removes_session() {
        let shared = Arc::new(AbortableTurnState::default());
        shared.hold_after_abort.store(true, Ordering::SeqCst);
        register_acp_backend("manager-abortable-turn-close", {
            let shared = shared.clone();
            move || {
                Box::new(AbortableTurnBackend {
                    id: "manager-abortable-turn-close",
                    state: shared.clone(),
                })
            }
        })
        .expect("register abortable close backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-abortable-turn-close".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-abortable-close".to_owned(),
            conversation_id: Some("conv-abortable-close".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let request = AcpTurnRequest {
            session_key: "session-abortable-close".to_owned(),
            input: "long-running".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };

        let run_task = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(config.as_ref(), &bootstrap, &request)
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.turn_entered.notified())
            .await
            .expect("abortable close turn should enter backend");

        let close_task = {
            let manager = manager.clone();
            let config = config.clone();
            let session_key = bootstrap.session_key.clone();
            tokio::spawn(async move { manager.close(config.as_ref(), session_key.as_str()).await })
        };

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if shared.abort_observed.load(Ordering::SeqCst) > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("close should trigger active turn repair immediately");

        let status = manager
            .get_status(config.as_ref(), bootstrap.session_key.as_str())
            .await
            .expect("status should expose close repair state");

        assert_eq!(status.state, AcpSessionState::Cancelling);
        assert_eq!(status.pending_turns, 1);
        assert_eq!(
            status.active_turn_id.as_deref(),
            Some("abortable-session-abortable-close")
        );

        shared.release_abort_completion.notify_waiters();

        tokio::time::timeout(Duration::from_secs(1), async {
            close_task
                .await
                .expect("close join should succeed")
                .expect("close should finish after active turn repair")
        })
        .await
        .expect("close repair should complete promptly");
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            run_task
                .await
                .expect("abortable close join should succeed")
                .expect("abortable close turn should resolve as cancelled success")
        })
        .await
        .expect("closed turn should finish promptly");

        assert_eq!(result.stop_reason, Some(AcpTurnStopReason::Cancelled));
        assert_eq!(shared.cancel_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shared.close_calls.load(Ordering::SeqCst), 1);
        assert!(
            manager
                .list_sessions()
                .expect("list sessions")
                .into_iter()
                .all(|entry| entry.session_key != "session-abortable-close"),
            "close repair should remove the persisted session"
        );
    }

    #[tokio::test]
    async fn run_turn_with_sink_forwards_streamed_events_and_backfills_result_events() {
        let backend_id: &'static str =
            Box::leak(format!("manager-streaming-sink-{}", super::now_ms()).into_boxed_str());
        let state = Arc::new(StreamingSinkState::default());
        register_acp_backend(backend_id, {
            let state = state.clone();
            move || {
                Box::new(StreamingSinkBackend {
                    id: backend_id,
                    state: state.clone(),
                })
            }
        })
        .expect("register streaming sink backend");

        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some(backend_id.to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let bootstrap = AcpSessionBootstrap {
            session_key: "stream-session".to_owned(),
            conversation_id: Some("stream-conversation".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let request = AcpTurnRequest {
            session_key: "stream-session".to_owned(),
            input: "hello".to_owned(),
            working_directory: None,
            metadata: BTreeMap::new(),
        };
        let sink = RecordingTurnEventSink::default();

        let result = manager
            .run_turn_with_sink(&config, &bootstrap, &request, Some(&sink))
            .await
            .expect("streaming sink run should succeed");

        assert_eq!(result.output_text, "streamed: hello");
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0]["type"], "text");
        assert_eq!(result.events[1]["type"], "done");
        let captured = sink.events.lock().expect("captured sink events").clone();
        assert_eq!(captured, result.events);
        assert_eq!(state.sink_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn get_status_uses_fallback_while_running_turn_without_entering_backend() {
        let shared = Arc::new(SerializedControlState::default());
        register_acp_backend("manager-serialized-status", {
            let shared = shared.clone();
            move || {
                Box::new(SerializedControlBackend {
                    id: "manager-serialized-status",
                    state: shared.clone(),
                })
            }
        })
        .expect("register serialized status backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-serialized-status".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-status-serialized".to_owned(),
            conversation_id: Some("conv-status-serialized".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let first_turn = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(
                        config.as_ref(),
                        &bootstrap,
                        &AcpTurnRequest {
                            session_key: bootstrap.session_key.clone(),
                            input: "first".to_owned(),
                            working_directory: None,
                            metadata: BTreeMap::new(),
                        },
                    )
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.first_turn_entered.notified())
            .await
            .expect("first turn should enter backend");

        let status = manager
            .get_status(config.as_ref(), bootstrap.session_key.as_str())
            .await
            .expect("status should fall back while running turn is active");

        shared.release_first_turn.notify_waiters();

        first_turn
            .await
            .expect("first turn join should succeed")
            .expect("first turn should succeed");

        assert_eq!(status.state, AcpSessionState::Busy);
        assert_eq!(status.mode, Some(AcpSessionMode::Interactive));
        assert_eq!(status.pending_turns, 1);
        assert_eq!(
            status.active_turn_id.as_deref(),
            Some("serialized-session-status-serialized")
        );
        assert_eq!(
            shared.status_calls.load(Ordering::SeqCst),
            0,
            "busy-path status should not call backend.get_status concurrently with the running turn"
        );
        assert_eq!(
            shared
                .events
                .lock()
                .expect("serialized control events")
                .as_slice(),
            ["turn:first"],
        );
    }

    #[tokio::test]
    async fn set_mode_serializes_behind_running_turn_for_same_session() {
        let shared = Arc::new(SerializedControlState::default());
        register_acp_backend("manager-serialized-control", {
            let shared = shared.clone();
            move || {
                Box::new(SerializedControlBackend {
                    id: "manager-serialized-control",
                    state: shared.clone(),
                })
            }
        })
        .expect("register serialized control backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-serialized-control".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-control-serialized".to_owned(),
            conversation_id: Some("conv-control-serialized".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let first_turn = {
            let manager = manager.clone();
            let config = config.clone();
            let bootstrap = bootstrap.clone();
            tokio::spawn(async move {
                manager
                    .run_turn(
                        config.as_ref(),
                        &bootstrap,
                        &AcpTurnRequest {
                            session_key: bootstrap.session_key.clone(),
                            input: "first".to_owned(),
                            working_directory: None,
                            metadata: BTreeMap::new(),
                        },
                    )
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.first_turn_entered.notified())
            .await
            .expect("first turn should enter backend");

        let set_mode = {
            let manager = manager.clone();
            let config = config.clone();
            let session_key = bootstrap.session_key.clone();
            tokio::spawn(async move {
                manager
                    .set_mode(
                        config.as_ref(),
                        session_key.as_str(),
                        AcpSessionMode::Review,
                    )
                    .await
            })
        };

        let premature_control = tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                if shared.set_mode_calls.load(Ordering::SeqCst) > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            premature_control.is_err(),
            "set_mode entered backend before running turn completed"
        );
        assert!(
            !set_mode.is_finished(),
            "set_mode should remain queued while first turn is active"
        );

        let snapshot = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = manager
                    .observability_snapshot(config.as_ref())
                    .await
                    .expect("observability snapshot during queued set_mode");
                if snapshot.actors.queue_depth >= 2 {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued control should become visible to observability");
        assert_eq!(snapshot.actors.active, 1);
        assert_eq!(snapshot.actors.queue_depth, 2);
        assert_eq!(snapshot.actors.waiting, 1);
        assert_eq!(snapshot.turns.active, 1);
        assert_eq!(snapshot.turns.queue_depth, 1);

        shared.release_first_turn.notify_waiters();

        first_turn
            .await
            .expect("first turn join should succeed")
            .expect("first turn should succeed");
        set_mode
            .await
            .expect("set_mode join should succeed")
            .expect("set_mode should succeed");

        assert_eq!(shared.set_mode_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            shared
                .events
                .lock()
                .expect("serialized control events")
                .as_slice(),
            ["turn:first", "set-mode:review"],
        );
    }

    #[tokio::test]
    async fn cleanup_idle_sessions_skips_session_with_inflight_control_operation() {
        let shared = Arc::new(SlowControlState::default());
        register_acp_backend("manager-slow-control", {
            let shared = shared.clone();
            move || {
                Box::new(SlowControlBackend {
                    id: "manager-slow-control",
                    state: shared.clone(),
                })
            }
        })
        .expect("register slow control backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-slow-control".to_owned()),
                session_idle_ttl_ms: Some(1),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-slow-control".to_owned(),
            conversation_id: Some("conv-slow-control".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        manager
            .ensure_session(config.as_ref(), &bootstrap)
            .await
            .expect("ensure slow control session");
        tokio::time::sleep(Duration::from_millis(5)).await;

        let set_mode = {
            let manager = manager.clone();
            let config = config.clone();
            let session_key = bootstrap.session_key.clone();
            tokio::spawn(async move {
                manager
                    .set_mode(
                        config.as_ref(),
                        session_key.as_str(),
                        AcpSessionMode::Review,
                    )
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), shared.set_mode_entered.notified())
            .await
            .expect("set_mode should enter backend");

        let snapshot = manager
            .observability_snapshot(config.as_ref())
            .await
            .expect("observability snapshot during inflight control");
        assert_eq!(snapshot.runtime_cache.active_sessions, 1);
        assert_eq!(snapshot.actors.active, 1);
        assert_eq!(snapshot.actors.queue_depth, 1);
        assert_eq!(snapshot.actors.waiting, 0);
        assert_eq!(snapshot.turns.active, 0);
        assert_eq!(snapshot.turns.queue_depth, 0);
        assert_eq!(
            manager.list_sessions().expect("list sessions").len(),
            1,
            "idle cleanup must not evict a session while set_mode is in flight"
        );

        shared.release_set_mode.notify_waiters();
        set_mode
            .await
            .expect("set_mode join should succeed")
            .expect("set_mode should succeed");
    }

    #[tokio::test]
    async fn cleanup_idle_sessions_keeps_session_when_backend_close_fails() {
        let shared = Arc::new(CloseFailureState::default());
        register_acp_backend("manager-close-failure", {
            let shared = shared.clone();
            move || {
                Box::new(CloseFailureBackend {
                    id: "manager-close-failure",
                    state: shared.clone(),
                })
            }
        })
        .expect("register close-failure backend");

        let manager = Arc::new(AcpSessionManager::default());
        let config = Arc::new(LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-close-failure".to_owned()),
                session_idle_ttl_ms: Some(1),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        });
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-close-failure".to_owned(),
            conversation_id: Some("conv-close-failure".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        manager
            .ensure_session(config.as_ref(), &bootstrap)
            .await
            .expect("ensure close-failure session");
        tokio::time::sleep(Duration::from_millis(5)).await;

        manager
            .cleanup_idle_sessions(config.as_ref())
            .await
            .expect("idle cleanup should keep failed-close session");

        let sessions = manager.list_sessions().expect("list sessions");
        let snapshot = manager
            .observability_snapshot(config.as_ref())
            .await
            .expect("observability snapshot after close failure");

        assert_eq!(shared.close_calls.load(Ordering::SeqCst), 2);
        assert!(
            sessions
                .iter()
                .any(|entry| entry.session_key == "session-close-failure"),
            "failed idle close must keep the session metadata"
        );
        assert_eq!(snapshot.runtime_cache.active_sessions, 1);
        assert_eq!(snapshot.runtime_cache.evicted_total, 0);
    }

    #[tokio::test]
    async fn cancelled_waiting_turn_queue_guard_rolls_back_counts() {
        let manager = Arc::new(AcpSessionManager::default());
        let actor_key = "queued-turn-roll-back";

        let held_guard = manager
            .acquire_turn_queue_guard(actor_key.to_owned())
            .await
            .expect("initial turn queue guard");

        let waiting_guard = {
            let manager = manager.clone();
            tokio::spawn(
                async move { manager.acquire_turn_queue_guard(actor_key.to_owned()).await },
            )
        };

        wait_for_actor_counts(manager.as_ref(), actor_key, 2, 2).await;

        waiting_guard.abort();
        let join_result = waiting_guard.await;
        assert!(join_result.is_err(), "aborted waiter should not complete");

        wait_for_actor_counts(manager.as_ref(), actor_key, 1, 1).await;

        drop(held_guard);

        wait_for_actor_counts(manager.as_ref(), actor_key, 0, 0).await;
    }

    #[tokio::test]
    async fn cancelled_waiting_session_actor_guard_rolls_back_ref_count() {
        let manager = Arc::new(AcpSessionManager::default());
        let actor_key = "session-actor-roll-back";

        let held_guard = manager
            .acquire_session_actor_guard(actor_key.to_owned())
            .await
            .expect("initial session actor guard");

        let waiting_guard = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .acquire_session_actor_guard(actor_key.to_owned())
                    .await
            })
        };

        wait_for_actor_counts(manager.as_ref(), actor_key, 2, 0).await;

        waiting_guard.abort();
        let join_result = waiting_guard.await;
        assert!(join_result.is_err(), "aborted waiter should not complete");

        wait_for_actor_counts(manager.as_ref(), actor_key, 1, 0).await;

        drop(held_guard);

        wait_for_actor_counts(manager.as_ref(), actor_key, 0, 0).await;
    }

    #[tokio::test]
    async fn ensure_session_rejects_backend_mismatch_for_existing_session() {
        install_manager_backends("manager-counting-mismatch", "manager-alt-mismatch");
        let manager = AcpSessionManager::default();
        let initial = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-mismatch".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let switched = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-alt-mismatch".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-boundary".to_owned(),
            conversation_id: Some("conv-3".to_owned()),
            binding: None,
            working_directory: None,
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        manager
            .ensure_session(&initial, &bootstrap)
            .await
            .expect("initial ensure");
        let error = manager
            .ensure_session(&switched, &bootstrap)
            .await
            .expect_err("backend mismatch should fail");

        assert!(error.contains("bound to ACP backend"), "error: {error}");
    }

    #[tokio::test]
    async fn ensure_session_honors_max_concurrent_sessions() {
        install_manager_backends("manager-counting-cap", "manager-alt-cap");
        let manager = AcpSessionManager::default();
        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("manager-counting-cap".to_owned()),
                max_concurrent_sessions: Some(1),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        manager
            .ensure_session(
                &config,
                &AcpSessionBootstrap {
                    session_key: "session-cap-1".to_owned(),
                    conversation_id: Some("conv-cap-1".to_owned()),
                    binding: None,
                    working_directory: None,
                    initial_prompt: None,
                    mode: Some(AcpSessionMode::Interactive),
                    mcp_servers: Vec::new(),
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect("first session under cap");

        let error = manager
            .ensure_session(
                &config,
                &AcpSessionBootstrap {
                    session_key: "session-cap-2".to_owned(),
                    conversation_id: Some("conv-cap-2".to_owned()),
                    binding: None,
                    working_directory: None,
                    initial_prompt: None,
                    mode: Some(AcpSessionMode::Interactive),
                    mcp_servers: Vec::new(),
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect_err("second session should exceed cap");

        assert!(
            error.contains("max_concurrent_sessions=1"),
            "error should mention configured cap: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn sqlite_store_roundtrips_session_metadata() {
        let sqlite_path = std::env::temp_dir().join(format!(
            "loongclaw-acp-store-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sqlite_path);

        let store = AcpSqliteSessionStore::new(Some(sqlite_path.clone()));
        let metadata = super::super::AcpSessionMetadata {
            session_key: "sqlite-session".to_owned(),
            conversation_id: Some("telegram:42".to_owned()),
            binding: Some(super::super::AcpSessionBindingScope {
                route_session_id: "telegram:bot_123456:42".to_owned(),
                channel_id: Some("telegram".to_owned()),
                account_id: Some("bot_123456".to_owned()),
                conversation_id: Some("42".to_owned()),
                participant_id: None,
                thread_id: None,
            }),
            activation_origin: Some(super::super::AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "manager-counting".to_owned(),
            runtime_session_name: "sqlite-runtime".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-1".to_owned()),
            agent_session_id: Some("agent-1".to_owned()),
            mode: Some(AcpSessionMode::Review),
            state: AcpSessionState::Ready,
            last_activity_ms: 42,
            last_error: Some("warning".to_owned()),
        };

        store
            .upsert(metadata.clone())
            .expect("upsert sqlite metadata");
        let loaded = store
            .get("sqlite-session")
            .expect("load sqlite metadata")
            .expect("sqlite metadata should exist");
        let by_conversation = store
            .get_by_conversation_id("telegram:42")
            .expect("load sqlite metadata by conversation")
            .expect("sqlite conversation binding should exist");
        let by_binding = store
            .get_by_binding_route_session_id("telegram:bot_123456:42")
            .expect("load sqlite metadata by structured route")
            .expect("sqlite structured binding should exist");

        assert_eq!(loaded, metadata);
        assert_eq!(by_conversation, metadata);
        assert_eq!(by_binding, metadata);
        let _ = std::fs::remove_file(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn sqlite_store_migrates_legacy_schema_before_creating_conversation_index() {
        let sqlite_path = std::env::temp_dir().join(format!(
            "loongclaw-acp-legacy-store-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sqlite_path);

        let conn = rusqlite::Connection::open(&sqlite_path).expect("open legacy sqlite");
        conn.execute_batch(
            "
            CREATE TABLE acp_sessions(
              session_key TEXT PRIMARY KEY,
              backend_id TEXT NOT NULL,
              runtime_session_name TEXT NOT NULL,
              working_directory TEXT,
              backend_session_id TEXT,
              agent_session_id TEXT,
              mode TEXT,
              state TEXT NOT NULL
            );
            ",
        )
        .expect("create legacy ACP schema");
        drop(conn);

        let store = AcpSqliteSessionStore::new(Some(sqlite_path.clone()));
        let metadata = super::super::AcpSessionMetadata {
            session_key: "legacy-session".to_owned(),
            conversation_id: Some("feishu:chat-1".to_owned()),
            binding: Some(super::super::AcpSessionBindingScope {
                route_session_id: "feishu:workspace_a:chat-1".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("workspace_a".to_owned()),
                conversation_id: Some("chat-1".to_owned()),
                participant_id: None,
                thread_id: None,
            }),
            activation_origin: Some(super::super::AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "manager-counting".to_owned(),
            runtime_session_name: "legacy-runtime".to_owned(),
            working_directory: None,
            backend_session_id: None,
            agent_session_id: None,
            mode: Some(AcpSessionMode::Interactive),
            state: AcpSessionState::Ready,
            last_activity_ms: 7,
            last_error: None,
        };

        store
            .upsert(metadata.clone())
            .expect("upsert migrated metadata");
        let loaded = store
            .get_by_conversation_id("feishu:chat-1")
            .expect("lookup migrated metadata")
            .expect("migrated metadata should exist");
        let by_binding = store
            .get_by_binding_route_session_id("feishu:workspace_a:chat-1")
            .expect("lookup migrated structured binding")
            .expect("migrated structured binding should exist");
        assert_eq!(loaded, metadata);
        assert_eq!(by_binding, metadata);

        let _ = std::fs::remove_file(&sqlite_path);
    }
}
