use super::*;

impl AcpSessionManager {
    pub(super) fn resolve_existing_session(
        &self,
        config: &LoongClawConfig,
        selected_backend_id: &str,
        bootstrap: &AcpSessionBootstrap,
    ) -> CliResult<Option<AcpSessionMetadata>> {
        if let Some(existing) = self.store.get(bootstrap.session_key.as_str())? {
            return self
                .validate_and_touch_existing_session(selected_backend_id, bootstrap, existing)
                .map(Some);
        }

        if config.acp.bindings_enabled {
            if let Some(binding) = AcpSessionBindingScope::from_bootstrap(bootstrap)
                && let Some(existing) = self
                    .store
                    .get_by_binding_route_session_id(binding.route_session_id.as_str())?
            {
                return self
                    .validate_and_touch_existing_session(selected_backend_id, bootstrap, existing)
                    .map(Some);
            }
            if let Some(conversation_id) =
                normalized_conversation_id(bootstrap.conversation_id.as_deref())
                && let Some(existing) = self
                    .store
                    .get_by_conversation_id(conversation_id.as_str())?
            {
                return self
                    .validate_and_touch_existing_session(selected_backend_id, bootstrap, existing)
                    .map(Some);
            }
        }

        Ok(None)
    }

    pub(super) fn validate_and_touch_existing_session(
        &self,
        selected_backend_id: &str,
        bootstrap: &AcpSessionBootstrap,
        mut existing: AcpSessionMetadata,
    ) -> CliResult<AcpSessionMetadata> {
        if existing.backend_id != selected_backend_id {
            return Err(format!(
                "session `{}` is already bound to ACP backend `{}` (requested `{}`); use a new session key or close the existing session first",
                existing.session_key, existing.backend_id, selected_backend_id
            ));
        }

        if let Some(binding) = AcpSessionBindingScope::from_bootstrap(bootstrap) {
            if let Some(existing_binding) = existing.binding.as_ref() {
                if existing_binding != &binding {
                    return Err(format!(
                        "session `{}` is already bound to ACP route `{}` (requested `{}`); use a new session key or close the existing session first",
                        existing.session_key,
                        existing_binding.route_session_id,
                        binding.route_session_id
                    ));
                }
            } else {
                existing.binding = Some(binding);
            }
        }

        if let Some(conversation_id) =
            normalized_conversation_id(bootstrap.conversation_id.as_deref())
        {
            if existing.binding.is_some() {
                if existing.conversation_id.is_none() {
                    existing.conversation_id = Some(conversation_id);
                }
            } else {
                existing.conversation_id = Some(conversation_id);
            }
        }
        if existing.mode.is_none() {
            existing.mode = bootstrap.mode;
        }
        existing.touch();
        self.store.upsert(existing.clone())?;
        Ok(existing)
    }

    pub(super) fn enforce_max_concurrent_sessions(
        &self,
        config: &LoongClawConfig,
        requested_session_key: &str,
    ) -> CliResult<()> {
        if self.store.get(requested_session_key)?.is_some() {
            return Ok(());
        }
        let current = self.store.list()?.len();
        let limit = config.acp.max_concurrent_sessions();
        if current >= limit {
            return Err(format!(
                "ACP control plane already tracks {current} sessions, which reaches max_concurrent_sessions={limit}"
            ));
        }
        Ok(())
    }

    pub(super) async fn cleanup_idle_sessions(&self, config: &LoongClawConfig) -> CliResult<()> {
        let ttl_ms = config.acp.session_idle_ttl_ms();
        if ttl_ms == 0 {
            return Ok(());
        }

        let now = now_ms();
        for metadata in self.store.list()? {
            if matches!(
                metadata.state,
                AcpSessionState::Busy | AcpSessionState::Cancelling | AcpSessionState::Initializing
            ) {
                continue;
            }
            if self.actor_ref_count_for_metadata(&metadata)? > 0 {
                continue;
            }
            if self.pending_turn_count_for_metadata(&metadata)? > 0 {
                continue;
            }
            if self.is_active_turn_for_metadata(&metadata)? {
                continue;
            }
            if now.saturating_sub(metadata.last_activity_ms) < ttl_ms {
                continue;
            }

            let close_result = match resolve_acp_backend(Some(metadata.backend_id.as_str())) {
                Ok(backend) => backend.close(config, &metadata.to_handle()).await,
                Err(error) => Err(error),
            };
            if let Err(error) = close_result {
                self.record_error(error.as_str())?;
                tracing::warn!(
                    target: "loongclaw.acp",
                    session_key = %metadata.session_key,
                    backend_id = %metadata.backend_id,
                    error = %error,
                    "failed to close idle ACP session; keeping metadata for reuse or follow-up cancellation"
                );
                continue;
            }
            let _ = self.store.remove(metadata.session_key.as_str());
            self.record_eviction(now)?;
        }

        Ok(())
    }

    pub(super) fn fallback_status(
        &self,
        metadata: &AcpSessionMetadata,
        active_turn: bool,
        pending_turns: usize,
    ) -> AcpSessionStatus {
        AcpSessionStatus {
            session_key: metadata.session_key.clone(),
            backend_id: metadata.backend_id.clone(),
            conversation_id: metadata.conversation_id.clone(),
            binding: metadata.binding.clone(),
            activation_origin: metadata.activation_origin,
            state: projected_status_state(metadata.state, active_turn, pending_turns),
            mode: metadata.mode,
            pending_turns: pending_turns.max(usize::from(active_turn)),
            active_turn_id: active_turn.then(|| metadata.runtime_session_name.clone()),
            last_activity_ms: metadata.last_activity_ms,
            last_error: metadata.last_error.clone(),
        }
    }

    pub(super) async fn request_active_turn_cancellation(
        &self,
        config: &LoongClawConfig,
        mut metadata: AcpSessionMetadata,
        active_turn: Arc<ActiveTurnState>,
    ) -> CliResult<()> {
        active_turn.abort_controller.abort();

        metadata.state = AcpSessionState::Cancelling;
        metadata.clear_error();
        metadata.touch();
        self.store.upsert(metadata.clone())?;

        let backend = resolve_acp_backend(Some(active_turn.handle.backend_id.as_str()))?;
        match backend.cancel(config, &active_turn.handle).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.record_error(error.as_str())?;
                metadata.state = AcpSessionState::Error;
                metadata.set_error(error.clone());
                self.store.upsert(metadata)?;
                Err(error)
            }
        }
    }
}
