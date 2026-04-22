use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type InternalEventSink = Arc<dyn Fn(&str, Value) + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalEventJournalRecord {
    pub line_cursor: u64,
    pub event_name: String,
    pub payload: Value,
    pub recorded_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct InternalEventJournalCursor {
    pub line_cursor: u64,
    pub byte_offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_fingerprint: Option<String>,
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
    crate::config::default_loong_home()
        .join("automation")
        .join("internal-events.jsonl")
}

pub fn probe_internal_event_journal_runtime_ready() -> Result<(), String> {
    let path = internal_event_journal_path();
    let journal = open_internal_event_journal(path.as_path())?;
    lock_internal_event_journal(&journal, path.as_path())?;
    unlock_internal_event_journal(&journal, path.as_path())
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
    let path = internal_event_journal_path();
    if !path.exists() {
        return Ok((Vec::new(), cursor));
    }
    let current_fingerprint = load_internal_event_journal_fingerprint(path.as_path())?;
    let mut file = fs::File::open(path.as_path()).map_err(|error| {
        format!(
            "open internal event journal {} failed: {error}",
            path.display()
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "read internal event journal metadata {} failed: {error}",
            path.display()
        )
    })?;
    let mut cursor = if cursor.byte_offset > metadata.len()
        || cursor.journal_fingerprint != current_fingerprint
    {
        InternalEventJournalCursor::default()
    } else {
        cursor
    };
    cursor.journal_fingerprint = current_fingerprint;
    file.seek(SeekFrom::Start(cursor.byte_offset))
        .map_err(|error| {
            format!(
                "seek internal event journal {} to {} failed: {error}",
                path.display(),
                cursor.byte_offset
            )
        })?;
    let mut reader = BufReader::new(file);
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
}

pub fn internal_event_journal_cursor_from_line_cursor(
    line_cursor: u64,
) -> Result<InternalEventJournalCursor, String> {
    if line_cursor == 0 {
        return Ok(InternalEventJournalCursor {
            journal_fingerprint: load_internal_event_journal_fingerprint(
                internal_event_journal_path().as_path(),
            )?,
            ..InternalEventJournalCursor::default()
        });
    }
    let path = internal_event_journal_path();
    if !path.exists() {
        return Ok(InternalEventJournalCursor::default());
    }
    let file = fs::File::open(path.as_path()).map_err(|error| {
        format!(
            "open internal event journal {} failed: {error}",
            path.display()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut cursor = InternalEventJournalCursor::default();
    let mut line = String::new();
    while cursor.line_cursor < line_cursor {
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
    }
    cursor.journal_fingerprint = load_internal_event_journal_fingerprint(path.as_path())?;
    Ok(cursor)
}

fn append_internal_event_journal_record(event_name: &str, payload: &Value) -> Result<(), String> {
    let path = internal_event_journal_path();
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

fn lock_internal_event_journal(file: &File, path: &std::path::Path) -> Result<(), String> {
    file.lock().map_err(|error| {
        format!(
            "lock internal event journal {} failed: {error}",
            path.display()
        )
    })
}

fn unlock_internal_event_journal(file: &File, path: &std::path::Path) -> Result<(), String> {
    file.unlock().map_err(|error| {
        format!(
            "unlock internal event journal {} failed: {error}",
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
    let reader = BufReader::new(file);
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
