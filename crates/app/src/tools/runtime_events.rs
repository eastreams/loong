use std::future::Future;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRuntimeStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFileChangeKind {
    Create,
    Overwrite,
    Edit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputDelta {
    pub stream: ToolRuntimeStream,
    pub chunk: String,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFileChangePreview {
    pub path: String,
    pub kind: ToolFileChangeKind,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommandMetrics {
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRuntimeEvent {
    OutputDelta(ToolOutputDelta),
    FileChangePreview(ToolFileChangePreview),
    CommandMetrics(ToolCommandMetrics),
}

pub trait ToolRuntimeEventSink: Send + Sync {
    fn emit(&self, event: ToolRuntimeEvent);
}

tokio::task_local! {
    static TOOL_RUNTIME_EVENT_SINK_TASK: Arc<dyn ToolRuntimeEventSink>;
}

pub(crate) async fn with_tool_runtime_event_sink<T>(
    sink: Arc<dyn ToolRuntimeEventSink>,
    future: impl Future<Output = T>,
) -> T {
    TOOL_RUNTIME_EVENT_SINK_TASK.scope(sink, future).await
}

pub(crate) fn current_tool_runtime_event_sink() -> Option<Arc<dyn ToolRuntimeEventSink>> {
    let sink = TOOL_RUNTIME_EVENT_SINK_TASK.try_with(Arc::clone);
    sink.ok()
}
