use super::*;
use crate::config::{MemoryMode, MemoryProfile};
use std::sync::Condvar;

#[derive(Default)]
struct SqliteMetricCapture {
    active_thread: Option<ThreadId>,
    cached_prepare_counts: HashMap<&'static str, usize>,
    summary_materialization_counts: HashMap<&'static str, usize>,
    runtime_path_normalization_counts: HashMap<&'static str, usize>,
}

#[derive(Default)]
struct SqliteRuntimeCacheMissGate {
    path: Option<PathBuf>,
    target_waiters: usize,
    waiting_threads: usize,
    released: bool,
}

fn sqlite_runtime_test_support_lock() -> &'static Mutex<()> {
    super::sqlite_runtime_test_lock()
}

fn bootstrap_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static BOOTSTRAP_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    BOOTSTRAP_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn schema_init_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    static SCHEMA_INIT_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
    SCHEMA_INIT_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn schema_repair_counts() -> &'static Mutex<HashMap<&'static str, usize>> {
    static SCHEMA_REPAIR_COUNTS: OnceLock<Mutex<HashMap<&'static str, usize>>> = OnceLock::new();
    SCHEMA_REPAIR_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sqlite_metric_capture() -> &'static Mutex<SqliteMetricCapture> {
    static SQLITE_METRIC_CAPTURE: OnceLock<Mutex<SqliteMetricCapture>> = OnceLock::new();
    SQLITE_METRIC_CAPTURE.get_or_init(|| Mutex::new(SqliteMetricCapture::default()))
}

fn sqlite_runtime_cache_miss_gate() -> &'static (Mutex<SqliteRuntimeCacheMissGate>, Condvar) {
    static SQLITE_RUNTIME_CACHE_MISS_GATE: OnceLock<(Mutex<SqliteRuntimeCacheMissGate>, Condvar)> =
        OnceLock::new();
    SQLITE_RUNTIME_CACHE_MISS_GATE.get_or_init(|| {
        (
            Mutex::new(SqliteRuntimeCacheMissGate::default()),
            Condvar::new(),
        )
    })
}

fn lock_sqlite_metric_capture() -> std::sync::MutexGuard<'static, SqliteMetricCapture> {
    sqlite_metric_capture()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn record_sqlite_bootstrap(path: &Path) {
    let normalized_path = normalize_runtime_db_path_best_effort(path);
    let mut counts = bootstrap_counts().lock().expect("bootstrap counts lock");
    let entry = counts.entry(normalized_path).or_insert(0);
    *entry += 1;
}

pub(super) fn record_sqlite_schema_init(path: &Path) {
    let normalized_path = normalize_runtime_db_path_best_effort(path);
    let mut counts = schema_init_counts()
        .lock()
        .expect("schema init counts lock");
    let entry = counts.entry(normalized_path).or_insert(0);
    *entry += 1;
}

pub(super) fn sqlite_bootstrap_count(path: &Path) -> usize {
    let normalized_path = normalize_runtime_db_path_best_effort(path);
    let counts = bootstrap_counts().lock().expect("bootstrap counts lock");
    counts.get(&normalized_path).copied().unwrap_or_default()
}

pub(super) fn sqlite_schema_init_count(path: &Path) -> usize {
    let normalized_path = normalize_runtime_db_path_best_effort(path);
    let counts = schema_init_counts()
        .lock()
        .expect("schema init counts lock");
    counts.get(&normalized_path).copied().unwrap_or_default()
}

pub(super) fn sqlite_bootstrap_count_under_prefix(prefix: &Path) -> usize {
    let normalized_prefix = normalize_runtime_db_path_best_effort(prefix);
    let counts = bootstrap_counts().lock().expect("bootstrap counts lock");
    counts
        .iter()
        .filter(|(path, _)| path.starts_with(&normalized_prefix))
        .map(|(_, count)| *count)
        .sum()
}

pub(super) fn record_sqlite_schema_repair(kind: &'static str) {
    let mut counts = schema_repair_counts()
        .lock()
        .expect("schema repair counts lock");
    let entry = counts.entry(kind).or_insert(0);
    *entry += 1;
}

pub(super) fn sqlite_schema_repair_count(kind: &'static str) -> usize {
    let counts = schema_repair_counts()
        .lock()
        .expect("schema repair counts lock");
    counts.get(kind).copied().unwrap_or_default()
}

pub(super) fn reset_sqlite_schema_repair_metrics() {
    schema_repair_counts()
        .lock()
        .expect("schema repair counts lock")
        .clear();
}

pub(super) fn record_cached_prepare(sql: &'static str) {
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture.cached_prepare_counts.entry(sql).or_insert(0);
        *entry += 1;
    }
}

pub(super) fn cached_prepare_count_for_sql_fragment(fragment: &str) -> usize {
    let capture = lock_sqlite_metric_capture();
    capture
        .cached_prepare_counts
        .iter()
        .filter(|(sql, _)| sql.contains(fragment))
        .map(|(_, count)| *count)
        .sum()
}

pub(super) fn reset_cached_prepare_metrics() {
    lock_sqlite_metric_capture().cached_prepare_counts.clear();
}

