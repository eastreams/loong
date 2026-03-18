use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// Re-export data types from contracts
pub use loongclaw_contracts::{AuditEvent, AuditEventKind, ExecutionPlane, PlaneTier};

use crate::errors::AuditError;

pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError>;
}

#[derive(Debug, Default)]
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn record(&self, _event: AuditEvent) -> Result<(), AuditError> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryAuditSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

impl InMemoryAuditSink {
    #[must_use]
    pub fn snapshot(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .map_or_else(|_| Vec::new(), |guard| guard.clone())
    }
}

impl AuditSink for InMemoryAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let mut guard = self
            .events
            .lock()
            .map_err(|_err| AuditError::Sink("audit mutex poisoned".to_owned()))?;
        guard.push(event);
        Ok(())
    }
}

#[derive(Debug)]
pub struct JsonlAuditSink {
    path: PathBuf,
    journal: Mutex<File>,
}

impl JsonlAuditSink {
    pub fn new(path: PathBuf) -> Result<Self, AuditError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                AuditError::Sink(format!(
                    "failed to prepare audit journal parent directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }

        let journal = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| {
                AuditError::Sink(format!(
                    "failed to open audit journal `{}`: {error}",
                    path.display()
                ))
            })?;

        Ok(Self {
            path,
            journal: Mutex::new(journal),
        })
    }
}

fn serialize_audit_event_line(
    event: &AuditEvent,
    journal_path: &Path,
) -> Result<Vec<u8>, AuditError> {
    let mut encoded = serde_json::to_vec(event).map_err(|error| {
        AuditError::Sink(format!(
            "failed to serialize audit event for `{}`: {error}",
            journal_path.display()
        ))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

impl AuditSink for JsonlAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let mut guard = self
            .journal
            .lock()
            .map_err(|_error| AuditError::Sink("audit journal mutex poisoned".to_owned()))?;
        let encoded = serialize_audit_event_line(&event, &self.path)?;

        guard.lock().map_err(|error| {
            AuditError::Sink(format!(
                "failed to lock audit journal `{}`: {error}",
                self.path.display()
            ))
        })?;

        let write_result = guard
            .write_all(&encoded)
            .map_err(|error| {
                AuditError::Sink(format!(
                    "failed to append audit event to `{}`: {error}",
                    self.path.display()
                ))
            })
            .and_then(|()| {
                guard.flush().map_err(|error| {
                    AuditError::Sink(format!(
                        "failed to flush audit journal `{}`: {error}",
                        self.path.display()
                    ))
                })
            });

        let unlock_result = guard.unlock().map_err(|error| {
            AuditError::Sink(format!(
                "failed to unlock audit journal `{}`: {error}",
                self.path.display()
            ))
        });

        match (write_result, unlock_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

pub struct FanoutAuditSink {
    children: Vec<Arc<dyn AuditSink>>,
}

impl FanoutAuditSink {
    #[must_use]
    pub fn new(children: Vec<Arc<dyn AuditSink>>) -> Self {
        assert!(
            !children.is_empty(),
            "fanout audit sink requires at least one child"
        );
        Self { children }
    }
}

impl AuditSink for FanoutAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        if let Some((last, rest)) = self.children.split_last() {
            for sink in rest {
                sink.record(event.clone())?;
            }
            last.record(event)?;
        }
        Ok(())
    }
}
