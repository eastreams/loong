use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type InternalEventSink = Arc<dyn Fn(&str, Value) + Send + Sync + 'static>;
const DEFAULT_INTERNAL_EVENT_SEGMENT_ID: &str = "segment-000001";
const LEGACY_INTERNAL_EVENT_SEGMENT_ID: &str = "legacy";
const DEFAULT_INTERNAL_EVENT_SEGMENT_MAX_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalEventJournalRecord {
    pub line_cursor: u64,
    pub event_name: String,
    pub payload: Value,
    pub recorded_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct InternalEventJournalCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    pub line_cursor: u64,
    pub byte_offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InternalEventJournalSegment {
    segment_id: String,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InternalEventJournalLayout {
    segments: Vec<InternalEventJournalSegment>,
    active_segment: InternalEventJournalSegment,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct InternalEventJournalState {
    #[serde(default = "default_internal_event_journal_state_schema_version")]
    schema_version: u32,
    active_segment_id: String,
    #[serde(default)]
    segments: Vec<InternalEventJournalStateSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct InternalEventJournalStateSegment {
    segment_id: String,
    status: InternalEventJournalSegmentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sealed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum InternalEventJournalSegmentStatus {
    Active,
    Sealed,
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InternalEventJournalSegmentInfo {
    pub segment_id: String,
    pub path: String,
    pub is_active: bool,
    pub status: String,
    pub created_at_ms: Option<i64>,
    pub sealed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InternalEventJournalLayoutInfo {
    pub active_segment_id: String,
    pub segments: Vec<InternalEventJournalSegmentInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalEventJournalGcPolicy {
    pub retain_floor_segment_id: Option<String>,
    pub retain_last_sealed_segments: usize,
    pub retain_min_age_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InternalEventJournalGcDecision {
    pub segment_id: String,
    pub path: String,
    pub status: String,
    pub created_at_ms: Option<i64>,
    pub sealed_at_ms: Option<i64>,
    pub action: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InternalEventJournalGcPlan {
    pub active_segment_id: String,
    pub retain_floor_segment_id: Option<String>,
    pub retain_last_sealed_segments: usize,
    pub retain_min_age_ms: Option<u64>,
    pub decisions: Vec<InternalEventJournalGcDecision>,
}

fn internal_event_sink_registry() -> &'static RwLock<Option<InternalEventSink>> {
    static REGISTRY: OnceLock<RwLock<Option<InternalEventSink>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(None))
}

pub fn install_internal_event_sink(sink: InternalEventSink) {
    let registry = internal_event_sink_registry();
    let mut guard = registry
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(sink);
}

pub fn emit_internal_event(event_name: &str, payload: Value) {
    let _ = append_internal_event_journal_record(event_name, &payload);
    let registry = internal_event_sink_registry();
    let sink = registry
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(sink) = sink {
        sink(event_name, payload);
    }
}

pub fn append_internal_event_to_journal(event_name: &str, payload: &Value) -> Result<(), String> {
    append_internal_event_journal_record(event_name, payload)
}

pub fn emit_internal_event_with_metadata(event_name: &str, source_surface: &str, payload: Value) {
    let metadata = json!({
        "event_name": event_name,
        "source_surface": source_surface,
    });
    match payload {
        Value::Object(mut object) => {
            object.insert("_automation".to_owned(), metadata);
            emit_internal_event(event_name, Value::Object(object));
        }
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_)
        | other @ Value::Array(_) => emit_internal_event(
            event_name,
            json!({
                "_automation": metadata,
                "value": other,
            }),
        ),
    }
}

pub fn internal_event_journal_path() -> PathBuf {
    current_internal_event_journal_segment_path()
}

pub fn probe_internal_event_journal_runtime_ready() -> Result<(), String> {
    let path = internal_event_journal_path();
    let journal = open_internal_event_journal(path.as_path())?;
    lock_internal_event_journal(&journal, path.as_path())?;
    unlock_internal_event_journal(&journal, path.as_path())
}

pub fn inspect_internal_event_journal_layout() -> Result<InternalEventJournalLayoutInfo, String> {
    let layout = discover_internal_event_journal_layout()?;
    let state = load_internal_event_journal_state()?;
    Ok(InternalEventJournalLayoutInfo {
        active_segment_id: layout.active_segment.segment_id.clone(),
        segments: layout
            .segments
            .iter()
            .map(|segment| InternalEventJournalSegmentInfo {
                status: state
                    .as_ref()
                    .and_then(|value| {
                        value
                            .segments
                            .iter()
                            .find(|entry| entry.segment_id == segment.segment_id)
                    })
                    .map(|entry| {
                        match entry.status {
                            InternalEventJournalSegmentStatus::Active => "active",
                            InternalEventJournalSegmentStatus::Sealed => "sealed",
                            InternalEventJournalSegmentStatus::Legacy => "legacy",
                        }
                        .to_owned()
                    })
                    .unwrap_or_else(|| {
                        if segment.segment_id == layout.active_segment.segment_id {
                            "active".to_owned()
                        } else if segment.segment_id == LEGACY_INTERNAL_EVENT_SEGMENT_ID {
                            "legacy".to_owned()
                        } else {
                            "sealed".to_owned()
                        }
                    }),
                created_at_ms: state
                    .as_ref()
                    .and_then(|value| {
                        value
                            .segments
                            .iter()
                            .find(|entry| entry.segment_id == segment.segment_id)
                    })
                    .and_then(|entry| entry.created_at_ms),
                sealed_at_ms: state
                    .as_ref()
                    .and_then(|value| {
                        value
                            .segments
                            .iter()
                            .find(|entry| entry.segment_id == segment.segment_id)
                    })
                    .and_then(|entry| entry.sealed_at_ms),
                segment_id: segment.segment_id.clone(),
                path: segment.path.display().to_string(),
                is_active: segment.segment_id == layout.active_segment.segment_id,
            })
            .collect(),
    })
}

pub fn repair_internal_event_journal_state() -> Result<InternalEventJournalLayoutInfo, String> {
    let existing_state = load_internal_event_journal_state()?;
    let mut layout = discover_internal_event_journal_layout()?;
    let shadow_active_segment_id = load_internal_event_active_segment_shadow_id()?;
    let active_segment_path = layout.active_segment.path.clone();
    if !active_segment_path.exists()
        && let Some(shadow_active_segment_id) = shadow_active_segment_id
    {
        let shadow_active_path = if shadow_active_segment_id == LEGACY_INTERNAL_EVENT_SEGMENT_ID {
            legacy_internal_event_journal_segment().path
        } else {
            internal_event_segment_path(shadow_active_segment_id.as_str())
        };
        if shadow_active_path.exists() {
            if !layout
                .segments
                .iter()
                .any(|segment| segment.segment_id == shadow_active_segment_id)
            {
                let shadow_segment = InternalEventJournalSegment {
                    segment_id: shadow_active_segment_id.clone(),
                    path: shadow_active_path,
                };
                layout.segments.push(shadow_segment);
                layout.segments.sort_by(|left, right| {
                    compare_internal_event_segment_ids(&left.segment_id, &right.segment_id)
                });
            }
            if let Some(repaired_active_segment) = layout
                .segments
                .iter()
                .find(|segment| segment.segment_id == shadow_active_segment_id)
                .cloned()
            {
                layout.active_segment = repaired_active_segment;
            }
        }
    }

    let mut repaired_segments = Vec::new();
    for segment in &layout.segments {
        let should_keep_segment =
            segment.path.exists() || segment.segment_id == layout.active_segment.segment_id;
        if !should_keep_segment {
            continue;
        }
        let existing_entry = existing_state.as_ref().and_then(|state| {
            state
                .segments
                .iter()
                .find(|entry| entry.segment_id == segment.segment_id)
        });

        let repaired_status = if segment.segment_id == layout.active_segment.segment_id {
            InternalEventJournalSegmentStatus::Active
        } else if segment.segment_id == LEGACY_INTERNAL_EVENT_SEGMENT_ID {
            InternalEventJournalSegmentStatus::Legacy
        } else {
            InternalEventJournalSegmentStatus::Sealed
        };

        let created_at_ms = existing_entry.and_then(|entry| entry.created_at_ms);
        let sealed_at_ms = if repaired_status == InternalEventJournalSegmentStatus::Active {
            None
        } else {
            existing_entry.and_then(|entry| entry.sealed_at_ms)
        };

        let repaired_segment = InternalEventJournalStateSegment {
            segment_id: segment.segment_id.clone(),
            status: repaired_status,
            created_at_ms,
            sealed_at_ms,
        };
        repaired_segments.push(repaired_segment);
    }

    let repaired_state = InternalEventJournalState {
        schema_version: default_internal_event_journal_state_schema_version(),
        active_segment_id: layout.active_segment.segment_id.clone(),
        segments: repaired_segments,
    };

    store_internal_event_journal_state(&repaired_state)?;
    store_internal_event_active_segment_id_shadow(layout.active_segment.segment_id.as_str())?;

    inspect_internal_event_journal_layout()
}

pub fn rotate_internal_event_journal_segment() -> Result<String, String> {
    let legacy = legacy_internal_event_journal_segment();
    let current_active_segment_id = load_internal_event_active_segment_id()?
        .unwrap_or_else(|| DEFAULT_INTERNAL_EVENT_SEGMENT_ID.to_owned());
    if !internal_event_active_segment_id_path().exists() && legacy.path.exists() {
        let legacy_target = internal_event_segment_path(DEFAULT_INTERNAL_EVENT_SEGMENT_ID);
        prepare_internal_event_journal_parent(legacy_target.as_path())?;
        if !legacy_target.exists() {
            fs::rename(legacy.path.as_path(), legacy_target.as_path()).map_err(|error| {
                format!(
                    "migrate legacy internal event journal {} to {} failed: {error}",
                    legacy.path.display(),
                    legacy_target.display()
                )
            })?;
        }
    }
    let next_segment_id = next_internal_event_segment_id(current_active_segment_id.as_str())?;
    let mut state = load_internal_event_journal_state()?.unwrap_or_else(|| {
        bootstrap_internal_event_journal_state(current_active_segment_id.as_str())
    });
    for segment in &mut state.segments {
        if segment.segment_id == current_active_segment_id {
            segment.status = InternalEventJournalSegmentStatus::Sealed;
            if segment.sealed_at_ms.is_none() {
                segment.sealed_at_ms = Some(now_ms());
            }
        }
    }
    if !state
        .segments
        .iter()
        .any(|segment| segment.segment_id == next_segment_id)
    {
        state.segments.push(InternalEventJournalStateSegment {
            segment_id: next_segment_id.clone(),
            status: InternalEventJournalSegmentStatus::Active,
            created_at_ms: Some(now_ms()),
            sealed_at_ms: None,
        });
    }
    state.active_segment_id = next_segment_id.clone();
    store_internal_event_journal_state(&state)?;
    store_internal_event_active_segment_id_shadow(next_segment_id.as_str())?;
    Ok(next_segment_id)
}

pub fn prune_internal_event_journal_segments(
    consumed_cursor: &InternalEventJournalCursor,
) -> Result<Vec<String>, String> {
    let layout = discover_internal_event_journal_layout()?;
    let eligible = internal_event_segments_eligible_for_deletion(
        layout.segments.as_slice(),
        layout.active_segment.segment_id.as_str(),
        Some(consumed_cursor),
    );
    let mut pruned = Vec::new();
    for segment in eligible {
        if !segment.path.exists() {
            continue;
        }
        fs::remove_file(segment.path.as_path()).map_err(|error| {
            format!(
                "remove sealed internal event segment {} failed: {error}",
                segment.path.display()
            )
        })?;
        pruned.push(segment.segment_id);
    }
    if !pruned.is_empty()
        && let Some(mut state) = load_internal_event_journal_state()?
    {
        state
            .segments
            .retain(|segment| !pruned.contains(&segment.segment_id));
        store_internal_event_journal_state(&state)?;
    }
    Ok(pruned)
}

pub fn plan_internal_event_journal_gc(
    policy: &InternalEventJournalGcPolicy,
) -> Result<InternalEventJournalGcPlan, String> {
    let layout = discover_internal_event_journal_layout()?;
    let state = load_internal_event_journal_state()?;
    let retain_floor_segment_id = policy.retain_floor_segment_id.clone();

    if let Some(retain_floor_segment_id) = retain_floor_segment_id.as_deref() {
        let floor_exists = layout
            .segments
            .iter()
            .any(|segment| segment.segment_id == retain_floor_segment_id);
        if !floor_exists {
            return Err(format!(
                "internal event journal GC floor segment `{retain_floor_segment_id}` does not exist"
            ));
        }
    }

    let older_than_floor = layout
        .segments
        .iter()
        .filter(|segment| {
            if segment.segment_id == layout.active_segment.segment_id {
                return false;
            }
            let Some(retain_floor_segment_id) = retain_floor_segment_id.as_deref() else {
                return true;
            };
            compare_internal_event_segment_ids(segment.segment_id.as_str(), retain_floor_segment_id)
                .is_lt()
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut recent_retained_segment_ids = older_than_floor
        .iter()
        .rev()
        .take(policy.retain_last_sealed_segments)
        .map(|segment| segment.segment_id.clone())
        .collect::<Vec<_>>();
    recent_retained_segment_ids
        .sort_by(|left, right| compare_internal_event_segment_ids(left.as_str(), right.as_str()));

    let now_ms = now_ms();
    let decisions = layout
        .segments
        .iter()
        .map(|segment| {
            let state_entry = state.as_ref().and_then(|state| {
                state
                    .segments
                    .iter()
                    .find(|entry| entry.segment_id == segment.segment_id)
            });
            let status = state_entry
                .map(|entry| match entry.status {
                    InternalEventJournalSegmentStatus::Active => "active".to_owned(),
                    InternalEventJournalSegmentStatus::Sealed => "sealed".to_owned(),
                    InternalEventJournalSegmentStatus::Legacy => "legacy".to_owned(),
                })
                .unwrap_or_else(|| {
                    if segment.segment_id == layout.active_segment.segment_id {
                        "active".to_owned()
                    } else if segment.segment_id == LEGACY_INTERNAL_EVENT_SEGMENT_ID {
                        "legacy".to_owned()
                    } else {
                        "sealed".to_owned()
                    }
                });
            let created_at_ms = state_entry.and_then(|entry| entry.created_at_ms);
            let sealed_at_ms = state_entry.and_then(|entry| entry.sealed_at_ms);

            let action_and_reason = if segment.segment_id == layout.active_segment.segment_id {
                ("retain".to_owned(), "active_segment".to_owned())
            } else if retain_floor_segment_id
                .as_deref()
                .is_some_and(|retain_floor_segment_id| {
                    compare_internal_event_segment_ids(
                        segment.segment_id.as_str(),
                        retain_floor_segment_id,
                    )
                    .is_ge()
                })
            {
                ("retain".to_owned(), "floor_segment_or_newer".to_owned())
            } else if recent_retained_segment_ids
                .iter()
                .any(|retained_segment_id| retained_segment_id == &segment.segment_id)
            {
                ("retain".to_owned(), "retain_last_sealed".to_owned())
            } else if policy.retain_min_age_ms.is_some_and(|retain_min_age_ms| {
                let reference_ms = sealed_at_ms.or(created_at_ms).unwrap_or_default();
                if reference_ms <= 0 {
                    return false;
                }
                let age_ms = now_ms.saturating_sub(reference_ms);
                age_ms < i64::try_from(retain_min_age_ms).unwrap_or(i64::MAX)
            }) {
                ("retain".to_owned(), "retain_min_age".to_owned())
            } else {
                ("prune".to_owned(), "eligible".to_owned())
            };

            InternalEventJournalGcDecision {
                segment_id: segment.segment_id.clone(),
                path: segment.path.display().to_string(),
                status,
                created_at_ms,
                sealed_at_ms,
                action: action_and_reason.0,
                reason: action_and_reason.1,
            }
        })
        .collect::<Vec<_>>();

    Ok(InternalEventJournalGcPlan {
        active_segment_id: layout.active_segment.segment_id,
        retain_floor_segment_id,
        retain_last_sealed_segments: policy.retain_last_sealed_segments,
        retain_min_age_ms: policy.retain_min_age_ms,
        decisions,
    })
}

pub fn gc_internal_event_journal_segments(
    policy: &InternalEventJournalGcPolicy,
) -> Result<InternalEventJournalGcPlan, String> {
    let plan = plan_internal_event_journal_gc(policy)?;
    let pruned_segment_ids = plan
        .decisions
        .iter()
        .filter(|decision| decision.action == "prune")
        .map(|decision| decision.segment_id.clone())
        .collect::<Vec<_>>();

    for decision in &plan.decisions {
        if decision.action != "prune" {
            continue;
        }
        let path = PathBuf::from(decision.path.as_str());
        if !path.exists() {
            continue;
        }
        fs::remove_file(path.as_path()).map_err(|error| {
            format!(
                "remove sealed internal event segment {} failed: {error}",
                path.display()
            )
        })?;
    }

    if !pruned_segment_ids.is_empty()
        && let Some(mut state) = load_internal_event_journal_state()?
    {
        state.segments.retain(|segment| {
            !pruned_segment_ids
                .iter()
                .any(|pruned_segment_id| pruned_segment_id == &segment.segment_id)
        });
        store_internal_event_journal_state(&state)?;
    }

    Ok(plan)
}

pub fn read_internal_event_journal_since(
    after_cursor: u64,
) -> Result<Vec<InternalEventJournalRecord>, String> {
    let cursor = internal_event_journal_cursor_from_line_cursor(after_cursor)?;
    let (events, _) = read_internal_event_journal_after(cursor)?;
    Ok(events)
}

pub fn read_internal_event_journal_after(
    cursor: InternalEventJournalCursor,
) -> Result<(Vec<InternalEventJournalRecord>, InternalEventJournalCursor), String> {
    let layout = discover_internal_event_journal_layout()?;
    if layout.segments.is_empty() {
        return Ok((Vec::new(), cursor));
    }
    let (start_index, normalized_cursor) =
        resolve_internal_event_read_start(layout.segments.as_slice(), cursor);
    let mut aggregate_events = Vec::new();
    let mut next_cursor = normalized_cursor;

    for (index, segment) in layout.segments.iter().enumerate().skip(start_index) {
        let seed_cursor = if index == start_index {
            if next_cursor.segment_id.is_none() {
                InternalEventJournalCursor {
                    segment_id: Some(segment.segment_id.clone()),
                    ..next_cursor
                }
            } else {
                next_cursor.clone()
            }
        } else {
            InternalEventJournalCursor {
                segment_id: Some(segment.segment_id.clone()),
                ..InternalEventJournalCursor::default()
            }
        };
        let (segment_events, segment_cursor) =
            read_internal_event_journal_segment_after(segment, seed_cursor)?;
        aggregate_events.extend(segment_events);
        next_cursor = segment_cursor;
    }

    Ok((aggregate_events, next_cursor))
}

pub fn internal_event_journal_cursor_from_line_cursor(
    line_cursor: u64,
) -> Result<InternalEventJournalCursor, String> {
    let layout = discover_internal_event_journal_layout()?;
    let initial_segment = layout
        .segments
        .first()
        .cloned()
        .unwrap_or_else(default_internal_event_journal_segment);
    if line_cursor == 0 {
        return Ok(InternalEventJournalCursor {
            segment_id: Some(initial_segment.segment_id.clone()),
            journal_fingerprint: load_internal_event_journal_fingerprint(
                initial_segment.path.as_path(),
            )?,
            ..InternalEventJournalCursor::default()
        });
    }
    let segment = initial_segment;
    if !segment.path.exists() {
        return Ok(InternalEventJournalCursor::default());
    }
    let file = open_internal_event_journal(segment.path.as_path())?;
    lock_internal_event_journal(&file, segment.path.as_path())?;
    let read_result = (|| -> Result<_, String> {
        let mut cursor = InternalEventJournalCursor {
            segment_id: Some(segment.segment_id.clone()),
            journal_fingerprint: load_internal_event_journal_fingerprint_from_handle(
                &file,
                segment.path.as_path(),
            )?,
            ..InternalEventJournalCursor::default()
        };
        let reader_file = file.try_clone().map_err(|error| {
            format!(
                "clone internal event journal handle {} failed: {error}",
                segment.path.display()
            )
        })?;
        let mut reader_file = reader_file;
        reader_file.seek(SeekFrom::Start(0)).map_err(|error| {
            format!(
                "seek internal event journal {} to start failed: {error}",
                segment.path.display()
            )
        })?;
        let mut reader = BufReader::new(reader_file);
        let mut line = String::new();
        while cursor.line_cursor < line_cursor {
            line.clear();
            let bytes_read = reader.read_line(&mut line).map_err(|error| {
                format!(
                    "read internal event journal line {} from {} failed: {error}",
                    cursor.line_cursor.saturating_add(1),
                    segment.path.display()
                )
            })?;
            if bytes_read == 0 {
                break;
            }
            cursor.line_cursor = cursor.line_cursor.saturating_add(1);
            cursor.byte_offset = cursor.byte_offset.saturating_add(
                u64::try_from(bytes_read)
                    .map_err(|error| format!("internal event cursor overflowed u64: {error}"))?,
            );
        }
        Ok(cursor)
    })();
    let unlock_result = unlock_internal_event_journal(&file, segment.path.as_path());
    let cursor = read_result?;
    unlock_result?;
    Ok(cursor)
}

fn append_internal_event_journal_record(event_name: &str, payload: &Value) -> Result<(), String> {
    let control_lock_path = internal_event_journal_control_lock_path();
    let control_lock = open_internal_event_journal_control_lock(control_lock_path.as_path())?;
    lock_internal_event_journal_control_lock(&control_lock, control_lock_path.as_path())?;
    let append_result = (|| -> Result<(), String> {
        maybe_rotate_internal_event_journal_segment_for_size()?;
        let path = current_internal_event_journal_segment_path();
        let recorded_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis() as i64)
            .unwrap_or_default();
        let encoded = serde_json::to_string(&json!({
            "event_name": event_name,
            "payload": payload,
            "recorded_at_ms": recorded_at_ms,
        }))
        .map_err(|error| format!("serialize internal event journal record failed: {error}"))?;
        let mut file = open_internal_event_journal(path.as_path())?;
        lock_internal_event_journal(&file, path.as_path())?;
        let append_result = (|| -> Result<(), String> {
            file.write_all(encoded.as_bytes()).map_err(|error| {
                format!(
                    "append internal event journal {} failed: {error}",
                    path.display()
                )
            })?;
            file.write_all(b"\n").map_err(|error| {
                format!(
                    "append newline to internal event journal {} failed: {error}",
                    path.display()
                )
            })?;
            file.flush().map_err(|error| {
                format!(
                    "flush internal event journal {} failed: {error}",
                    path.display()
                )
            })?;
            Ok(())
        })();
        let unlock_result = unlock_internal_event_journal(&file, path.as_path());
        append_result?;
        unlock_result
    })();
    let unlock_result =
        unlock_internal_event_journal_control_lock(&control_lock, control_lock_path.as_path());
    append_result?;
    unlock_result
}

fn prepare_internal_event_journal_parent(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create internal event journal directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    Ok(())
}

fn open_internal_event_journal(path: &std::path::Path) -> Result<File, String> {
    prepare_internal_event_journal_parent(path)?;
    OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "open internal event journal {} failed: {error}",
                path.display()
            )
        })
}

fn open_internal_event_journal_control_lock(path: &std::path::Path) -> Result<File, String> {
    prepare_internal_event_journal_parent(path)?;
    OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "open internal event journal control lock {} failed: {error}",
                path.display()
            )
        })
}

pub fn internal_event_segments_dir() -> PathBuf {
    crate::config::default_loong_home()
        .join("automation")
        .join("internal-events")
}

pub fn internal_event_active_segment_id_path() -> PathBuf {
    crate::config::default_loong_home()
        .join("automation")
        .join("internal-events.active")
}

pub fn internal_event_journal_state_path() -> PathBuf {
    crate::config::default_loong_home()
        .join("automation")
        .join("internal-events.state.json")
}

pub fn internal_event_segment_path(segment_id: &str) -> PathBuf {
    internal_event_segments_dir().join(format!("{segment_id}.jsonl"))
}

fn internal_event_journal_control_lock_path() -> PathBuf {
    crate::config::default_loong_home()
        .join("automation")
        .join("internal-events.control.lock")
}

fn lock_internal_event_journal(file: &File, path: &std::path::Path) -> Result<(), String> {
    file.lock().map_err(|error| {
        format!(
            "lock internal event journal {} failed: {error}",
            path.display()
        )
    })
}

fn lock_internal_event_journal_control_lock(
    file: &File,
    path: &std::path::Path,
) -> Result<(), String> {
    file.lock().map_err(|error| {
        format!(
            "lock internal event journal control lock {} failed: {error}",
            path.display()
        )
    })
}

fn default_internal_event_journal_segment() -> InternalEventJournalSegment {
    InternalEventJournalSegment {
        segment_id: DEFAULT_INTERNAL_EVENT_SEGMENT_ID.to_owned(),
        path: internal_event_segment_path(DEFAULT_INTERNAL_EVENT_SEGMENT_ID),
    }
}

fn legacy_internal_event_journal_segment() -> InternalEventJournalSegment {
    InternalEventJournalSegment {
        segment_id: LEGACY_INTERNAL_EVENT_SEGMENT_ID.to_owned(),
        path: crate::config::default_loong_home()
            .join("automation")
            .join("internal-events.jsonl"),
    }
}

fn current_internal_event_journal_segment_path() -> PathBuf {
    discover_internal_event_journal_layout()
        .ok()
        .map(|layout| layout.active_segment.path)
        .unwrap_or_else(|| default_internal_event_journal_segment().path)
}

fn load_internal_event_active_segment_id() -> Result<Option<String>, String> {
    if let Some(state) = load_internal_event_journal_state()? {
        return Ok(Some(state.active_segment_id));
    }
    load_internal_event_active_segment_shadow_id()
}

fn load_internal_event_active_segment_shadow_id() -> Result<Option<String>, String> {
    let path = internal_event_active_segment_id_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path.as_path()).map_err(|error| {
        format!(
            "read internal event active segment {} failed: {error}",
            path.display()
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_owned()))
}

fn store_internal_event_active_segment_id_shadow(segment_id: &str) -> Result<(), String> {
    let path = internal_event_active_segment_id_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create internal event active segment directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let tmp_path = path.with_extension("active.tmp");
    fs::write(&tmp_path, format!("{segment_id}\n")).map_err(|error| {
        format!(
            "write internal event active segment temp file {} failed: {error}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, &path).map_err(|error| {
        format!(
            "publish internal event active segment {} from {} failed: {error}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn load_internal_event_journal_state() -> Result<Option<InternalEventJournalState>, String> {
    let path = internal_event_journal_state_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path.as_path()).map_err(|error| {
        format!(
            "read internal event journal state {} failed: {error}",
            path.display()
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Ok(state) = serde_json::from_str::<InternalEventJournalState>(trimmed) {
        return Ok(Some(state));
    }
    let legacy_state = serde_json::from_str::<serde_json::Value>(trimmed).map_err(|error| {
        format!(
            "parse internal event journal state {} failed: {error}",
            path.display()
        )
    })?;
    let Some(active_segment_id) = legacy_state
        .get("active_segment_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Err(format!(
            "parse internal event journal state {} failed: missing string active_segment_id",
            path.display()
        ));
    };
    Ok(Some(bootstrap_internal_event_journal_state(
        active_segment_id,
    )))
}

fn store_internal_event_journal_state(state: &InternalEventJournalState) -> Result<(), String> {
    let path = internal_event_journal_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create internal event journal state directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(state)
        .map_err(|error| format!("serialize internal event journal state failed: {error}"))?;
    let tmp_path = path.with_extension("state.tmp");
    fs::write(&tmp_path, format!("{encoded}\n")).map_err(|error| {
        format!(
            "write internal event journal temp state {} failed: {error}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, &path).map_err(|error| {
        format!(
            "publish internal event journal state {} from {} failed: {error}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn default_internal_event_journal_state_schema_version() -> u32 {
    1
}

fn next_internal_event_segment_id(current: &str) -> Result<String, String> {
    let Some(raw_suffix) = current.strip_prefix("segment-") else {
        return Err(format!(
            "current internal event segment id `{current}` does not use `segment-` numeric format"
        ));
    };
    let parsed = raw_suffix.parse::<u64>().map_err(|error| {
        format!("parse internal event segment suffix `{raw_suffix}` failed: {error}")
    })?;
    let next = parsed.saturating_add(1);
    Ok(format!("segment-{next:06}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

fn internal_event_segment_max_bytes() -> u64 {
    let value = std::env::var("LOONG_INTERNAL_EVENT_SEGMENT_MAX_BYTES")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0);
    value.unwrap_or(DEFAULT_INTERNAL_EVENT_SEGMENT_MAX_BYTES)
}

fn maybe_rotate_internal_event_journal_segment_for_size() -> Result<(), String> {
    let threshold_bytes = internal_event_segment_max_bytes();
    let current_path = current_internal_event_journal_segment_path();
    let current_len = if current_path.exists() {
        fs::metadata(current_path.as_path())
            .map_err(|error| {
                format!(
                    "read internal event journal metadata {} failed: {error}",
                    current_path.display()
                )
            })?
            .len()
    } else {
        0
    };
    if current_len < threshold_bytes {
        return Ok(());
    }
    let _ = rotate_internal_event_journal_segment()?;
    Ok(())
}

fn bootstrap_internal_event_journal_state(active_segment_id: &str) -> InternalEventJournalState {
    let mut segments = Vec::new();
    let legacy = legacy_internal_event_journal_segment();
    if legacy.path.exists() && active_segment_id != LEGACY_INTERNAL_EVENT_SEGMENT_ID {
        segments.push(InternalEventJournalStateSegment {
            segment_id: LEGACY_INTERNAL_EVENT_SEGMENT_ID.to_owned(),
            status: InternalEventJournalSegmentStatus::Legacy,
            created_at_ms: None,
            sealed_at_ms: None,
        });
    }
    if !segments
        .iter()
        .any(|segment| segment.segment_id == active_segment_id)
    {
        segments.push(InternalEventJournalStateSegment {
            segment_id: active_segment_id.to_owned(),
            status: InternalEventJournalSegmentStatus::Active,
            created_at_ms: Some(now_ms()),
            sealed_at_ms: None,
        });
    }
    InternalEventJournalState {
        schema_version: default_internal_event_journal_state_schema_version(),
        active_segment_id: active_segment_id.to_owned(),
        segments,
    }
}

fn discover_internal_event_journal_layout() -> Result<InternalEventJournalLayout, String> {
    let manifest_state = load_internal_event_journal_state()?;
    let mut segments = Vec::new();
    let legacy = legacy_internal_event_journal_segment();
    if legacy.path.exists() {
        segments.push(legacy);
    }

    if let Some(state) = manifest_state.as_ref() {
        for entry in &state.segments {
            let path = if entry.segment_id == LEGACY_INTERNAL_EVENT_SEGMENT_ID {
                legacy_internal_event_journal_segment().path
            } else {
                internal_event_segment_path(entry.segment_id.as_str())
            };
            if segments
                .iter()
                .any(|segment| segment.segment_id == entry.segment_id)
            {
                continue;
            }
            segments.push(InternalEventJournalSegment {
                segment_id: entry.segment_id.clone(),
                path,
            });
        }
    }

    let segments_dir = internal_event_segments_dir();
    if segments_dir.exists() {
        let entries = fs::read_dir(segments_dir.as_path()).map_err(|error| {
            format!(
                "read internal event segments directory {} failed: {error}",
                segments_dir.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "read internal event segment entry under {} failed: {error}",
                    segments_dir.display()
                )
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if segments.iter().any(|segment| segment.segment_id == stem) {
                continue;
            }
            segments.push(InternalEventJournalSegment {
                segment_id: stem.to_owned(),
                path,
            });
        }
    }

    let active_segment = if let Some(active_segment_id) = manifest_state
        .as_ref()
        .map(|state| state.active_segment_id.clone())
    {
        InternalEventJournalSegment {
            segment_id: active_segment_id.clone(),
            path: internal_event_segment_path(active_segment_id.as_str()),
        }
    } else if let Some(active_segment_id) = load_internal_event_active_segment_id()? {
        InternalEventJournalSegment {
            segment_id: active_segment_id.clone(),
            path: internal_event_segment_path(active_segment_id.as_str()),
        }
    } else if let Some(existing_segment) = segments
        .iter()
        .filter(|segment| segment.segment_id != LEGACY_INTERNAL_EVENT_SEGMENT_ID)
        .max_by(|left, right| {
            compare_internal_event_segment_ids(&left.segment_id, &right.segment_id)
        })
        .cloned()
    {
        existing_segment
    } else if let Some(legacy_segment) = segments
        .iter()
        .find(|segment| segment.segment_id == LEGACY_INTERNAL_EVENT_SEGMENT_ID)
        .cloned()
    {
        legacy_segment
    } else {
        default_internal_event_journal_segment()
    };

    if !segments
        .iter()
        .any(|segment| segment.segment_id == active_segment.segment_id)
    {
        segments.push(active_segment.clone());
    }
    segments.sort_by(|left, right| {
        compare_internal_event_segment_ids(&left.segment_id, &right.segment_id)
    });

    Ok(InternalEventJournalLayout {
        segments,
        active_segment,
    })
}

fn resolve_internal_event_read_start(
    segments: &[InternalEventJournalSegment],
    cursor: InternalEventJournalCursor,
) -> (usize, InternalEventJournalCursor) {
    if let Some(segment_id) = cursor.segment_id.as_deref()
        && let Some(index) = segments
            .iter()
            .position(|segment| segment.segment_id == segment_id)
    {
        return (index, cursor);
    }
    if let Some(stale_segment_id) = cursor.segment_id.as_deref()
        && let Some(index) = segments.iter().position(|segment| {
            compare_internal_event_segment_ids(segment.segment_id.as_str(), stale_segment_id)
                .is_ge()
        })
    {
        let Some(normalized_segment) = segments.get(index) else {
            return (0, InternalEventJournalCursor::default());
        };
        return (
            index,
            InternalEventJournalCursor {
                segment_id: Some(normalized_segment.segment_id.clone()),
                ..InternalEventJournalCursor::default()
            },
        );
    }
    let Some(first_segment) = segments.first() else {
        return (0, cursor);
    };
    (
        0,
        InternalEventJournalCursor {
            segment_id: Some(first_segment.segment_id.clone()),
            ..InternalEventJournalCursor::default()
        },
    )
}

fn internal_event_segments_eligible_for_deletion(
    segments: &[InternalEventJournalSegment],
    active_segment_id: &str,
    floor_cursor: Option<&InternalEventJournalCursor>,
) -> Vec<InternalEventJournalSegment> {
    let Some(floor_segment_id) = floor_cursor.and_then(|cursor| cursor.segment_id.as_deref())
    else {
        return Vec::new();
    };
    if !segments
        .iter()
        .any(|segment| segment.segment_id == floor_segment_id)
    {
        return Vec::new();
    }
    segments
        .iter()
        .filter(|segment| {
            segment.segment_id != active_segment_id
                && compare_internal_event_segment_ids(segment.segment_id.as_str(), floor_segment_id)
                    .is_lt()
        })
        .cloned()
        .collect()
}

fn compare_internal_event_segment_ids(left: &str, right: &str) -> std::cmp::Ordering {
    match (
        parse_internal_event_segment_sequence(left),
        parse_internal_event_segment_sequence(right),
    ) {
        (Some(left_sequence), Some(right_sequence)) => left_sequence.cmp(&right_sequence),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => left.cmp(right),
    }
}

fn parse_internal_event_segment_sequence(segment_id: &str) -> Option<u64> {
    segment_id
        .strip_prefix("segment-")
        .and_then(|suffix| suffix.parse::<u64>().ok())
}

fn read_internal_event_journal_segment_after(
    segment: &InternalEventJournalSegment,
    cursor: InternalEventJournalCursor,
) -> Result<(Vec<InternalEventJournalRecord>, InternalEventJournalCursor), String> {
    let path = segment.path.as_path();
    if !path.exists() {
        return Ok((
            Vec::new(),
            InternalEventJournalCursor {
                segment_id: Some(segment.segment_id.clone()),
                ..cursor
            },
        ));
    }
    let file = open_internal_event_journal(path)?;
    lock_internal_event_journal(&file, path)?;
    let read_result = (|| -> Result<_, String> {
        let metadata = file.metadata().map_err(|error| {
            format!(
                "read internal event journal metadata {} failed: {error}",
                path.display()
            )
        })?;
        let current_fingerprint = load_internal_event_journal_fingerprint_from_handle(&file, path)?;
        let mut cursor = if cursor.byte_offset > metadata.len()
            || cursor.journal_fingerprint != current_fingerprint
        {
            InternalEventJournalCursor {
                segment_id: Some(segment.segment_id.clone()),
                ..InternalEventJournalCursor::default()
            }
        } else {
            cursor
        };
        cursor.segment_id = Some(segment.segment_id.clone());
        cursor.journal_fingerprint = current_fingerprint;

        let mut reader_file = file.try_clone().map_err(|error| {
            format!(
                "clone internal event journal handle {} failed: {error}",
                path.display()
            )
        })?;
        reader_file
            .seek(SeekFrom::Start(cursor.byte_offset))
            .map_err(|error| {
                format!(
                    "seek internal event journal {} to {} failed: {error}",
                    path.display(),
                    cursor.byte_offset
                )
            })?;
        let mut reader = BufReader::new(reader_file);
        let mut events = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).map_err(|error| {
                format!(
                    "read internal event journal line {} from {} failed: {error}",
                    cursor.line_cursor.saturating_add(1),
                    path.display()
                )
            })?;
            if bytes_read == 0 {
                break;
            }
            cursor.line_cursor = cursor.line_cursor.saturating_add(1);
            cursor.byte_offset = cursor.byte_offset.saturating_add(
                u64::try_from(bytes_read)
                    .map_err(|error| format!("internal event cursor overflowed u64: {error}"))?,
            );
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line).map_err(|error| {
                format!(
                    "parse internal event journal line {} from {} failed: {error}",
                    cursor.line_cursor,
                    path.display()
                )
            })?;
            let event_name = value
                .get("event_name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "internal event journal line {} in {} is missing string event_name",
                        cursor.line_cursor,
                        path.display()
                    )
                })?
                .to_owned();
            let payload = value.get("payload").cloned().unwrap_or(Value::Null);
            let recorded_at_ms = value
                .get("recorded_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            events.push(InternalEventJournalRecord {
                line_cursor: cursor.line_cursor,
                event_name,
                payload,
                recorded_at_ms,
            });
        }
        Ok((events, cursor))
    })();
    let unlock_result = unlock_internal_event_journal(&file, path);
    let output = read_result?;
    unlock_result?;
    Ok(output)
}

fn unlock_internal_event_journal(file: &File, path: &std::path::Path) -> Result<(), String> {
    file.unlock().map_err(|error| {
        format!(
            "unlock internal event journal {} failed: {error}",
            path.display()
        )
    })
}

fn unlock_internal_event_journal_control_lock(
    file: &File,
    path: &std::path::Path,
) -> Result<(), String> {
    file.unlock().map_err(|error| {
        format!(
            "unlock internal event journal control lock {} failed: {error}",
            path.display()
        )
    })
}

fn load_internal_event_journal_fingerprint(
    path: &std::path::Path,
) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let file = fs::File::open(path).map_err(|error| {
        format!(
            "open internal event journal {} failed: {error}",
            path.display()
        )
    })?;
    load_internal_event_journal_fingerprint_from_handle(&file, path)
}

fn load_internal_event_journal_fingerprint_from_handle(
    file: &File,
    path: &std::path::Path,
) -> Result<Option<String>, String> {
    let cloned = file.try_clone().map_err(|error| {
        format!(
            "clone internal event journal handle {} failed: {error}",
            path.display()
        )
    })?;
    let mut cloned = cloned;
    cloned.seek(SeekFrom::Start(0)).map_err(|error| {
        format!(
            "seek internal event journal {} to start failed: {error}",
            path.display()
        )
    })?;
    let reader = BufReader::new(cloned);
    for line in reader.lines() {
        let line = line.map_err(|error| {
            format!(
                "read internal event journal {} for fingerprint failed: {error}",
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let digest = Sha256::digest(line.as_bytes());
        return Ok(Some(hex::encode(digest)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedLoongHome;
    use std::sync::Mutex;

    #[test]
    fn emit_internal_event_with_metadata_wraps_object_payload() {
        let captured = Arc::new(Mutex::new(None::<Value>));
        let sink_capture = captured.clone();
        install_internal_event_sink(Arc::new(move |_event_name, payload| {
            *sink_capture.lock().expect("capture lock") = Some(payload);
        }));

        emit_internal_event_with_metadata(
            "session.cancelled",
            "app.tools.session",
            json!({
                "session_id": "abc"
            }),
        );

        let payload = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("captured payload");
        assert_eq!(payload["session_id"], "abc");
        assert_eq!(payload["_automation"]["event_name"], "session.cancelled");
        assert_eq!(
            payload["_automation"]["source_surface"],
            "app.tools.session"
        );
    }

    #[test]
    fn emit_internal_event_writes_and_reads_journal_records() {
        let _home = ScopedLoongHome::new("internal-events-journal-records");
        emit_internal_event_with_metadata(
            "session.cancelled",
            "app.tools.session",
            json!({
                "session_id": "abc"
            }),
        );

        let events = read_internal_event_journal_since(0).expect("read internal event journal");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].line_cursor, 1);
        assert_eq!(events[0].event_name, "session.cancelled");
        assert_eq!(events[0].payload["session_id"], "abc");
        assert_eq!(
            events[0].payload["_automation"]["source_surface"],
            "app.tools.session"
        );
    }
}
