use super::turn_engine::ToolResultEnvelope;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultLine {
    status_marker: String,
    envelope: ToolResultEnvelope,
}

impl ToolResultLine {
    pub fn new(status_marker: impl Into<String>, envelope: ToolResultEnvelope) -> Self {
        Self {
            status_marker: status_marker.into(),
            envelope,
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        let (status_prefix, payload) = trimmed.split_once(' ')?;
        let status_marker = status_prefix.strip_prefix('[')?.strip_suffix(']')?.trim();
        if status_marker.is_empty() {
            return None;
        }
        let envelope = serde_json::from_str::<ToolResultEnvelope>(payload).ok()?;
        Some(Self::new(status_marker, envelope))
    }

    pub fn render(&self) -> Option<String> {
        let payload = serde_json::to_string(&self.envelope).ok()?;
        Some(format!("[{}] {payload}", self.status_marker))
    }

    pub fn tool_name(&self) -> &str {
        self.envelope.tool.as_str()
    }

    pub fn set_tool_name(&mut self, tool_name: impl Into<String>) {
        self.envelope.tool = tool_name.into();
    }

    pub fn payload_truncated(&self) -> bool {
        self.envelope.payload_truncated
    }

    pub fn set_payload_truncated(&mut self, truncated: bool) {
        self.envelope.payload_truncated = truncated;
    }

    pub fn payload_summary_str(&self) -> &str {
        self.envelope.payload_summary.as_str()
    }

    pub fn payload_summary_json(&self) -> Option<Value> {
        serde_json::from_str(self.envelope.payload_summary.as_str()).ok()
    }

    pub fn replace_payload_summary_str(&mut self, payload_summary: String) {
        self.envelope.payload_summary = payload_summary;
    }

    pub fn envelope(&self) -> &ToolResultEnvelope {
        &self.envelope
    }
}
