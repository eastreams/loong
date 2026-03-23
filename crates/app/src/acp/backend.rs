use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use crate::CliResult;
use crate::config::LoongClawConfig;

use super::binding::AcpSessionBindingScope;

pub const ACP_RUNTIME_API_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AcpCapability {
    SessionLifecycle,
    TurnExecution,
    TurnEventStreaming,
    Cancellation,
    StatusInspection,
    ModeSwitching,
    ConfigPatching,
    Doctor,
    PersistentBindings,
    McpServerInjection,
}

impl AcpCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionLifecycle => "session_lifecycle",
            Self::TurnExecution => "turn_execution",
            Self::TurnEventStreaming => "turn_event_streaming",
            Self::Cancellation => "cancellation",
            Self::StatusInspection => "status_inspection",
            Self::ModeSwitching => "mode_switching",
            Self::ConfigPatching => "config_patching",
            Self::Doctor => "doctor",
            Self::PersistentBindings => "persistent_bindings",
            Self::McpServerInjection => "mcp_server_injection",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpBackendMetadata {
    pub id: &'static str,
    pub api_version: u16,
    pub capabilities: BTreeSet<AcpCapability>,
    pub summary: &'static str,
}

impl AcpBackendMetadata {
    pub fn new(
        id: &'static str,
        capabilities: impl IntoIterator<Item = AcpCapability>,
        summary: &'static str,
    ) -> Self {
        Self {
            id,
            api_version: ACP_RUNTIME_API_VERSION,
            capabilities: capabilities.into_iter().collect(),
            summary,
        }
    }

