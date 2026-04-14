use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use tokio::runtime::Handle;
use tokio::time::{Duration, sleep};

use crate::config::LoongClawConfig;
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::session::frozen_result::FrozenResult;
use crate::session::repository::{NewSessionEvent, SessionRepository};

pub(crate) const DELEGATE_RESULTS_ANNOUNCED_EVENT_KIND: &str = "delegate_results_announced";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateAnnounceSettings {
    pub debounce_ms: u64,
    pub max_batch: usize,
}

impl DelegateAnnounceSettings {
    pub(crate) fn from_config(config: &LoongClawConfig) -> Self {
        let delegate_config = &config.tools.delegate;

        Self {
            debounce_ms: delegate_config.announce_debounce_ms,
            max_batch: delegate_config.announce_max_batch.max(1),
        }
    }
}

#[derive(Debug)]
struct DelegateAnnounceQueueState {
    pending_child_session_ids: VecDeque<String>,
    last_enqueued_at: Instant,
    draining: bool,
    settings: DelegateAnnounceSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct DelegateResultAnnouncement {
    child_session_id: String,
    label: Option<String>,
    state: String,
    status: String,
    recorded_at: i64,
    frozen_result: Option<FrozenResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DelegateResultOverflowSummary {
    omitted_count: usize,
    omitted_child_session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DelegateAnnounceBatch {
    child_session_ids: Vec<String>,
    settings: DelegateAnnounceSettings,
}

pub(crate) fn enqueue_delegate_result_announce(
    memory_config: MemoryRuntimeConfig,
    parent_session_id: String,
    child_session_id: String,
    settings: DelegateAnnounceSettings,
) {
    let queue_key = delegate_announce_queue_key(&memory_config, parent_session_id.as_str());
    let immediate_flush = settings.debounce_ms == 0;
    let should_spawn = {
        let queues = delegate_announce_queues();
        let mut queues = queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let queue = queues
            .entry(queue_key.clone())
            .or_insert_with(|| DelegateAnnounceQueueState {
                pending_child_session_ids: VecDeque::new(),
                last_enqueued_at: Instant::now(),
                draining: false,
                settings: settings.clone(),
            });
        queue.pending_child_session_ids.push_back(child_session_id);
        queue.last_enqueued_at = Instant::now();
        queue.settings = settings;
        let already_draining = queue.draining;
        if !already_draining {
            queue.draining = true;
        }
        !already_draining
    };

    if !should_spawn {
        return;
    }

    if immediate_flush {
        loop {
            let batch = take_delegate_announce_batch(queue_key.as_str());
            let Some(batch) = batch else {
                let has_pending = finish_delegate_announce_batch(queue_key.as_str());
                if !has_pending {
                    return;
                }
                continue;
            };

            let flush_result =
                flush_delegate_announce_batch(&memory_config, parent_session_id.as_str(), &batch);
            if let Err(error) = flush_result {
                restore_delegate_announce_batch(queue_key.as_str(), &batch);
                tracing::warn!(
                    parent_session_id = %parent_session_id,
                    error = %error,
                    "delegate announce immediate flush failed"
                );
                let has_pending = finish_delegate_announce_batch(queue_key.as_str());
                if !has_pending {
                    return;
                }

                if let Ok(runtime_handle) = Handle::try_current() {
                    let retry_memory_config = memory_config;
                    let retry_queue_key = queue_key;
                    let retry_parent_session_id = parent_session_id;
                    runtime_handle.spawn(async move {
                        drain_delegate_announce_queue(
                            retry_memory_config,
                            retry_queue_key,
                            retry_parent_session_id,
                        )
                        .await;
                    });
                    return;
                }

                pause_delegate_announce_queue(queue_key.as_str());
                return;
            }

            let has_pending = finish_delegate_announce_batch(queue_key.as_str());
            if !has_pending {
                return;
            }
        }
    }

    let handle = Handle::try_current();
    let Ok(handle) = handle else {
        tracing::warn!(
            parent_session_id = %parent_session_id,
            "delegate announce queue skipped because no tokio runtime was available"
        );
        pause_delegate_announce_queue(queue_key.as_str());
        return;
    };

    handle.spawn(async move {
        drain_delegate_announce_queue(memory_config, queue_key, parent_session_id).await;
    });
}

async fn drain_delegate_announce_queue(
    memory_config: MemoryRuntimeConfig,
    queue_key: String,
    parent_session_id: String,
) {
    loop {
        let wait_duration = next_delegate_announce_wait_duration(queue_key.as_str());
        let Some(wait_duration) = wait_duration else {
            return;
        };
        if !wait_duration.is_zero() {
            sleep(wait_duration).await;
            continue;
        }

        let batch = take_delegate_announce_batch(queue_key.as_str());
        let Some(batch) = batch else {
            continue;
        };

        let flush_result =
            flush_delegate_announce_batch(&memory_config, parent_session_id.as_str(), &batch);
        if let Err(error) = flush_result {
            restore_delegate_announce_batch(queue_key.as_str(), &batch);
            tracing::warn!(
                parent_session_id = %parent_session_id,
                error = %error,
                "delegate announce queue flush failed"
            );
        }

        let has_pending = finish_delegate_announce_batch(queue_key.as_str());
        if !has_pending {
            return;
        }
    }
}

fn flush_delegate_announce_batch(
    memory_config: &MemoryRuntimeConfig,
    parent_session_id: &str,
    batch: &DelegateAnnounceBatch,
) -> Result<(), String> {
    let repo = SessionRepository::new(memory_config)?;

    let deduped_child_session_ids = dedupe_child_session_ids(batch.child_session_ids.clone());
    let announce_results =
        load_delegate_announce_results(&repo, parent_session_id, &deduped_child_session_ids)?;
    if announce_results.is_empty() {
        return Ok(());
    }

    let overflow_summary =
        build_delegate_announce_overflow_summary(&announce_results, batch.settings.max_batch);
    let total_result_count = announce_results.len();
    let visible_results =
        trim_delegate_announce_results(announce_results, batch.settings.max_batch);
    let announce_kind = if visible_results.len() == 1 && overflow_summary.is_none() {
        "delegate_result"
    } else {
        "batch_delegate_results"
    };

    let payload = serde_json::json!({
        "announce_kind": announce_kind,
        "trigger_turn": false,
        "result_count": total_result_count,
        "visible_result_count": visible_results.len(),
        "results": visible_results,
        "overflow_summary": overflow_summary,
    });
    let event = NewSessionEvent {
        session_id: parent_session_id.to_owned(),
        event_kind: DELEGATE_RESULTS_ANNOUNCED_EVENT_KIND.to_owned(),
        actor_session_id: None,
        payload_json: payload,
    };
    let append_result = repo.append_event_if_session_active(event)?;
    let append_succeeded = append_result.is_some();
    if !append_succeeded {
        return Ok(());
    }

    Ok(())
}

fn load_delegate_announce_results(
    repo: &SessionRepository,
    parent_session_id: &str,
    child_session_ids: &[String],
) -> Result<Vec<DelegateResultAnnouncement>, String> {
    let mut results = Vec::new();

    for child_session_id in child_session_ids {
        let child_session = repo.load_session_summary_with_legacy_fallback(child_session_id)?;
        let Some(child_session) = child_session else {
            continue;
        };
        let child_parent_session_id = child_session.parent_session_id.clone();
        if child_parent_session_id.as_deref() != Some(parent_session_id) {
            continue;
        }
        let terminal_outcome = repo.load_terminal_outcome(child_session_id)?;
        let Some(terminal_outcome) = terminal_outcome else {
            continue;
        };

        let result = DelegateResultAnnouncement {
            child_session_id: child_session.session_id,
            label: child_session.label,
            state: child_session.state.as_str().to_owned(),
            status: terminal_outcome.status,
            recorded_at: terminal_outcome.recorded_at,
            frozen_result: terminal_outcome.frozen_result,
        };
        results.push(result);
    }

    Ok(results)
}

fn dedupe_child_session_ids(child_session_ids: Vec<String>) -> Vec<String> {
    let mut pending = child_session_ids;
    let mut deduped = Vec::new();

    while let Some(child_session_id) = pending.pop() {
        let already_present = deduped.iter().any(|current| current == &child_session_id);
        if already_present {
            continue;
        }
        deduped.push(child_session_id);
    }

    deduped.reverse();

    deduped
}

fn trim_delegate_announce_results(
    results: Vec<DelegateResultAnnouncement>,
    max_batch: usize,
) -> Vec<DelegateResultAnnouncement> {
    if results.len() <= max_batch {
        return results;
    }

    let keep_from_index = results.len().saturating_sub(max_batch);
    results.into_iter().skip(keep_from_index).collect()
}

fn build_delegate_announce_overflow_summary(
    results: &[DelegateResultAnnouncement],
    max_batch: usize,
) -> Option<DelegateResultOverflowSummary> {
    if results.len() <= max_batch {
        return None;
    }

    let omitted_count = results.len().saturating_sub(max_batch);
    let omitted_child_session_ids = results
        .iter()
        .take(omitted_count)
        .map(|result| result.child_session_id.clone())
        .collect();

    Some(DelegateResultOverflowSummary {
        omitted_count,
        omitted_child_session_ids,
    })
}

fn next_delegate_announce_wait_duration(parent_session_id: &str) -> Option<Duration> {
    let queues = delegate_announce_queues();
    let queues = queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let queue = queues.get(parent_session_id)?;
    let debounce_duration = Duration::from_millis(queue.settings.debounce_ms);
    let elapsed_duration = queue.last_enqueued_at.elapsed();
    if elapsed_duration >= debounce_duration {
        return Some(Duration::ZERO);
    }

    let wait_duration = debounce_duration - elapsed_duration;

    Some(wait_duration)
}

fn take_delegate_announce_batch(parent_session_id: &str) -> Option<DelegateAnnounceBatch> {
    let queues = delegate_announce_queues();
    let mut queues = queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let queue = queues.get_mut(parent_session_id)?;
    let debounce_duration = Duration::from_millis(queue.settings.debounce_ms);
    let elapsed_duration = queue.last_enqueued_at.elapsed();
    if elapsed_duration < debounce_duration {
        return None;
    }

    let child_session_ids = queue.pending_child_session_ids.drain(..).collect();
    let settings = queue.settings.clone();

    Some(DelegateAnnounceBatch {
        child_session_ids,
        settings,
    })
}

fn finish_delegate_announce_batch(parent_session_id: &str) -> bool {
    let queues = delegate_announce_queues();
    let mut queues = queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(queue) = queues.get_mut(parent_session_id) else {
        return false;
    };
    let has_pending = !queue.pending_child_session_ids.is_empty();
    if has_pending {
        return true;
    }

    queues.remove(parent_session_id);

    false
}

fn restore_delegate_announce_batch(parent_session_id: &str, batch: &DelegateAnnounceBatch) {
    let queues = delegate_announce_queues();
    let mut queues = queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(queue) = queues.get_mut(parent_session_id) else {
        return;
    };

    let child_session_ids = batch.child_session_ids.iter().rev();
    for child_session_id in child_session_ids {
        queue
            .pending_child_session_ids
            .push_front(child_session_id.clone());
    }
    queue.last_enqueued_at = Instant::now();
    queue.settings = batch.settings.clone();
}

fn pause_delegate_announce_queue(parent_session_id: &str) {
    let queues = delegate_announce_queues();
    let mut queues = queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(queue) = queues.get_mut(parent_session_id) else {
        return;
    };

    queue.draining = false;
}

fn delegate_announce_queue_key(
    memory_config: &MemoryRuntimeConfig,
    parent_session_id: &str,
) -> String {
    let sqlite_path = memory_config.sqlite_path.clone();
    let sqlite_path = sqlite_path
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "in-memory".to_owned());

    format!("{sqlite_path}::{parent_session_id}")
}

fn delegate_announce_queues() -> &'static Mutex<HashMap<String, DelegateAnnounceQueueState>> {
    static QUEUES: OnceLock<Mutex<HashMap<String, DelegateAnnounceQueueState>>> = OnceLock::new();

    QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(crate) fn reset_delegate_announce_queues_for_tests() {
    let queues = delegate_announce_queues();
    let mut queues = queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    queues.clear();
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::OnceLock;

    use serde_json::json;
    use tokio::sync::Mutex as AsyncMutex;
    use tokio::time::{Duration, sleep};

    use super::{
        DELEGATE_RESULTS_ANNOUNCED_EVENT_KIND, DelegateAnnounceSettings,
        delegate_announce_queue_key, drain_delegate_announce_queue,
        enqueue_delegate_result_announce, reset_delegate_announce_queues_for_tests,
    };
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::frozen_result::{FrozenContent, FrozenResult};
    use crate::session::repository::{
        FinalizeSessionTerminalRequest, NewSessionRecord, SessionKind, SessionRepository,
        SessionState,
    };

    const DELEGATE_ANNOUNCE_EVENT_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-announce-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);

        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn announce_test_lock() -> &'static AsyncMutex<()> {
        static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();

        LOCK.get_or_init(|| AsyncMutex::new(()))
    }

