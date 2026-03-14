use async_trait::async_trait;
use std::collections::BTreeSet;
use std::sync::Arc;
#[cfg(feature = "memory-sqlite")]
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use loongclaw_contracts::{Capability, KernelError, ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};

use crate::config::{LoongClawConfig, SessionVisibility, ToolConfig};
use crate::context::KernelContext;
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::tools::{
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    runtime_tool_view, runtime_tool_view_for_config, tool_catalog, ToolExecutionKind, ToolView,
};

use super::runtime::SessionContext;

pub use crate::tools::delegate::{AsyncDelegateSpawnRequest, AsyncDelegateSpawner};

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
    request: &crate::tools::delegate::AsyncDelegateSpawnRequest,
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
impl crate::tools::delegate::AsyncDelegateSpawner for SubprocessAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::tools::delegate::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
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
    async_delegate_spawner: Option<Arc<dyn crate::tools::delegate::AsyncDelegateSpawner>>,
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
        async_delegate_spawner: Arc<dyn crate::tools::delegate::AsyncDelegateSpawner>,
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
            Self::production(
                crate::memory::runtime_config::get_memory_runtime_config().clone(),
                ToolConfig::default(),
            )
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
        crate::tools::execute_app_tool_with_runtime_support(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
            crate::tools::AppToolRuntimeSupport {
                app_config: self.app_config.as_deref(),
                async_delegate_spawner: self.async_delegate_spawner.clone(),
            },
        )
        .await
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
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            };
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
        let request = crate::tools::delegate::AsyncDelegateSpawnRequest {
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