    pub fn capability_names(&self) -> Vec<&'static str> {
        self.capabilities
            .iter()
            .copied()
            .map(AcpCapability::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpSessionState {
    Initializing,
    Ready,
    Busy,
    Cancelling,
    Error,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpSessionMode {
    Interactive,
    Background,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpRoutingOrigin {
    ExplicitRequest,
    AutomaticAgentPrefixed,
    AutomaticDispatch,
}

impl AcpRoutingOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitRequest => "explicit_request",
            Self::AutomaticAgentPrefixed => "automatic_agent_prefixed",
            Self::AutomaticDispatch => "automatic_dispatch",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "explicit_request" => Some(Self::ExplicitRequest),
            "automatic_agent_prefixed" => Some(Self::AutomaticAgentPrefixed),
            "automatic_dispatch" => Some(Self::AutomaticDispatch),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpRoutingIntent {
    #[default]
    Automatic,
    Explicit,
}

impl AcpRoutingIntent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Explicit => "explicit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionHandle {
    pub session_key: String,
    pub backend_id: String,
    pub runtime_session_name: String,
    pub working_directory: Option<PathBuf>,
    pub backend_session_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub binding: Option<AcpSessionBindingScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionMetadata {
    pub session_key: String,
    pub conversation_id: Option<String>,
    pub binding: Option<AcpSessionBindingScope>,
    pub activation_origin: Option<AcpRoutingOrigin>,
    pub backend_id: String,
    pub runtime_session_name: String,
    pub working_directory: Option<PathBuf>,
    pub backend_session_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub mode: Option<AcpSessionMode>,
    pub state: AcpSessionState,
    pub last_activity_ms: u64,
    pub last_error: Option<String>,
}

impl AcpSessionHandle {
    pub fn into_metadata(
        self,
        conversation_id: Option<String>,
        binding: Option<AcpSessionBindingScope>,
        mode: Option<AcpSessionMode>,
        state: AcpSessionState,
    ) -> AcpSessionMetadata {
        let AcpSessionHandle {
            session_key,
            backend_id,
            runtime_session_name,
            working_directory,
            backend_session_id,
            agent_session_id,
            binding: handle_binding,
        } = self;
        AcpSessionMetadata {
            session_key,
            conversation_id,
            binding: binding.or(handle_binding),
            activation_origin: None,
            backend_id,
            runtime_session_name,
            working_directory,
            backend_session_id,
            agent_session_id,
            mode,
            state,
            last_activity_ms: now_ms(),
            last_error: None,
        }
    }
}

impl AcpSessionMetadata {
    pub fn to_handle(&self) -> AcpSessionHandle {
        AcpSessionHandle {
            session_key: self.session_key.clone(),
            backend_id: self.backend_id.clone(),
            runtime_session_name: self.runtime_session_name.clone(),
            working_directory: self.working_directory.clone(),
            backend_session_id: self.backend_session_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            binding: self.binding.clone(),
        }
    }

    pub fn touch(&mut self) {
        self.last_activity_ms = now_ms();
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
        self.touch();
    }
}

pub const ACP_TURN_METADATA_TRACE_ID: &str = "loongclaw.trace_id";
pub const ACP_TURN_METADATA_SOURCE_MESSAGE_ID: &str = "loongclaw.channel.source_message_id";
pub const ACP_TURN_METADATA_ACK_CURSOR: &str = "loongclaw.channel.ack_cursor";
pub const ACP_TURN_METADATA_ROUTING_INTENT: &str = "loongclaw.acp.routing_intent";
pub const ACP_TURN_METADATA_ROUTING_ORIGIN: &str = "loongclaw.acp.routing_origin";
pub const ACP_SESSION_METADATA_ACTIVATION_ORIGIN: &str = "loongclaw.acp.activation_origin";

#[derive(Clone, Copy, Default)]
pub struct AcpTurnProvenance<'a> {
    pub trace_id: Option<&'a str>,
    pub source_message_id: Option<&'a str>,
    pub ack_cursor: Option<&'a str>,
}

impl AcpTurnProvenance<'_> {
    pub(crate) fn extend_request_metadata(self, metadata: &mut BTreeMap<String, String>) {
        insert_trimmed_metadata(metadata, ACP_TURN_METADATA_TRACE_ID, self.trace_id);
        insert_trimmed_metadata(
            metadata,
            ACP_TURN_METADATA_SOURCE_MESSAGE_ID,
            self.source_message_id,
        );
        insert_trimmed_metadata(metadata, ACP_TURN_METADATA_ACK_CURSOR, self.ack_cursor);
    }
}

#[derive(Clone, Copy)]
pub struct AcpConversationTurnOptions<'a> {
    pub routing_intent: AcpRoutingIntent,
    pub event_sink: Option<&'a dyn AcpTurnEventSink>,
    pub additional_bootstrap_mcp_servers: Option<&'a [String]>,
    pub working_directory: Option<&'a Path>,
    pub provenance: AcpTurnProvenance<'a>,
}

impl Default for AcpConversationTurnOptions<'_> {
    fn default() -> Self {
        Self {
            routing_intent: AcpRoutingIntent::Automatic,
            event_sink: None,
            additional_bootstrap_mcp_servers: None,
            working_directory: None,
            provenance: AcpTurnProvenance::default(),
        }
    }
}

impl<'a> AcpConversationTurnOptions<'a> {
    pub fn automatic() -> Self {
        Self::default()
    }

    pub fn explicit() -> Self {
        Self {
            routing_intent: AcpRoutingIntent::Explicit,
            ..Self::default()
        }
    }

    pub fn from_event_sink(event_sink: Option<&'a dyn AcpTurnEventSink>) -> Self {
        if event_sink.is_some() {
            Self::explicit().with_event_sink(event_sink)
        } else {
            Self::automatic()
        }
    }

    pub fn with_event_sink(mut self, event_sink: Option<&'a dyn AcpTurnEventSink>) -> Self {
        self.event_sink = event_sink;
        self
    }

    pub fn with_additional_bootstrap_mcp_servers(
        mut self,
        additional_bootstrap_mcp_servers: &'a [String],
    ) -> Self {
        self.additional_bootstrap_mcp_servers = (!additional_bootstrap_mcp_servers.is_empty())
            .then_some(additional_bootstrap_mcp_servers);
        self
    }