pub(super) fn record_runtime_path_normalization_full() {
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture
            .runtime_path_normalization_counts
            .entry("full")
            .or_insert(0);
        *entry += 1;
    }
}

pub(super) fn record_runtime_path_normalization_alias_hit() {
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture
            .runtime_path_normalization_counts
            .entry("alias_hit")
            .or_insert(0);
        *entry += 1;
    }
}

pub(super) fn runtime_path_normalization_full_count() -> usize {
    let capture = lock_sqlite_metric_capture();
    capture
        .runtime_path_normalization_counts
        .get("full")
        .copied()
        .unwrap_or_default()
}

pub(super) fn runtime_path_normalization_alias_hit_count() -> usize {
    let capture = lock_sqlite_metric_capture();
    capture
        .runtime_path_normalization_counts
        .get("alias_hit")
        .copied()
        .unwrap_or_default()
}

pub(super) fn configure_sqlite_runtime_cache_miss(path: &Path, target_waiters: usize) {
    let normalized_path = normalize_runtime_db_path_best_effort(path);
    let (gate_lock, gate_condvar) = sqlite_runtime_cache_miss_gate();
    let mut gate = gate_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    gate.path = Some(normalized_path);
    gate.target_waiters = target_waiters;
    gate.waiting_threads = 0;
    gate.released = false;
    gate_condvar.notify_all();
}

pub(super) fn wait_for_sqlite_runtime_cache_miss(path: &Path) {
    let normalized_path = normalize_runtime_db_path_best_effort(path);
    let (gate_lock, gate_condvar) = sqlite_runtime_cache_miss_gate();
    let mut gate = gate_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(configured_path) = gate.path.as_ref() else {
        return;
    };
    if *configured_path != normalized_path {
        return;
    }
    if gate.released {
        return;
    }

    gate.waiting_threads += 1;
    if gate.waiting_threads >= gate.target_waiters {
        gate.released = true;
        gate_condvar.notify_all();
        return;
    }

    while !gate.released {
        gate = gate_condvar
            .wait(gate)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

pub(super) fn clear_sqlite_runtime_cache_miss() {
    let (gate_lock, gate_condvar) = sqlite_runtime_cache_miss_gate();
    let mut gate = gate_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    gate.path = None;
    gate.target_waiters = 0;
    gate.waiting_threads = 0;
    gate.released = false;
    gate_condvar.notify_all();
}

pub(super) fn record_summary_streaming_query(kind: &'static str) {
    let key = match kind {
        "rebuild" => "streaming_rebuild",
        "catch_up" => "streaming_catch_up",
        _ => kind,
    };
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture
            .summary_materialization_counts
            .entry(key)
            .or_insert(0);
        *entry += 1;
    }
}

pub(super) fn summary_buffered_query_count(kind: &'static str) -> usize {
    let key = match kind {
        "rebuild" => "buffered_rebuild",
        "catch_up" => "buffered_catch_up",
        _ => kind,
    };
    let capture = lock_sqlite_metric_capture();
    capture
        .summary_materialization_counts
        .get(key)
        .copied()
        .unwrap_or_default()
}

pub(super) fn summary_streaming_query_count(kind: &'static str) -> usize {
    let key = match kind {
        "rebuild" => "streaming_rebuild",
        "catch_up" => "streaming_catch_up",
        _ => kind,
    };
    let capture = lock_sqlite_metric_capture();
    capture
        .summary_materialization_counts
        .get(key)
        .copied()
        .unwrap_or_default()
}

pub(super) fn record_summary_payload_decode() {
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture
            .summary_materialization_counts
            .entry("payload_decode")
            .or_insert(0);
        *entry += 1;
    }
}

pub(super) fn record_summary_row_observed() {
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture
            .summary_materialization_counts
            .entry("row_observed")
            .or_insert(0);
        *entry += 1;
    }
}

pub(super) fn summary_row_observed_count() -> usize {
    let capture = lock_sqlite_metric_capture();
    capture
        .summary_materialization_counts
        .get("row_observed")
        .copied()
        .unwrap_or_default()
}

pub(super) fn summary_frontier_probe_count(kind: &'static str) -> usize {
    let key = match kind {
        "rebuild" => "frontier_probe_rebuild",
        "catch_up" => "frontier_probe_catch_up",
        _ => kind,
    };
    let capture = lock_sqlite_metric_capture();
    capture
        .summary_materialization_counts
        .get(key)
        .copied()
        .unwrap_or_default()
}

pub(super) fn record_summary_frontier_probe(kind: &'static str) {
    let key = match kind {
        "rebuild" => "frontier_probe_rebuild",
        "catch_up" => "frontier_probe_catch_up",
        _ => kind,
    };
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    if capture.active_thread == Some(current_thread) {
        let entry = capture
            .summary_materialization_counts
            .entry(key)
            .or_insert(0);
        *entry += 1;
    }
}

pub(super) fn summary_payload_decode_count() -> usize {
    let capture = lock_sqlite_metric_capture();
    capture
        .summary_materialization_counts
        .get("payload_decode")
        .copied()
        .unwrap_or_default()
}

