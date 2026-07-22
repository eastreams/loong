use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

#[cfg(test)]
use loong_contracts::Capabilities;
use loong_contracts::{Capability, ExecutionRoute, GovernedSessionMode, HarnessKind};
use loong_kernel::{
    FixedClock, InMemoryAuditSink, Kernel, VerticalPackManifest,
    access::fs::{
        FsAtomicWriteAllowPolicy, FsContentSearchAllowPolicy, FsCopyFileAllowPolicy,
        FsCreateDirAllAllowPolicy, FsGlobAllowPolicy, FsInspectPathAllowPolicy,
        FsPathAllowedRootsPolicy, FsReadAllowPolicy, FsReadDirAllowPolicy,
        FsReadFilenameDenyPolicy, FsRemoveDirAllAllowPolicy, FsRemoveFileAllowPolicy,
        FsRenameAllowPolicy, FsResolvePathAllowPolicy, FsWriteAllowPolicy,
    },
    access::memory::{
        MemoryAppendTurnAllowPolicy, MemoryCompactAllowPolicy, MemoryReadStageEnvelopeAllowPolicy,
        MemoryReplaceTurnsAllowPolicy, MemoryTranscriptAllowPolicy, MemoryWindowAllowPolicy,
    },
    policy::PolicyPipelineBuilder,
};
use loong_runtime::runtime::Runtime;

use crate::context::{Context, RuntimeContextFactory, Session};
use crate::conversation::{
    DefaultLegacyToolDispatcher, ProviderTurn, ToolIntent, TurnEngine, TurnResult,
};
use crate::session::store::SessionStoreConfig;
#[cfg(test)]
use crate::tools::ToolView;
use crate::tools::runtime_config::ToolRuntimeConfig;

fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

pub fn lock_process_env_for_tests() -> MutexGuard<'static, ()> {
    env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Test owner for the same Runtime -> Session -> borrowed Context shape as production.
///
/// Tests must call [`Self::context`] inside the execution scope rather than
/// storing a Context or manufacturing a `'static` borrow.
#[cfg(test)]
pub(crate) struct TestRuntimeSession {
    pub(crate) runtime: Arc<Runtime<RuntimeContextFactory>>,
    pub(crate) session: Session,
    /// Explicit owner for requests that fall through the typed tool plane.
    ///
    /// Keeping bearer evidence inside the dispatcher makes tests exercise the
    /// same boundary as production instead of threading raw tokens through the
    /// recursive execution path.
    pub(crate) legacy_tools: DefaultLegacyToolDispatcher,
}

#[cfg(test)]
impl TestRuntimeSession {
    pub(crate) fn from_config(
        config: &crate::config::LoongConfig,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        mode: GovernedSessionMode,
    ) -> Result<Self, String> {
        let session_id = session_id.into();
        let agent_id = agent_id.into();
        let runtime = crate::runtime::bootstrap_runtime_with_config(config)?;
        let session = Session::from_config(runtime.as_ref(), config, session_id, agent_id, mode)?;
        let legacy_tools = DefaultLegacyToolDispatcher::with_config(
            Arc::clone(&runtime),
            &session,
            SessionStoreConfig::from_memory_config(&config.memory),
            config.clone(),
        )?;
        Ok(Self {
            runtime,
            session,
            legacy_tools,
        })
    }

    #[must_use]
    pub(crate) fn context(&self) -> Context<'_> {
        Context::new(&self.runtime, &self.session)
            .expect("test Runtime and Session should share one ownership domain")
    }
}