    pub fn with_working_directory(mut self, working_directory: Option<&'a Path>) -> Self {
        self.working_directory = working_directory.filter(|path| !path.as_os_str().is_empty());
        self
    }

    pub fn with_provenance(mut self, provenance: AcpTurnProvenance<'a>) -> Self {
        self.provenance = provenance;
        self
    }
}

#[derive(Default)]
pub struct BufferedAcpTurnEventSink {
    events: Mutex<Vec<Value>>,
}

impl BufferedAcpTurnEventSink {
    pub fn snapshot(&self) -> CliResult<Vec<Value>> {
        self.events
            .lock()
            .map(|guard| guard.clone())
            .map_err(|_error| "ACP buffered turn event sink lock poisoned".to_owned())
    }
}

impl AcpTurnEventSink for BufferedAcpTurnEventSink {
    fn on_event(&self, event: &Value) -> CliResult<()> {
        self.events
            .lock()
            .map_err(|_error| "ACP buffered turn event sink lock poisoned".to_owned())?
            .push(event.clone());
        Ok(())
    }
}

pub struct CompositeAcpTurnEventSink<'a> {
    pub primary: &'a dyn AcpTurnEventSink,
    pub secondary: &'a dyn AcpTurnEventSink,
}

impl AcpTurnEventSink for CompositeAcpTurnEventSink<'_> {
    fn on_event(&self, event: &Value) -> CliResult<()> {
        self.primary.on_event(event)?;
        self.secondary.on_event(event)
    }
}

/// A reusable ACP-owned sink that emits one serialized runtime event per line.
///
/// This keeps simple operator-facing streaming surfaces, such as CLI stderr output,
/// behind the ACP event contract instead of scattering caller-local printers.
pub struct JsonlAcpTurnEventSink<W> {
    writer: Mutex<W>,
    prefix: String,
}

impl<W> JsonlAcpTurnEventSink<W> {
    pub fn new(writer: W) -> Self {
        Self::with_prefix(writer, "")
    }

    pub fn with_prefix(writer: W, prefix: impl Into<String>) -> Self {
        Self {
            writer: Mutex::new(writer),
            prefix: prefix.into(),
        }
    }
}

impl JsonlAcpTurnEventSink<io::Stderr> {
    /// Construct a JSONL sink that writes runtime events to process stderr.
    pub fn stderr_with_prefix(prefix: impl Into<String>) -> Self {
        Self::with_prefix(io::stderr(), prefix)
    }
}

