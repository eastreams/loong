use super::*;

impl AcpSessionManager {
    pub(super) async fn acquire_session_actor_guard(
        &self,
        actor_key: String,
    ) -> CliResult<SessionActorGuard> {
        self.increment_actor_ref_count(actor_key.as_str())?;
        let queue_lock = match self.get_or_insert_session_actor(actor_key.as_str()) {
            Ok(lock) => lock,
            Err(error) => {
                let _ = self.decrement_actor_ref_count(actor_key.as_str());
                return Err(error);
            }
        };

        Ok(SessionActorGuard {
            actor_key,
            actor_guard: Some(queue_lock.lock_owned().await),
            session_actor_locks: self.session_actor_locks.clone(),
            actor_ref_counts: self.actor_ref_counts.clone(),
            pending_turns: self.pending_turns.clone(),
            count_pending_turn: false,
        })
    }

    pub(super) async fn acquire_turn_queue_guard(
        &self,
        actor_key: String,
    ) -> CliResult<SessionActorGuard> {
        self.increment_actor_ref_count(actor_key.as_str())?;
        if let Err(error) = self.increment_pending_turn(actor_key.as_str()) {
            let _ = self.decrement_actor_ref_count(actor_key.as_str());
            return Err(error);
        }
        let queue_lock = match self.get_or_insert_session_actor(actor_key.as_str()) {
            Ok(lock) => lock,
            Err(error) => {
                let _ = self.decrement_pending_turn(actor_key.as_str());
                let _ = self.decrement_actor_ref_count(actor_key.as_str());
                return Err(error);
            }
        };

        Ok(SessionActorGuard {
            actor_key,
            actor_guard: Some(queue_lock.lock_owned().await),
            session_actor_locks: self.session_actor_locks.clone(),
            actor_ref_counts: self.actor_ref_counts.clone(),
            pending_turns: self.pending_turns.clone(),
            count_pending_turn: true,
        })
    }

