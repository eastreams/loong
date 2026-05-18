use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactDurabilityClass {
    DurableTruth,
    DiscardableCache,
    DerivedProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalState {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionArtifactKind {
    AssistantTextOutput {
        text: String,
    },
    ToolInvocationEvent {
        tool_name: String,
        invocation_summary: String,
    },
    PatchEditRecord {
        paths: Vec<PathBuf>,
        summary: String,
    },
    GeneratedArtifactReference {
        location: PathBuf,
        description: String,
    },
    DiagnosticFailureRecord {
        severity: DiagnosticSeverity,
        code: String,
        message: String,
    },
    ApprovalCheckpoint {
        gate: String,
        state: ApprovalState,
        actor: Option<String>,
    },
}

impl ExecutionArtifactKind {
    pub const fn default_durability(&self) -> ArtifactDurabilityClass {
        match self {
            Self::AssistantTextOutput { .. } => ArtifactDurabilityClass::DerivedProjection,
            Self::ToolInvocationEvent { .. }
            | Self::PatchEditRecord { .. }
            | Self::GeneratedArtifactReference { .. }
            | Self::DiagnosticFailureRecord { .. }
            | Self::ApprovalCheckpoint { .. } => ArtifactDurabilityClass::DurableTruth,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionArtifact {
    pub artifact_id: String,
    pub kind: ExecutionArtifactKind,
    pub durability: ArtifactDurabilityClass,
}

impl ExecutionArtifact {
    pub fn new(artifact_id: impl Into<String>, kind: ExecutionArtifactKind) -> Self {
        let durability = kind.default_durability();
        Self {
            artifact_id: artifact_id.into(),
            kind,
            durability,
        }
    }

    pub fn with_durability(
        artifact_id: impl Into<String>,
        kind: ExecutionArtifactKind,
        durability: ArtifactDurabilityClass,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            kind,
            durability,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExecutionArtifacts {
    records: Vec<ExecutionArtifact>,
}

impl ExecutionArtifacts {
    pub fn record(&mut self, artifact: ExecutionArtifact) {
        self.records.push(artifact);
    }

    pub fn record_bounded(&mut self, artifact: ExecutionArtifact, max_records: Option<usize>) {
        self.records.push(artifact);
        self.enforce_max_records(max_records);
    }

    pub fn iter(&self) -> impl Iterator<Item = &ExecutionArtifact> {
        self.records.iter()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn durable_truth(&self) -> impl Iterator<Item = &ExecutionArtifact> {
        self.records
            .iter()
            .filter(|artifact| artifact.durability == ArtifactDurabilityClass::DurableTruth)
    }

    pub fn discardable_cache(&self) -> impl Iterator<Item = &ExecutionArtifact> {
        self.records
            .iter()
            .filter(|artifact| artifact.durability == ArtifactDurabilityClass::DiscardableCache)
    }

    pub fn derived_projection(&self) -> impl Iterator<Item = &ExecutionArtifact> {
        self.records
            .iter()
            .filter(|artifact| artifact.durability == ArtifactDurabilityClass::DerivedProjection)
    }

    pub fn enforce_max_records(&mut self, max_records: Option<usize>) {
        let Some(max_records) = max_records else {
            return;
        };
        if self.records.len() <= max_records {
            return;
        }
        let drop_count = self.records.len() - max_records;
        self.records.drain(0..drop_count);
    }
}