pub(super) fn summary_normalization_count() -> usize {
    let capture = lock_sqlite_metric_capture();
    capture
        .summary_materialization_counts
        .get("normalization")
        .copied()
        .unwrap_or_default()
}

pub(super) fn reset_summary_materialization_metrics() {
    lock_sqlite_metric_capture()
        .summary_materialization_counts
        .clear();
}

pub(super) fn begin_sqlite_metric_capture() {
    let current_thread = std::thread::current().id();
    let mut capture = lock_sqlite_metric_capture();
    capture.active_thread = Some(current_thread);
    capture.cached_prepare_counts.clear();
    capture.summary_materialization_counts.clear();
    capture.runtime_path_normalization_counts.clear();
}

pub(super) fn end_sqlite_metric_capture() {
    let mut capture = lock_sqlite_metric_capture();
    capture.active_thread = None;
    capture.cached_prepare_counts.clear();
    capture.summary_materialization_counts.clear();
    capture.runtime_path_normalization_counts.clear();
}

#[test]
fn prompt_window_turn_is_visible_filters_internal_persisted_records() {
    let conversation_event = crate::memory::build_conversation_event_content(
        "provider_prompt_frame_snapshot",
        serde_json::json!({"phase": "initial"}),
    );
    let tool_decision = crate::memory::build_tool_decision_content(
        "turn-1",
        "call-1",
        serde_json::json!({"decision": "allow"}),
    );
    let plain_assistant = "assistant reply";
    let plain_user = "user prompt";

    assert!(!prompt_window_turn_is_visible(
        "session-visible-filter",
        "assistant",
        conversation_event.as_str()
    ));
    assert!(!prompt_window_turn_is_visible(
        "session-visible-filter",
        "assistant",
        tool_decision.as_str()
    ));
    assert!(prompt_window_turn_is_visible(
        "session-visible-filter",
        "assistant",
        plain_assistant
    ));
    assert!(prompt_window_turn_is_visible(
        "session-visible-filter",
        "user",
        plain_user
    ));
}

#[test]
fn prompt_window_mixed_overflow_regression() {
    let runtime_test_support_lock = sqlite_runtime_test_support_lock();
    let guard_result = runtime_test_support_lock.lock();
    let _guard = match guard_result {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-prompt-window-mixed-overflow-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("prompt-window-mixed-overflow.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = MemoryRuntimeConfig {
        profile: MemoryProfile::WindowOnly,
        mode: MemoryMode::WindowOnly,
        sqlite_path: Some(db_path.clone()),
        sliding_window: 3,
        ..MemoryRuntimeConfig::default()
    };
    let hidden_inner = crate::memory::build_conversation_event_content(
        "provider_prompt_frame_snapshot",
        serde_json::json!({"phase": "initial"}),
    );
    let hidden_tail = crate::memory::build_tool_decision_content(
        "turn-4",
        "call-1",
        serde_json::json!({"decision": "allow"}),
    );

    assert!(!prompt_window_turn_is_visible(
        "prompt-window-mixed-overflow-session",
        "assistant",
        hidden_inner.as_str()
    ));
    assert!(!prompt_window_turn_is_visible(
        "prompt-window-mixed-overflow-session",
        "assistant",
        hidden_tail.as_str()
    ));
    assert!(prompt_window_turn_is_visible(
        "prompt-window-mixed-overflow-session",
        "assistant",
        "visible 4"
    ));

    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "user",
        "visible 1",
        &config,
    )
    .expect("append visible turn 1");
    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "assistant",
        hidden_inner.as_str(),
        &config,
    )
    .expect("append hidden inner record");
    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "assistant",
        "visible 2",
        &config,
    )
    .expect("append visible turn 2");
    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "user",
        "visible 3",
        &config,
    )
    .expect("append visible turn 3");
    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "assistant",
        hidden_tail.as_str(),
        &config,
    )
    .expect("append hidden tail record");
    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "assistant",
        "visible 4",
        &config,
    )
    .expect("append visible turn 4");
    append_turn_direct(
        "prompt-window-mixed-overflow-session",
        "user",
        "visible 5",
        &config,
    )
    .expect("append visible turn 5");

    let snapshot = load_context_snapshot("prompt-window-mixed-overflow-session", &config)
        .expect("load mixed-overflow context snapshot");
    let window_contents = snapshot
        .window_turns
        .iter()
        .map(|turn| turn.content.as_str())
        .collect::<Vec<_>>();

    assert_eq!(snapshot.window_turns.len(), 3);
    assert_eq!(window_contents, vec!["visible 3", "visible 4", "visible 5"]);
    assert!(snapshot.summary_body.is_none());

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

pub(super) fn reset_test_state() {
    bootstrap_counts()
        .lock()
        .expect("bootstrap counts lock")
        .clear();
    schema_init_counts()
        .lock()
        .expect("schema init counts lock")
        .clear();
    reset_sqlite_schema_repair_metrics();
    end_sqlite_metric_capture();
    reset_cached_prepare_metrics();
    reset_summary_materialization_metrics();
    clear_sqlite_runtime_cache_miss();
}
