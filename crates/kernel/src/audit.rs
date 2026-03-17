use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
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
    write_lock: Mutex<()>,
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

        OpenOptions::new()
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
            write_lock: Mutex::new(()),
        })
    }
}

impl AuditSink for JsonlAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_error| AuditError::Sink("audit write mutex poisoned".to_owned()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                AuditError::Sink(format!(
                    "failed to open audit journal `{}` for append: {error}",
                    self.path.display()
                ))
            })?;
        let json_line = serde_json::to_string(&event).map_err(|error| {
            AuditError::Sink(format!("failed to serialize audit event: {error}"))
        })?;
        file.write_all(json_line.as_bytes()).map_err(|error| {
            AuditError::Sink(format!(
                "failed to write audit journal `{}`: {error}",
                self.path.display()
            ))
        })?;
        file.write_all(b"\n").map_err(|error| {
            AuditError::Sink(format!(
                "failed to finalize audit journal `{}`: {error}",
                self.path.display()
            ))
        })?;
        Ok(())
    }
}

pub struct FanoutAuditSink {
    children: Vec<Arc<dyn AuditSink>>,
}

impl FanoutAuditSink {
    #[must_use]
    pub fn new(children: Vec<Arc<dyn AuditSink>>) -> Self {
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