    pub(super) fn get_or_insert_session_actor(
        &self,
        actor_key: &str,
    ) -> CliResult<Arc<AsyncMutex<()>>> {
        let mut guard = self
            .session_actor_locks
            .write()
            .map_err(|_error| "ACP session actor registry lock poisoned".to_owned())?;
        Ok(guard
            .entry(actor_key.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone())
    }

    pub(super) fn increment_actor_ref_count(&self, actor_key: &str) -> CliResult<()> {
        let mut guard = self
            .actor_ref_counts
            .write()
            .map_err(|_error| "ACP actor reference registry lock poisoned".to_owned())?;
        *guard.entry(actor_key.to_owned()).or_insert(0) += 1;
        Ok(())
    }

    pub(super) fn decrement_actor_ref_count(&self, actor_key: &str) -> CliResult<()> {
        let mut guard = self
            .actor_ref_counts
            .write()
            .map_err(|_error| "ACP actor reference registry lock poisoned".to_owned())?;
        if let Some(count) = guard.get_mut(actor_key) {
            if *count <= 1 {
                guard.remove(actor_key);
            } else {
                *count -= 1;
            }
        }
        Ok(())
    }

    pub(super) fn increment_pending_turn(&self, actor_key: &str) -> CliResult<()> {
        let mut guard = self
            .pending_turns
            .write()
            .map_err(|_error| "ACP pending turn registry lock poisoned".to_owned())?;
        *guard.entry(actor_key.to_owned()).or_insert(0) += 1;
        Ok(())
    }

    pub(super) fn decrement_pending_turn(&self, actor_key: &str) -> CliResult<()> {
        let mut guard = self
            .pending_turns
            .write()
            .map_err(|_error| "ACP pending turn registry lock poisoned".to_owned())?;
        if let Some(count) = guard.get_mut(actor_key) {
            if *count <= 1 {
                guard.remove(actor_key);
            } else {
                *count -= 1;
            }
        }
        Ok(())
    }

    pub(super) fn pending_turn_count_for_metadata(
        &self,
        metadata: &AcpSessionMetadata,
    ) -> CliResult<usize> {
        self.pending_turn_count(actor_key_for_metadata(metadata).as_str())
    }

    pub(super) fn actor_ref_count_for_metadata(
        &self,
        metadata: &AcpSessionMetadata,
    ) -> CliResult<usize> {
        self.actor_ref_count(actor_key_for_metadata(metadata).as_str())
    }

    pub(super) fn pending_turn_count(&self, actor_key: &str) -> CliResult<usize> {
        let guard = self
            .pending_turns
            .read()
            .map_err(|_error| "ACP pending turn registry lock poisoned".to_owned())?;
        Ok(guard.get(actor_key).copied().unwrap_or(0))
    }

    pub(super) fn actor_ref_count(&self, actor_key: &str) -> CliResult<usize> {
        let guard = self
            .actor_ref_counts
            .read()
            .map_err(|_error| "ACP actor reference registry lock poisoned".to_owned())?;
        Ok(guard.get(actor_key).copied().unwrap_or(0))
    }

    pub(super) fn active_turn(&self, actor_key: &str) -> CliResult<Option<Arc<ActiveTurnState>>> {
        let guard = self
            .active_turns
            .read()
            .map_err(|_error| "ACP active turn registry lock poisoned".to_owned())?;
        Ok(guard.get(actor_key).cloned())
    }

    pub(super) fn is_active_turn_for_metadata(
        &self,
        metadata: &AcpSessionMetadata,
    ) -> CliResult<bool> {
        Ok(self
            .active_turn(actor_key_for_metadata(metadata).as_str())?
            .is_some())
    }

    pub(super) fn record_turn_completion(&self, started_ms: u64, succeeded: bool) -> CliResult<()> {
        let duration_ms = now_ms().saturating_sub(started_ms);
        let mut guard = self
            .turn_latency_stats
            .write()
            .map_err(|_error| "ACP turn latency registry lock poisoned".to_owned())?;
        if succeeded {
            guard.completed = guard.completed.saturating_add(1);
        } else {
            guard.failed = guard.failed.saturating_add(1);
        }
        guard.total_ms = guard.total_ms.saturating_add(duration_ms);
        guard.max_ms = guard.max_ms.max(duration_ms);
        Ok(())
    }

    pub(super) fn record_error(&self, error: &str) -> CliResult<()> {
        let key = normalize_error_key(error);
        let mut guard = self
            .error_counts_by_code
            .write()
            .map_err(|_error| "ACP error registry lock poisoned".to_owned())?;
        *guard.entry(key).or_insert(0) += 1;
        Ok(())
    }

    pub(super) fn record_eviction(&self, at_ms: u64) -> CliResult<()> {
        let mut count_guard = self
            .evicted_runtime_count
            .write()
            .map_err(|_error| "ACP eviction counter lock poisoned".to_owned())?;
        *count_guard = count_guard.saturating_add(1);
        let mut ts_guard = self
            .last_evicted_at_ms
            .write()
            .map_err(|_error| "ACP last eviction lock poisoned".to_owned())?;
        *ts_guard = Some(at_ms);
        Ok(())
    }

    pub(super) fn register_active_turn(
        &self,
        actor_key: &str,
        active_turn: Arc<ActiveTurnState>,
    ) -> CliResult<()> {
        let mut guard = self
            .active_turns
            .write()
            .map_err(|_error| "ACP active turn registry lock poisoned".to_owned())?;
        guard.insert(actor_key.to_owned(), active_turn);
        Ok(())
    }

    pub(super) fn clear_active_turn(&self, actor_key: &str) -> CliResult<()> {
        let mut guard = self
            .active_turns
            .write()
            .map_err(|_error| "ACP active turn registry lock poisoned".to_owned())?;
        guard.remove(actor_key);
        Ok(())
    }
}

pub(super) struct SessionActorGuard {
    pub(super) actor_key: String,
    pub(super) actor_guard: Option<OwnedMutexGuard<()>>,
    pub(super) session_actor_locks: Arc<RwLock<BTreeMap<String, Arc<AsyncMutex<()>>>>>,
    pub(super) actor_ref_counts: Arc<RwLock<BTreeMap<String, usize>>>,
    pub(super) pending_turns: Arc<RwLock<BTreeMap<String, usize>>>,
    pub(super) count_pending_turn: bool,
}

impl Drop for SessionActorGuard {
    fn drop(&mut self) {
        self.actor_guard.take();

        if self.count_pending_turn
            && let Ok(mut guard) = self.pending_turns.write()
        {
            match guard.get_mut(self.actor_key.as_str()) {
                Some(count) if *count <= 1 => {
                    guard.remove(self.actor_key.as_str());
                }
                Some(count) => {
                    *count -= 1;
                }
                None => {}
            }
        }

        if let Ok(mut guard) = self.actor_ref_counts.write() {
            match guard.get_mut(self.actor_key.as_str()) {
                Some(count) if *count <= 1 => {
                    guard.remove(self.actor_key.as_str());
                }
                Some(count) => {
                    *count -= 1;
                }
                None => {}
            }
        }

        let should_remove_actor = self
            .actor_ref_counts
            .read()
            .map(|guard| !guard.contains_key(self.actor_key.as_str()))
            .unwrap_or(false);
        if should_remove_actor && let Ok(mut guard) = self.session_actor_locks.write() {
            guard.remove(self.actor_key.as_str());
        }
    }
}