impl<W> AcpTurnEventSink for JsonlAcpTurnEventSink<W>
where
    W: Write + Send,
{
    fn on_event(&self, event: &Value) -> CliResult<()> {
        let rendered = serde_json::to_string(event)
            .map_err(|error| format!("serialize ACP runtime event failed: {error}"))?;
        let mut writer = self
            .writer
            .lock()
            .map_err(|_error| "ACP JSONL turn event sink lock poisoned".to_owned())?;
        writer
            .write_all(self.prefix.as_bytes())
            .map_err(|error| format!("write ACP runtime event prefix failed: {error}"))?;
        writer
            .write_all(rendered.as_bytes())
            .map_err(|error| format!("write ACP runtime event payload failed: {error}"))?;
        writer
            .write_all(b"\n")
            .map_err(|error| format!("write ACP runtime event newline failed: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("flush ACP runtime event sink failed: {error}"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionBootstrap {
    pub session_key: String,
    pub conversation_id: Option<String>,
    pub binding: Option<AcpSessionBindingScope>,
    pub working_directory: Option<PathBuf>,
    pub initial_prompt: Option<String>,
    pub mode: Option<AcpSessionMode>,
    pub mcp_servers: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpTurnRequest {
    pub session_key: String,
    pub input: String,
    pub working_directory: Option<PathBuf>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpTurnStopReason {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcpTurnResult {
    pub output_text: String,
    pub state: AcpSessionState,
    pub usage: Option<Value>,
    pub events: Vec<Value>,
    pub stop_reason: Option<AcpTurnStopReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingTokenEvent {
    pub event_type: String,
    pub delta: TokenDelta,
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenDelta {
    pub text: Option<String>,
    pub tool_call: Option<ToolCallDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub name: Option<String>,
    pub args: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionStatus {
    pub session_key: String,
    pub backend_id: String,
    pub conversation_id: Option<String>,
    pub binding: Option<AcpSessionBindingScope>,
    pub activation_origin: Option<AcpRoutingOrigin>,
    pub state: AcpSessionState,
    pub mode: Option<AcpSessionMode>,
    pub pending_turns: usize,
    pub active_turn_id: Option<String>,
    pub last_activity_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpConfigPatch {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpDoctorReport {
    pub healthy: bool,
    pub diagnostics: BTreeMap<String, String>,
}

pub trait AcpTurnEventSink: Send + Sync {
    fn on_event(&self, event: &Value) -> CliResult<()>;
}

#[derive(Debug, Clone)]
pub struct AcpAbortSignal {
    receiver: watch::Receiver<bool>,
}

#[derive(Debug)]
pub struct AcpAbortController {
    sender: watch::Sender<bool>,
}

impl Default for AcpAbortController {
    fn default() -> Self {
        Self::new()
    }
}

impl AcpAbortController {
    pub fn new() -> Self {
        let (sender, _receiver) = watch::channel(false);
        Self { sender }
    }

    pub fn signal(&self) -> AcpAbortSignal {
        AcpAbortSignal {
            receiver: self.sender.subscribe(),
        }
    }

    pub fn abort(&self) {
        let _ = self.sender.send(true);
    }

    pub fn is_aborted(&self) -> bool {
        *self.sender.borrow()
    }
}

impl AcpAbortSignal {
    pub fn is_aborted(&self) -> bool {
        *self.receiver.borrow()
    }

    pub async fn cancelled(&mut self) {
        if self.is_aborted() {
            return;
        }

        while self.receiver.changed().await.is_ok() {
            if self.is_aborted() {
                return;
            }
        }
    }
}

#[async_trait]
pub trait AcpRuntimeBackend: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> AcpBackendMetadata {
        AcpBackendMetadata::new(self.id(), [], "ACP runtime backend")
    }

    async fn ensure_session(
        &self,
        config: &LoongClawConfig,
        request: &AcpSessionBootstrap,
    ) -> CliResult<AcpSessionHandle>;

    async fn run_turn(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
        request: &AcpTurnRequest,
    ) -> CliResult<AcpTurnResult>;

    async fn run_turn_with_sink(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
        request: &AcpTurnRequest,
        _abort: Option<AcpAbortSignal>,
        sink: Option<&dyn AcpTurnEventSink>,
    ) -> CliResult<AcpTurnResult> {
        let result = self.run_turn(config, session, request).await?;
        if let Some(sink) = sink {
            for event in &result.events {
                sink.on_event(event)?;
            }
        }
        Ok(result)
    }

    async fn cancel(&self, config: &LoongClawConfig, session: &AcpSessionHandle) -> CliResult<()>;

    async fn close(&self, config: &LoongClawConfig, session: &AcpSessionHandle) -> CliResult<()>;

    async fn get_status(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
    ) -> CliResult<Option<AcpSessionStatus>> {
        Ok(None)
    }

    async fn set_mode(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
        _mode: AcpSessionMode,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn set_config_option(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
        _patch: &AcpConfigPatch,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn doctor(&self, _config: &LoongClawConfig) -> CliResult<Option<AcpDoctorReport>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct PlanningStubAcpBackend;

#[async_trait]
impl AcpRuntimeBackend for PlanningStubAcpBackend {
    fn id(&self) -> &'static str {
        "planning_stub"
    }

    fn metadata(&self) -> AcpBackendMetadata {
        AcpBackendMetadata::new(
            self.id(),
            [AcpCapability::Doctor],
            "Placeholder ACP backend for control-plane wiring, config selection, and diagnostics.",
        )
    }

    async fn ensure_session(
        &self,
        _config: &LoongClawConfig,
        _request: &AcpSessionBootstrap,
    ) -> CliResult<AcpSessionHandle> {
        Err("ACP backend `planning_stub` is a placeholder and cannot spawn sessions yet".to_owned())
    }

    async fn run_turn(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
        _request: &AcpTurnRequest,
    ) -> CliResult<AcpTurnResult> {
        Err("ACP backend `planning_stub` is a placeholder and cannot execute turns yet".to_owned())
    }

    async fn cancel(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
    ) -> CliResult<()> {
        Err(
            "ACP backend `planning_stub` is a placeholder and cannot cancel sessions yet"
                .to_owned(),
        )
    }

    async fn close(&self, _config: &LoongClawConfig, _session: &AcpSessionHandle) -> CliResult<()> {
        Err("ACP backend `planning_stub` is a placeholder and cannot close sessions yet".to_owned())
    }

    async fn doctor(&self, _config: &LoongClawConfig) -> CliResult<Option<AcpDoctorReport>> {
        Ok(Some(AcpDoctorReport {
            healthy: false,
            diagnostics: BTreeMap::from([
                ("backend".to_owned(), self.id().to_owned()),
                ("status".to_owned(), "placeholder".to_owned()),
                (
                    "message".to_owned(),
                    "planning_stub only validates ACP control-plane wiring; switch to a real backend such as `acpx` before enabling runtime execution".to_owned(),
                ),
            ]),
        }))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn insert_trimmed_metadata(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<&str>,
) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    metadata.insert(key.to_owned(), value.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::Arc;

    #[test]
    fn session_handle_into_metadata_preserves_handle_binding_when_explicit_binding_missing() {
        let handle = AcpSessionHandle {
            session_key: "agent:codex:opaque-session".to_owned(),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-1".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-1".to_owned()),
            agent_session_id: Some("agent-1".to_owned()),
            binding: Some(AcpSessionBindingScope {
                route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc_123".to_owned()),
                thread_id: Some("om_thread_1".to_owned()),
            }),
        };

        let metadata = handle.into_metadata(
            Some("opaque-session".to_owned()),
            None,
            Some(AcpSessionMode::Interactive),
            AcpSessionState::Ready,
        );
        assert_eq!(
            metadata
                .binding
                .as_ref()
                .map(|binding| binding.route_session_id.as_str()),
            Some("feishu:lark-prod:oc_123:om_thread_1")
        );
        assert_eq!(
            metadata
                .binding
                .as_ref()
                .and_then(|binding| binding.thread_id.as_deref()),
            Some("om_thread_1")
        );
    }

    #[test]
    fn session_metadata_to_handle_preserves_binding_scope() {
        let metadata = AcpSessionMetadata {
            session_key: "agent:codex:opaque-session".to_owned(),
            conversation_id: Some("opaque-session".to_owned()),
            binding: Some(AcpSessionBindingScope {
                route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc_123".to_owned()),
                thread_id: Some("om_thread_1".to_owned()),
            }),
            activation_origin: Some(AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-1".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-1".to_owned()),
            agent_session_id: Some("agent-1".to_owned()),
            mode: Some(AcpSessionMode::Interactive),
            state: AcpSessionState::Ready,
            last_activity_ms: 123,
            last_error: None,
        };

        let handle = metadata.to_handle();
        assert_eq!(
            handle
                .binding
                .as_ref()
                .map(|binding| binding.route_session_id.as_str()),
            Some("feishu:lark-prod:oc_123:om_thread_1")
        );
        assert_eq!(
            handle
                .binding
                .as_ref()
                .and_then(|binding| binding.thread_id.as_deref()),
            Some("om_thread_1")
        );
    }

    #[test]
    fn acp_routing_origin_parse_accepts_known_values() {
        assert_eq!(
            AcpRoutingOrigin::parse("explicit_request"),
            Some(AcpRoutingOrigin::ExplicitRequest)
        );
        assert_eq!(
            AcpRoutingOrigin::parse("automatic_agent_prefixed"),
            Some(AcpRoutingOrigin::AutomaticAgentPrefixed)
        );
        assert_eq!(
            AcpRoutingOrigin::parse("automatic_dispatch"),
            Some(AcpRoutingOrigin::AutomaticDispatch)
        );
        assert_eq!(AcpRoutingOrigin::parse("unknown"), None);
    }

    #[test]
    fn acp_conversation_turn_options_from_event_sink_marks_explicit_request() {
        struct NoopSink;

        impl AcpTurnEventSink for NoopSink {
            fn on_event(&self, _event: &Value) -> CliResult<()> {
                Ok(())
            }
        }

        let sink = NoopSink;
        let options = AcpConversationTurnOptions::from_event_sink(Some(&sink));
        assert_eq!(options.routing_intent, AcpRoutingIntent::Explicit);
        assert!(options.event_sink.is_some());

        let automatic = AcpConversationTurnOptions::from_event_sink(None);
        assert_eq!(automatic.routing_intent, AcpRoutingIntent::Automatic);
        assert!(automatic.event_sink.is_none());
    }

    #[test]
    fn acp_conversation_turn_options_builders_normalize_optional_fields() {
        let empty_servers = Vec::new();
        let populated_servers = vec!["filesystem".to_owned(), "github".to_owned()];
        let provenance = AcpTurnProvenance {
            trace_id: Some(" trace-123 "),
            source_message_id: Some("msg-42"),
            ack_cursor: Some("ack-9"),
        };

        let options = AcpConversationTurnOptions::automatic()
            .with_additional_bootstrap_mcp_servers(&empty_servers)
            .with_working_directory(Some(Path::new("")))
            .with_provenance(provenance);
        assert!(options.additional_bootstrap_mcp_servers.is_none());
        assert!(options.working_directory.is_none());
        assert_eq!(options.provenance.trace_id, Some(" trace-123 "));

        let options = AcpConversationTurnOptions::explicit()
            .with_additional_bootstrap_mcp_servers(&populated_servers)
            .with_working_directory(Some(Path::new("/workspace/project")));
        assert_eq!(options.routing_intent, AcpRoutingIntent::Explicit);
        assert_eq!(
            options.additional_bootstrap_mcp_servers,
            Some(populated_servers.as_slice())
        );
        assert_eq!(
            options.working_directory,
            Some(Path::new("/workspace/project"))
        );
    }

    #[derive(Clone, Default)]
    struct SharedBufferWriter(Arc<Mutex<Vec<u8>>>);

    impl SharedBufferWriter {
        fn snapshot(&self) -> Vec<u8> {
            self.0.lock().expect("shared buffer writer lock").clone()
        }
    }

    impl Write for SharedBufferWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("shared buffer writer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn jsonl_turn_event_sink_writes_prefixed_json_lines() {
        let writer = SharedBufferWriter::default();
        let sink = JsonlAcpTurnEventSink::with_prefix(writer.clone(), "acp-event> ");

        sink.on_event(&serde_json::json!({
            "type": "text",
            "text": "hello"
        }))
        .expect("event should be written");

        let rendered =
            String::from_utf8(writer.snapshot()).expect("event sink output should be valid utf-8");
        assert_eq!(
            rendered,
            "acp-event> {\"text\":\"hello\",\"type\":\"text\"}\n"
        );
    }
}
