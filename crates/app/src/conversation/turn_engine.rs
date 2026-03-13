use async_trait::async_trait;
use futures_util::FutureExt;
use std::any::Any;
use std::collections::BTreeSet;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
#[cfg(feature = "memory-sqlite")]
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use loongclaw_contracts::{Capability, KernelError, ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::{LoongClawConfig, SessionVisibility, ToolConfig};
use crate::context::KernelContext;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{build_async_spawn_failure_recovery_payload, RECOVERY_EVENT_KIND};
use crate::tools::{
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    runtime_tool_view, runtime_tool_view_for_config, tool_catalog, ToolExecutionKind, ToolView,
};

use super::runtime::SessionContext;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub assistant_text: String,
    pub tool_intents: Vec<ToolIntent>,
    pub raw_meta: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntent {
    pub tool_name: String,
    pub args_json: serde_json::Value,
    pub source: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecision {
    pub allow: bool,
    pub deny: bool,
    pub approval_required: bool,
    pub reason: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TurnResult {
    FinalText(String),
    NeedsApproval(String),
    ToolDenied(String),
    ToolError(String),
    ProviderError(String),
}

#[async_trait]
pub trait AppToolDispatcher: Send + Sync {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncDelegateSpawnRequest {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub label: Option<String>,
    pub timeout_seconds: u64,
}

#[async_trait]
pub trait AsyncDelegateSpawner: Send + Sync {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String>;
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct AsyncDelegateSubprocessPlan {
    program: PathBuf,
    args: Vec<String>,
}

#[cfg(feature = "memory-sqlite")]
fn build_async_delegate_subprocess_plan(
    executable_path: &Path,
    config_path: Option<&str>,
    request: &AsyncDelegateSpawnRequest,
) -> AsyncDelegateSubprocessPlan {
    let mut args = vec!["run-turn".to_owned()];
    if let Some(config_path) = config_path.map(str::trim).filter(|value| !value.is_empty()) {
        args.push("--config".to_owned());
        args.push(config_path.to_owned());
    }
    args.extend([
        "--session".to_owned(),
        request.child_session_id.clone(),
        "--input".to_owned(),
        request.task.clone(),
        "--timeout-seconds".to_owned(),
        request.timeout_seconds.to_string(),
        "--delegate-child".to_owned(),
    ]);
    AsyncDelegateSubprocessPlan {
        program: executable_path.to_path_buf(),
        args,
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
struct SubprocessAsyncDelegateSpawner {
    executable_path: PathBuf,
    config_path: Option<String>,
}

#[cfg(feature = "memory-sqlite")]
impl SubprocessAsyncDelegateSpawner {
    fn from_current_process() -> Result<Self, String> {
        let executable_path = std::env::current_exe().map_err(|error| {
            format!("resolve current executable for async delegate failed: {error}")
        })?;
        let config_path = std::env::var("LOONGCLAW_CONFIG_PATH")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Ok(Self {
            executable_path,
            config_path,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AsyncDelegateSpawner for SubprocessAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        let plan = build_async_delegate_subprocess_plan(
            &self.executable_path,
            self.config_path.as_deref(),
            &request,
        );
        let mut command = tokio::process::Command::new(&plan.program);
        command.args(&plan.args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|error| format!("spawn async delegate subprocess failed: {error}"))?;
        drop(child);
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
fn finalize_async_delegate_spawn_failure(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    error: String,
) -> Result<(), String> {
    let repo = crate::session::repository::SessionRepository::new(memory_config)?;
    let outcome = crate::tools::delegate::delegate_error_outcome(
        child_session_id.to_owned(),
        label,
        error.clone(),
        0,
    );
    repo.finalize_session_terminal(
        child_session_id,
        crate::session::repository::FinalizeSessionTerminalRequest {
            state: crate::session::repository::SessionState::Failed,
            last_error: Some(error.clone()),
            event_kind: "delegate_spawn_failed".to_owned(),
            actor_session_id: Some(parent_session_id.to_owned()),
            event_payload_json: json!({
                "error": error,
            }),
            outcome_status: outcome.status,
            outcome_payload_json: outcome.payload,
        },
    )?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn finalize_async_delegate_spawn_failure_with_recovery(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    error: String,
) -> Result<(), String> {
    let recovery_label = label.clone();
    match finalize_async_delegate_spawn_failure(
        memory_config,
        child_session_id,
        parent_session_id,
        label,
        error.clone(),
    ) {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let repo = crate::session::repository::SessionRepository::new(memory_config)?;
            let recovery_error = format!(
                "delegate_async_spawn_failure_persist_failed: {finalize_error}; original spawn error: {error}"
            );
            match repo.transition_session_with_event_if_current(
                child_session_id,
                crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                    expected_state: crate::session::repository::SessionState::Ready,
                    next_state: crate::session::repository::SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
                    event_payload_json: build_async_spawn_failure_recovery_payload(
                        recovery_label.as_deref(),
                        &error,
                        &recovery_error,
                    ),
                },
            ) {
                Ok(Some(_)) => Ok(()),
                Ok(None) => {
                    let current_state = repo
                        .load_session(child_session_id)?
                        .map(|session| session.state.as_str().to_owned())
                        .unwrap_or_else(|| "missing".to_owned());
                    Err(format!(
                        "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                    ))
                }
                Err(recovery_event_error) => match repo.update_session_state_if_current(
                    child_session_id,
                    crate::session::repository::SessionState::Ready,
                    crate::session::repository::SessionState::Failed,
                    Some(recovery_error.clone()),
                ) {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => {
                        let current_state = repo
                            .load_session(child_session_id)?
                            .map(|session| session.state.as_str().to_owned())
                            .unwrap_or_else(|| "missing".to_owned());
                        Err(format!(
                            "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                        ))
                    }
                    Err(mark_error) => Err(format!(
                        "{recovery_error}; delegate_async_spawn_recovery_failed: {mark_error}; delegate_async_spawn_recovery_event_failed: {recovery_event_error}"
                    )),
                },
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn format_async_delegate_spawn_panic(panic_payload: Box<dyn Any + Send>) -> String {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return format!("delegate_async_spawn_panic: {}", *message),
        Err(panic_payload) => panic_payload,
    };
    match panic_payload.downcast::<&'static str>() {
        Ok(message) => format!("delegate_async_spawn_panic: {}", *message),
        Err(_) => "delegate_async_spawn_panic".to_owned(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn spawn_async_delegate_detached(
    runtime_handle: tokio::runtime::Handle,
    memory_config: MemoryRuntimeConfig,
    spawner: Arc<dyn AsyncDelegateSpawner>,
    request: AsyncDelegateSpawnRequest,
) {
    let child_session_id = request.child_session_id.clone();
    let parent_session_id = request.parent_session_id.clone();
    let label = request.label.clone();
    runtime_handle.spawn(async move {
        let spawn_failure = match AssertUnwindSafe(spawner.spawn(request)).catch_unwind().await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(panic_payload) => Some(format_async_delegate_spawn_panic(panic_payload)),
        };
        if let Some(error) = spawn_failure {
            if let Err(finalize_error) = finalize_async_delegate_spawn_failure_with_recovery(
                &memory_config,
                &child_session_id,
                &parent_session_id,
                label,
                error.clone(),
            ) {
                eprintln!(
                    "error: async delegate spawn failure persistence failed for `{child_session_id}`: {finalize_error}; original spawn error: {error}"
                );
            }
        }
    });
}

pub struct NoopAppToolDispatcher;

#[async_trait]
impl AppToolDispatcher for NoopAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        _session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        Err(format!("app_tool_not_implemented: {}", request.tool_name))
    }
}

#[derive(Clone)]
pub struct DefaultAppToolDispatcher {
    memory_config: MemoryRuntimeConfig,
    tool_config: ToolConfig,
    app_config: Option<Arc<LoongClawConfig>>,
    async_delegate_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
}

impl DefaultAppToolDispatcher {
    pub fn new(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        Self {
            memory_config,
            tool_config,
            app_config: None,
            async_delegate_spawner: None,
        }
    }

    pub fn with_config(memory_config: MemoryRuntimeConfig, config: LoongClawConfig) -> Self {
        Self {
            memory_config,
            tool_config: config.tools.clone(),
            app_config: Some(Arc::new(config)),
            async_delegate_spawner: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn production(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        let async_delegate_spawner = SubprocessAsyncDelegateSpawner::from_current_process()
            .ok()
            .map(|spawner| Arc::new(spawner) as Arc<dyn AsyncDelegateSpawner>);
        Self {
            memory_config,
            tool_config,
            app_config: None,
            async_delegate_spawner,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn production_with_config(
        memory_config: MemoryRuntimeConfig,
        config: LoongClawConfig,
    ) -> Self {
        let async_delegate_spawner = SubprocessAsyncDelegateSpawner::from_current_process()
            .ok()
            .map(|spawner| Arc::new(spawner) as Arc<dyn AsyncDelegateSpawner>);
        Self {
            memory_config,
            tool_config: config.tools.clone(),
            app_config: Some(Arc::new(config)),
            async_delegate_spawner,
        }
    }

    pub fn with_async_delegate_spawner(
        memory_config: MemoryRuntimeConfig,
        tool_config: ToolConfig,
        async_delegate_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        Self {
            memory_config,
            tool_config,
            app_config: None,
            async_delegate_spawner: Some(async_delegate_spawner),
        }
    }

    pub fn runtime() -> Self {
        #[cfg(feature = "memory-sqlite")]
        {
            return Self::production(
                crate::memory::runtime_config::get_memory_runtime_config().clone(),
                ToolConfig::default(),
            );
        }
        #[cfg(not(feature = "memory-sqlite"))]
        Self::new(
            crate::memory::runtime_config::get_memory_runtime_config().clone(),
            ToolConfig::default(),
        )
    }

    fn effective_tool_config_for_session(&self, session_context: &SessionContext) -> ToolConfig {
        let mut tool_config = self.tool_config.clone();
        if session_context.parent_session_id.is_some() {
            tool_config.sessions.visibility = SessionVisibility::SelfOnly;
        }
        tool_config
    }

    #[cfg(feature = "memory-sqlite")]
    fn effective_tool_view_for_session(
        &self,
        session_context: &SessionContext,
    ) -> Result<ToolView, String> {
        let repo = crate::session::repository::SessionRepository::new(&self.memory_config)?;
        if let Some(session) = repo.load_session(&session_context.session_id)? {
            if session.parent_session_id.is_some() {
                let depth = repo
                    .session_lineage_depth(&session_context.session_id)
                    .map_err(|error| {
                        format!("compute session lineage depth for dispatcher tool view failed: {error}")
                    })?;
                let allow_nested_delegate = depth < self.tool_config.delegate.max_depth;
                return Ok(delegate_child_tool_view_for_config_with_delegate(
                    &self.tool_config,
                    allow_nested_delegate,
                ));
            }
            return Ok(runtime_tool_view_for_config(&self.tool_config));
        }
        if repo
            .load_session_summary_with_legacy_fallback(&session_context.session_id)?
            .is_some_and(|session| {
                session.kind == crate::session::repository::SessionKind::DelegateChild
            })
        {
            return Ok(delegate_child_tool_view_for_config(&self.tool_config));
        }
        Ok(runtime_tool_view_for_config(&self.tool_config))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn effective_tool_view_for_session(
        &self,
        _session_context: &SessionContext,
    ) -> Result<ToolView, String> {
        Ok(runtime_tool_view_for_config(&self.tool_config))
    }

    #[cfg(feature = "memory-sqlite")]
    async fn execute_delegate_async(
        &self,
        session_context: &SessionContext,
        payload: serde_json::Value,
    ) -> Result<ToolCoreOutcome, String> {
        if !self.tool_config.delegate.enabled {
            return Err("app_tool_disabled: delegate is disabled by config".to_owned());
        }

        let delegate_request = crate::tools::delegate::parse_delegate_request_with_default_timeout(
            &payload,
            self.tool_config.delegate.timeout_seconds,
        )?;
        let spawner = self
            .async_delegate_spawner
            .as_ref()
            .ok_or_else(|| "delegate_async_not_configured".to_owned())?;
        let runtime_handle = tokio::runtime::Handle::try_current()
            .map_err(|error| format!("delegate_async_runtime_unavailable: {error}"))?;
        let repo = crate::session::repository::SessionRepository::new(&self.memory_config)?;
        let current_depth = repo.session_lineage_depth(&session_context.session_id)?;
        let next_child_depth = current_depth.saturating_add(1);
        if next_child_depth > self.tool_config.delegate.max_depth {
            return Err(format!(
                "delegate_depth_exceeded: next child depth {next_child_depth} exceeds configured max_depth {}",
                self.tool_config.delegate.max_depth
            ));
        }

        let child_session_id = crate::tools::delegate::next_delegate_session_id();
        let child_label = delegate_request.label.clone();
        repo.create_session_with_event(
            crate::session::repository::CreateSessionWithEventRequest {
                session: crate::session::repository::NewSessionRecord {
                    session_id: child_session_id.clone(),
                    kind: crate::session::repository::SessionKind::DelegateChild,
                    parent_session_id: Some(session_context.session_id.clone()),
                    label: child_label.clone(),
                    state: crate::session::repository::SessionState::Ready,
                },
                event_kind: "delegate_queued".to_owned(),
                actor_session_id: Some(session_context.session_id.clone()),
                event_payload_json: json!({
                    "task": delegate_request.task.clone(),
                    "label": child_label.clone(),
                    "timeout_seconds": delegate_request.timeout_seconds,
                }),
            },
        )?;

        let spawn_request = AsyncDelegateSpawnRequest {
            child_session_id: child_session_id.clone(),
            parent_session_id: session_context.session_id.clone(),
            task: delegate_request.task,
            label: child_label.clone(),
            timeout_seconds: delegate_request.timeout_seconds,
        };

        spawn_async_delegate_detached(
            runtime_handle,
            self.memory_config.clone(),
            Arc::clone(spawner),
            spawn_request,
        );

        Ok(crate::tools::delegate::delegate_async_queued_outcome(
            child_session_id,
            delegate_request.label,
            delegate_request.timeout_seconds,
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    async fn execute_sessions_send(
        &self,
        session_context: &SessionContext,
        payload: serde_json::Value,
    ) -> Result<ToolCoreOutcome, String> {
        let app_config = self
            .app_config
            .as_ref()
            .ok_or_else(|| "sessions_send_not_configured".to_owned())?;
        let effective_tool_config = self.effective_tool_config_for_session(session_context);
        crate::tools::messaging::execute_sessions_send_with_config(
            payload,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
            app_config.as_ref(),
        )
        .await
    }
}

impl Default for DefaultAppToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl AppToolDispatcher for DefaultAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
        let effective_tool_view = self.effective_tool_view_for_session(session_context)?;
        if let Some(descriptor) = tool_catalog().descriptor(canonical_tool_name) {
            if descriptor.execution_kind == ToolExecutionKind::App
                && (!session_context.tool_view.contains(descriptor.name)
                    || !effective_tool_view.contains(descriptor.name))
            {
                return Err(format!("tool_not_visible: {}", descriptor.name));
            }
        }
        let effective_tool_config = self.effective_tool_config_for_session(session_context);
        if canonical_tool_name == "session_wait" {
            return crate::tools::wait_for_session_with_config(
                request.payload,
                &session_context.session_id,
                &self.memory_config,
                &effective_tool_config,
            )
            .await;
        }
        #[cfg(feature = "memory-sqlite")]
        if canonical_tool_name == "sessions_send" {
            return self
                .execute_sessions_send(session_context, request.payload)
                .await;
        }
        #[cfg(feature = "memory-sqlite")]
        if canonical_tool_name == "delegate_async" {
            return self
                .execute_delegate_async(session_context, request.payload)
                .await;
        }
        crate::tools::execute_app_tool_with_config(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
        )
    }
}

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    max_tool_steps: usize,
}

impl TurnEngine {
    pub fn new(max_tool_steps: usize) -> Self {
        Self { max_tool_steps }
    }

    /// Evaluate a provider turn and produce a deterministic result.
    /// Does NOT execute tools — just validates and gates.
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        self.evaluate_turn_in_view(turn, &runtime_tool_view())
    }

    pub fn evaluate_turn_in_view(&self, turn: &ProviderTurn, tool_view: &ToolView) -> TurnResult {
        self.evaluate_turn_in_context(turn, &session_context_from_turn(turn, tool_view.clone()))
    }

    pub fn evaluate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
    ) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::ToolDenied("max_tool_steps_exceeded".to_owned());
        }

        // Check each tool intent
        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            };
            if !session_context.tool_view.contains(descriptor.name) {
                return TurnResult::ToolDenied(format!("tool_not_visible: {}", intent.tool_name));
            }
        }

        // All tools validated — execution requires a kernel context
        TurnResult::NeedsApproval("kernel_context_required".to_owned())
    }

    /// Execute a provider turn with policy-gated tool execution through the kernel.
    ///
    /// Flow:
    /// 1. No tool intents → `FinalText`
    /// 2. Too many intents → `ToolDenied("max_tool_steps_exceeded")`
    /// 3. Unknown tool → `ToolDenied("tool_not_found: ...")`
    /// 4. No kernel context → `ToolDenied("no_kernel_context")`
    /// 5. Policy/capability check via kernel → `ToolDenied` with reason if denied
    /// 6. Execute tool → map result to `TurnResult`
    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_view(turn, &runtime_tool_view(), kernel_ctx)
            .await
    }

    pub async fn execute_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, tool_view.clone()),
            &DefaultAppToolDispatcher::runtime(),
            kernel_ctx,
        )
        .await
    }

    pub async fn execute_turn_in_context<D: AppToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &D,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::ToolDenied("max_tool_steps_exceeded".to_owned());
        }

        // Check each tool intent is known
        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            };
            if !session_context.tool_view.contains(descriptor.name) {
                return TurnResult::ToolDenied(format!("tool_not_visible: {}", intent.tool_name));
            }
        }

        // Execute each tool intent through the kernel
        let mut outputs = Vec::new();
        for intent in &turn.tool_intents {
            let descriptor = catalog
                .resolve(&intent.tool_name)
                .expect("tool descriptor should remain resolvable after validation");
            let request = ToolCoreRequest {
                tool_name: descriptor.name.to_owned(),
                payload: intent.args_json.clone(),
            };
            match descriptor.execution_kind {
                ToolExecutionKind::Core => {
                    let ctx = match kernel_ctx {
                        Some(ctx) => ctx,
                        None => return TurnResult::ToolDenied("no_kernel_context".to_owned()),
                    };
                    let caps = BTreeSet::from([Capability::InvokeTool]);
                    match ctx
                        .kernel
                        .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                        .await
                    {
                        Ok(outcome) => {
                            outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                        }
                        Err(e) => {
                            return match &e {
                                KernelError::Policy(_)
                                | KernelError::PackCapabilityBoundary { .. } => {
                                    TurnResult::ToolDenied(format!("{e}"))
                                }
                                _ => TurnResult::ToolError(format!("{e}")),
                            };
                        }
                    }
                }
                ToolExecutionKind::App => match app_dispatcher
                    .execute_app_tool(session_context, request, kernel_ctx)
                    .await
                {
                    Ok(outcome) => {
                        outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                    }
                    Err(error) => return TurnResult::ToolError(error),
                },
            }
        }

        TurnResult::FinalText(outputs.join("\n"))
    }
}

#[cfg(all(test, feature = "memory-sqlite"))]
mod tests {
    use super::*;

    #[test]
    fn delegate_async_subprocess_plan_includes_config_path_and_delegate_child_flag() {
        let request = AsyncDelegateSpawnRequest {
            child_session_id: "delegate:child-1".to_owned(),
            parent_session_id: "root-session".to_owned(),
            task: "child task".to_owned(),
            label: Some("research".to_owned()),
            timeout_seconds: 19,
        };

        let plan = build_async_delegate_subprocess_plan(
            std::path::Path::new("/tmp/loongclawd"),
            Some("/tmp/loongclaw.toml"),
            &request,
        );

        assert_eq!(plan.program, std::path::PathBuf::from("/tmp/loongclawd"));
        assert_eq!(
            plan.args,
            vec![
                "run-turn",
                "--config",
                "/tmp/loongclaw.toml",
                "--session",
                "delegate:child-1",
                "--input",
                "child task",
                "--timeout-seconds",
                "19",
                "--delegate-child",
            ]
        );
    }
}

fn session_context_from_turn(turn: &ProviderTurn, tool_view: ToolView) -> SessionContext {
    let session_id = turn
        .tool_intents
        .first()
        .map(|intent| intent.session_id.as_str())
        .unwrap_or("default");
    SessionContext::root_with_tool_view(session_id, tool_view)
}