    fn create_parent_session(repo: &SessionRepository, session_id: &str, state: SessionState) {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Parent".to_owned()),
            state,
        })
        .expect("create parent session");
    }

    fn create_child_session(
        repo: &SessionRepository,
        parent_session_id: &str,
        child_session_id: &str,
        label: &str,
        frozen_text: &str,
    ) {
        repo.create_session(NewSessionRecord {
            session_id: child_session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some(parent_session_id.to_owned()),
            label: Some(label.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child session");
        let frozen_result = FrozenResult {
            content: FrozenContent::Text(frozen_text.to_owned()),
            captured_at: std::time::SystemTime::now(),
            byte_len: frozen_text.len(),
            truncated: false,
        };
        repo.finalize_session_terminal(
            child_session_id,
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some(parent_session_id.to_owned()),
                event_payload_json: json!({
                    "result": "ok",
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": child_session_id,
                    "final_output": frozen_text,
                }),
                frozen_result: Some(frozen_result),
            },
        )
        .expect("finalize child session");
    }

    async fn wait_for_parent_announce_event(
        memory_config: &MemoryRuntimeConfig,
        parent_session_id: &str,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + DELEGATE_ANNOUNCE_EVENT_WAIT_TIMEOUT;

        loop {
            let repo = SessionRepository::new(memory_config).expect("session repository");
            let events = repo
                .list_recent_events(parent_session_id, 20)
                .expect("list parent events");
            let announce_event = events
                .into_iter()
                .find(|event| event.event_kind == DELEGATE_RESULTS_ANNOUNCED_EVENT_KIND);
            if let Some(announce_event) = announce_event {
                return announce_event.payload_json;
            }

            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for delegate announce event");
            }

            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn flush_delegate_announce_queue_for_tests(
        memory_config: &MemoryRuntimeConfig,
        parent_session_id: &str,
    ) {
        let queue_key = delegate_announce_queue_key(memory_config, parent_session_id);
        let parent_session_id = parent_session_id.to_owned();
        let memory_config = memory_config.clone();
        drain_delegate_announce_queue(memory_config, queue_key, parent_session_id).await;
    }

    #[tokio::test]
    async fn delegate_announce_queue_delivers_single_result_to_parent_session_events() {
        let _guard = announce_test_lock().lock().await;
        reset_delegate_announce_queues_for_tests();
        let memory_config = isolated_memory_config("single");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        create_parent_session(&repo, "root-session", SessionState::Ready);
        create_child_session(&repo, "root-session", "child-session", "Research", "done");

        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-session".to_owned(),
            DelegateAnnounceSettings {
                debounce_ms: 0,
                max_batch: 20,
            },
        );

        let payload = wait_for_parent_announce_event(&memory_config, "root-session").await;
        let results = payload["results"].as_array().expect("results array");

        assert_eq!(payload["announce_kind"], "delegate_result");
        assert_eq!(payload["trigger_turn"], false);
        assert_eq!(payload["result_count"], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["child_session_id"], "child-session");
        assert_eq!(results[0]["label"], "Research");
        assert_eq!(results[0]["frozen_result"]["content"]["text"], "done");
    }

    #[tokio::test]
    async fn delegate_announce_queue_batches_children_completed_within_debounce_window() {
        let _guard = announce_test_lock().lock().await;
        reset_delegate_announce_queues_for_tests();
        let memory_config = isolated_memory_config("batch");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        create_parent_session(&repo, "root-session", SessionState::Ready);
        create_child_session(&repo, "root-session", "child-1", "One", "alpha");
        create_child_session(&repo, "root-session", "child-2", "Two", "beta");
        create_child_session(&repo, "root-session", "child-3", "Three", "gamma");

        let settings = DelegateAnnounceSettings {
            debounce_ms: 100,
            max_batch: 20,
        };
        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-1".to_owned(),
            settings.clone(),
        );
        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-2".to_owned(),
            settings.clone(),
        );
        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-3".to_owned(),
            settings,
        );
        flush_delegate_announce_queue_for_tests(&memory_config, "root-session").await;

        let payload = wait_for_parent_announce_event(&memory_config, "root-session").await;
        let results = payload["results"].as_array().expect("results array");
        let events = repo
            .list_recent_events("root-session", 20)
            .expect("list parent events");
        let announce_events: Vec<_> = events
            .into_iter()
            .filter(|event| event.event_kind == DELEGATE_RESULTS_ANNOUNCED_EVENT_KIND)
            .collect();

        assert_eq!(announce_events.len(), 1);
        assert_eq!(payload["announce_kind"], "batch_delegate_results");
        assert_eq!(payload["result_count"], 3);
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn delegate_announce_queue_summarizes_oldest_results_when_batch_limit_is_exceeded() {
        let _guard = announce_test_lock().lock().await;
        reset_delegate_announce_queues_for_tests();
        let memory_config = isolated_memory_config("overflow");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        create_parent_session(&repo, "root-session", SessionState::Ready);
        create_child_session(&repo, "root-session", "child-1", "One", "alpha");
        create_child_session(&repo, "root-session", "child-2", "Two", "beta");
        create_child_session(&repo, "root-session", "child-3", "Three", "gamma");

        let settings = DelegateAnnounceSettings {
            debounce_ms: 100,
            max_batch: 2,
        };
        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-1".to_owned(),
            settings.clone(),
        );
        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-2".to_owned(),
            settings.clone(),
        );
        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-3".to_owned(),
            settings,
        );
        flush_delegate_announce_queue_for_tests(&memory_config, "root-session").await;

        let payload = wait_for_parent_announce_event(&memory_config, "root-session").await;
        let results = payload["results"].as_array().expect("results array");

        assert_eq!(payload["announce_kind"], "batch_delegate_results");
        assert_eq!(payload["overflow_summary"]["omitted_count"], 1);
        assert_eq!(
            payload["overflow_summary"]["omitted_child_session_ids"][0],
            "child-1"
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["child_session_id"], "child-2");
        assert_eq!(results[1]["child_session_id"], "child-3");
    }

    #[tokio::test]
    async fn delegate_announce_queue_drops_delivery_when_parent_is_terminal() {
        let _guard = announce_test_lock().lock().await;
        reset_delegate_announce_queues_for_tests();
        let memory_config = isolated_memory_config("drop-terminal-parent");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        create_parent_session(&repo, "root-session", SessionState::Completed);
        create_child_session(&repo, "root-session", "child-session", "Research", "done");

        enqueue_delegate_result_announce(
            memory_config,
            "root-session".to_owned(),
            "child-session".to_owned(),
            DelegateAnnounceSettings {
                debounce_ms: 0,
                max_batch: 20,
            },
        );

        sleep(Duration::from_millis(50)).await;

        let events = repo
            .list_recent_events("root-session", 20)
            .expect("list parent events");
        let announce_events: Vec<_> = events
            .into_iter()
            .filter(|event| event.event_kind == DELEGATE_RESULTS_ANNOUNCED_EVENT_KIND)
            .collect();

        assert!(announce_events.is_empty());
    }

    #[test]
    fn delegate_announce_queue_keeps_pending_batch_when_no_runtime_is_available() {
        let _guard = announce_test_lock().blocking_lock();
        reset_delegate_announce_queues_for_tests();
        let memory_config = isolated_memory_config("no-runtime");

        enqueue_delegate_result_announce(
            memory_config.clone(),
            "root-session".to_owned(),
            "child-session".to_owned(),
            DelegateAnnounceSettings {
                debounce_ms: 100,
                max_batch: 20,
            },
        );

        let queue_key = super::delegate_announce_queue_key(&memory_config, "root-session");
        let queues = super::delegate_announce_queues();
        let queues = queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let queue = queues
            .get(queue_key.as_str())
            .expect("queue should remain available");
        let pending_child_session_ids = queue.pending_child_session_ids.clone();
        let pending_child_session_ids: Vec<_> = pending_child_session_ids.into_iter().collect();

        assert_eq!(pending_child_session_ids, vec!["child-session".to_owned()]);
        assert!(!queue.draining);
    }
}
