use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use loong_contracts::{Capability, ExecutionRoute, HarnessKind};
use loong_kernel::{
    FixedClock, InMemoryAuditSink, Kernel, PolicyPipeline, VerticalPackManifest,
    policy::{FsReadAllowPolicy, FsReadFilenameDenyPolicy, FsResolvePathAllowedRootsPolicy},
};

use crate::context::{AppContextFactory, KernelContext};
use crate::conversation::{
    ConversationRuntimeBinding, DefaultAppToolDispatcher, ProviderTurn, SessionContext, ToolIntent,
    TurnEngine, TurnResult,
};
use crate::session::store::SessionStoreConfig;
use crate::tools::{ToolView, runtime_config::ToolRuntimeConfig};

fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

pub fn lock_process_env_for_tests() -> MutexGuard<'static, ()> {
    env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
                    tool_name: bridged_name,
                    args_json: bridged_args,
                    source: "fake_provider".to_owned(),
                    session_id: "test-session".to_owned(),
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
/// - A `KernelToolAdapter` with injected `ToolRuntimeConfig` (no OnceLock race)
/// - A real `InMemoryAuditSink` for audit assertions
/// - `max_tool_steps = 1`
#[allow(dead_code)]
pub struct TurnTestHarness {
    pub engine: TurnEngine,
    pub kernel_ctx: KernelContext,
    pub audit: Arc<InMemoryAuditSink>,
    pub temp_dir: PathBuf,
    memory_config: SessionStoreConfig,
    tool_view: ToolView,
}

impl TurnTestHarness {
    pub fn new() -> Self {
        Self::with_capabilities(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
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
        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&tool_config);

        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut policy = PolicyPipeline::<AppContextFactory>::new_legacy_allow_fallback()
            .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
            .with_policy(FsResolvePathAllowedRootsPolicy);
        if !tool_config.fs.deny_read_filenames.is_empty() {
            policy.push_policy(FsReadFilenameDenyPolicy::new(
                tool_config.fs.deny_read_filenames.clone(),
            ));
        }
        policy.push_policy(FsReadAllowPolicy);
        let mut kernel =
            Kernel::<AppContextFactory>::with_policy_runtime(policy, clock, audit.clone());

        let pack = Arc::new(VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: capabilities,
            metadata: BTreeMap::new(),
        });
        kernel
            .register_pack((*pack).clone())
            .expect("register pack");
        crate::tools::register_kernel_tools(
            &mut kernel,
            tool_config.clone(),
            crate::config::ObservabilityConfig::runtime_default(),
        )
        .expect("register kernel tools");

        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config =
                crate::memory::runtime_config::MemoryRuntimeConfig::from(&memory_config);
            kernel.register_core_memory_adapter(crate::memory::KernelMemoryAdapter::with_config(
                memory_config,
            ));
            kernel
                .set_default_core_memory_adapter("mvp-memory")
                .expect("set default memory adapter");
        }

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let ctx = KernelContext {
            kernel: Arc::new(kernel),
            pack,
            token,
            tool_runtime_config: tool_config,
        };

        Self {
            engine: TurnEngine::new(1),
            kernel_ctx: ctx,
            audit,
            temp_dir,
            memory_config,
            tool_view,
        }
    }

    /// Execute a provider turn through the full TurnEngine path.
    #[allow(dead_code)]
    pub async fn execute(&self, turn: &ProviderTurn) -> TurnResult {
        let session_context =
            SessionContext::root_with_tool_view("test-session", self.tool_view.clone());
        let dispatcher = DefaultAppToolDispatcher::new(
            self.memory_config.clone(),
            crate::config::ToolConfig::default(),
        );
        self.engine
            .execute_turn_in_context(
                turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&self.kernel_ctx),
                None,
            )
            .await
    }
}

impl Drop for TurnTestHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}