/// Persist the canonical identity required by typed Session fixtures.
///
/// This is deliberately test-only and is not a legacy backfill path. Tests that
/// exercise legacy read models must continue to seed turn-only identities
/// without calling it.
#[cfg(all(test, feature = "memory-sqlite"))]
pub(crate) fn ensure_root_session_for_test(
    config: &crate::config::LoongConfig,
    session_id: &str,
) -> Result<(), String> {
    use crate::session::repository::{
        NewSessionRecord, SessionKind, SessionRepository, SessionState,
    };

    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)?;
    repo.ensure_session(NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: SessionState::Ready,
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn runtime_session_for_test(
    session_id: impl Into<String>,
    tool_view: ToolView,
) -> TestRuntimeSession {
    let session_id = session_id.into();
    let config = crate::config::LoongConfig::default();
    let runtime = crate::runtime::bootstrap_runtime_with_config(&config).expect("test runtime");
    let session = Session::root(
        runtime.as_ref(),
        "test-agent",
        session_id,
        GovernedSessionMode::MutatingCapable,
        Capabilities::from([
            Capability::InvokeTool,
            Capability::NetworkEgress,
            Capability::MemoryRead,
            Capability::MemoryWrite,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        ToolRuntimeConfig::from_loong_config(&config, None),
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
            &config.memory,
        ),
        tool_view,
        None,
        None,
    )
    .expect("test session");
    let legacy_tools = DefaultLegacyToolDispatcher::with_config(
        Arc::clone(&runtime),
        &session,
        SessionStoreConfig::from_memory_config(&config.memory),
        config,
    )
    .expect("legacy test fallback");
    TestRuntimeSession {
        runtime,
        session,
        legacy_tools,
    }
}

#[cfg(test)]
pub(crate) fn child_runtime_session_for_test(
    session_id: impl Into<String>,
    parent_session_id: impl Into<String>,
    tool_view: ToolView,
) -> TestRuntimeSession {
    let mut owner = runtime_session_for_test(parent_session_id.into(), tool_view.clone());
    let execution = crate::conversation::ConstrainedSubagentExecution {
        mode: crate::conversation::ConstrainedSubagentMode::Inline,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 2,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: tool_view.tool_names().map(str::to_owned).collect(),
        capability_ceiling: owner.session.baseline_capabilities().clone(),
        workspace_root: None,
        runtime_narrowing: Default::default(),
        identity: None,
        profile: None,
    };
    owner.session = owner
        .session
        .delegate_child(session_id, tool_view, execution, None)
        .expect("test child session");
    owner.legacy_tools = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        SessionStoreConfig::default(),
        crate::config::ToolConfig::default(),
    )
    .expect("legacy child test fallback");
    owner
}

/// Monotonic counter for unique harness IDs (avoids temp dir collisions).
pub(crate) static HARNESS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ergonomic builder for constructing fake `ProviderTurn` responses in tests.
pub struct FakeProviderBuilder {
    text: String,
    tool_calls: Vec<(String, serde_json::Value)>,
}

impl FakeProviderBuilder {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            tool_calls: Vec::new(),
        }
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.text = text.to_owned();
        self
    }

    pub fn with_tool_call(mut self, tool_name: &str, args: serde_json::Value) -> Self {
        self.tool_calls.push((tool_name.to_owned(), args));
        self
    }

    pub fn build(self) -> ProviderTurn {
        let tool_intents = self
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(i, (name, args))| {
                // Bridge non-provider-exposed tools through tool.invoke with a
                // valid lease, mirroring what the real provider shape layer does.
                let (bridged_name, bridged_args) =
                    crate::tools::bridge_provider_tool_call_with_scope(
                        &name,
                        args,
                        Some("test-session"),
                        Some("test-turn"),
                    );
                ToolIntent {
                    tool_name: bridged_name.into(),
                    args_json: bridged_args,
                    source: "fake_provider".to_owned(),
                    turn_id: "test-turn".to_owned(),
                    tool_call_id: format!("call-{i}"),
                }
            })
            .collect();

        ProviderTurn {
            assistant_text: self.text,
            tool_intents,
            raw_meta: serde_json::Value::Null,
        }
    }
}

/// Integration test harness composing real kernel + real tools + fake provider.
///
/// Each harness gets:
/// - A unique temp dir (no collision between parallel tests)
/// - Session-owned `ToolRuntimeConfig` for the explicit legacy fallback
/// - A real `InMemoryAuditSink` for audit assertions
/// - `max_tool_steps = 1`
#[allow(dead_code)]
pub struct TurnTestHarness {
    pub engine: TurnEngine,
    pub runtime: Arc<Runtime<RuntimeContextFactory>>,
    pub session: Session,
    pub legacy_tools: DefaultLegacyToolDispatcher,
    pub audit: Arc<InMemoryAuditSink>,
    pub temp_dir: PathBuf,
}

impl TurnTestHarness {
    pub fn new() -> Self {
        Self::with_capabilities(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::MemoryRead,
        ]))
    }

    pub fn with_capabilities(capabilities: BTreeSet<Capability>) -> Self {
        Self::with_tool_config(capabilities, ToolRuntimeConfig::default())
    }

    /// Construct a harness with a caller-supplied `ToolRuntimeConfig`.
    /// Use this when a test needs specific allow/deny/approval lists rather
    /// than the generic defaults.
    pub fn with_tool_config(
        capabilities: BTreeSet<Capability>,
        tool_config_override: ToolRuntimeConfig,
    ) -> Self {
        let id = HARNESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir =
            std::env::temp_dir().join(format!("loong-integ-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        // Merge the caller's overrides with the unique temp dir as file_root.
        let tool_config = ToolRuntimeConfig {
            file_root: Some(temp_dir.clone()),
            config_path: Some(temp_dir.join("loong.toml")),
            ..tool_config_override
        };
        let memory_config = SessionStoreConfig::for_sqlite_path(temp_dir.join("memory.sqlite3"));
        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut policy =
            PolicyPipelineBuilder::<RuntimeContextFactory>::new_legacy_allow_fallback()
                .with_pre_policy(crate::tools::plane::ToolVisibilityPolicy)
                .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
                .with_policy(FsResolvePathAllowPolicy::target())
                .with_policy(FsResolvePathAllowPolicy::entry())
                .with_policy(FsPathAllowedRootsPolicy::target())
                .with_policy(FsPathAllowedRootsPolicy::entry())
                .with_policy(MemoryAppendTurnAllowPolicy)
                .with_policy(MemoryWindowAllowPolicy)
                .with_policy(MemoryTranscriptAllowPolicy)
                .with_policy(MemoryReplaceTurnsAllowPolicy)
                .with_policy(MemoryReadStageEnvelopeAllowPolicy)
                .with_policy(MemoryCompactAllowPolicy);
        if !tool_config.fs.deny_read_filenames.is_empty() {
            policy.push_policy(FsReadFilenameDenyPolicy::new(
                tool_config.fs.deny_read_filenames.clone(),
            ));
        }
        policy.push_policy(FsReadAllowPolicy);
        policy.push_policy(FsWriteAllowPolicy);
        policy.push_policy(FsAtomicWriteAllowPolicy);
        policy.push_policy(FsCopyFileAllowPolicy);
        policy.push_policy(FsCreateDirAllAllowPolicy);
        policy.push_policy(FsRemoveFileAllowPolicy);
        policy.push_policy(FsRemoveDirAllAllowPolicy);
        policy.push_policy(FsRenameAllowPolicy);
        policy.push_policy(FsInspectPathAllowPolicy);
        policy.push_policy(FsGlobAllowPolicy);
        policy.push_policy(FsReadDirAllowPolicy);
        policy.push_policy(FsContentSearchAllowPolicy);
        let mut kernel =
            Kernel::<RuntimeContextFactory>::with_policy_runtime(policy, clock, audit.clone());

        let pack = VerticalPackManifest {
            pack_id: crate::legacy_kernel::EMBEDDED_RUNTIME_PACK_ID.to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: capabilities.clone(),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");
        let typed_memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from(&memory_config);
        let runtime = Arc::new(Runtime::new(
            kernel,
            crate::tools::plane::builtin_tool_plane()
                .expect("builtin tool registration should succeed"),
        ));
        let tool_view =
            crate::tools::runtime_visible_tool_view(runtime.as_ref(), &tool_config, None);
        let session = Session::root(
            runtime.as_ref(),
            "test-agent",
            "test-session",
            GovernedSessionMode::MutatingCapable,
            capabilities.into_iter().collect(),
            tool_config,
            typed_memory_config,
            tool_view,
            None,
            None,
        )
        .expect("test session should be valid");

        let legacy_tools = DefaultLegacyToolDispatcher::new(
            Arc::clone(&runtime),
            &session,
            memory_config,
            crate::config::ToolConfig::default(),
        )
        .expect("legacy test fallback should derive from Session authority");

        Self {
            engine: TurnEngine::new(1),
            runtime,
            session,
            legacy_tools,
            audit,
            temp_dir,
        }
    }

    #[must_use]
    pub fn context(&self) -> Context<'_> {
        Context::new(&self.runtime, &self.session)
            .expect("test Runtime and Session should share one ownership domain")
    }

    /// Execute a provider turn through the full TurnEngine path.
    #[allow(dead_code)]
    pub async fn execute(&self, turn: &ProviderTurn) -> TurnResult {
        let session_context = self.context();
        self.engine
            .execute_turn_in_context(turn, &session_context, &self.legacy_tools, None)
            .await
    }
}

impl Drop for TurnTestHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
