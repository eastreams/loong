use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use loong_contracts::{Capability, ExecutionRoute, HarnessKind};
use loong_kernel::{
    FixedClock, InMemoryAuditSink, LoongKernel, StaticPolicyEngine, VerticalPackManifest,
};

use crate::context::KernelContext;
use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
use crate::tools::MvpToolAdapter;
use crate::tools::runtime_config::ToolRuntimeConfig;

fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
fn subprocess_lock() -> &'static Mutex<()> {
    static SUBPROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    SUBPROCESS_LOCK.get_or_init(|| Mutex::new(()))
}

pub struct ScopedEnv {
    originals: Vec<(&'static str, Option<OsString>)>,
    guard: Option<MutexGuard<'static, ()>>,
}

impl ScopedEnv {
    pub fn new() -> Self {
        let depth_before = scoped_env_depth();
        let guard = if depth_before == 0 {
            let guard = env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Some(guard)
        } else {
            None
        };
        set_scoped_env_depth(depth_before.saturating_add(1));
        Self {
            originals: Vec::new(),
            guard,
        }
    }

    #[allow(clippy::disallowed_methods)]
    pub fn set(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.capture_original(key);
        crate::process_env::set_var(key, value);
    }

    #[allow(dead_code, clippy::disallowed_methods)]
    pub fn remove(&mut self, key: &'static str) {
        self.capture_original(key);
        crate::process_env::remove_var(key);
    }

    fn capture_original(&mut self, key: &'static str) {
        if self.originals.iter().any(|(saved, _)| *saved == key) {
            return;
        }
        if default_loong_home_env_override_key(key) {
            crate::config::push_default_loong_home_env_override_for_tests();
        }
        self.originals.push((key, std::env::var_os(key)));
    }
}

impl Drop for ScopedEnv {
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        for (key, original) in self.originals.iter().rev() {
            match original {
                Some(value) => crate::process_env::set_var(key, value),
                None => crate::process_env::remove_var(key),
            }
            if default_loong_home_env_override_key(key) {
                crate::config::pop_default_loong_home_env_override_for_tests();
            }
        }

        let depth_before = scoped_env_depth();
        let depth_after = depth_before.saturating_sub(1);
        set_scoped_env_depth(depth_after);

        if depth_after == 0 {
            self.guard.take();
        }
    }
}

fn default_loong_home_env_override_key(key: &str) -> bool {
    matches!(key, "HOME" | "USERPROFILE" | "LOONG_HOME")
}

thread_local! {
    static SCOPED_ENV_DEPTH: Cell<usize> = const { Cell::new(0) };
}

fn scoped_env_depth() -> usize {
    SCOPED_ENV_DEPTH.with(|depth| depth.get())
}

fn set_scoped_env_depth(value: usize) {
    SCOPED_ENV_DEPTH.with(|depth| {
        depth.set(value);
    });
}

#[cfg(test)]
pub struct ScopedLoongHome {
    _temp_home: Option<tempfile::TempDir>,
    path: PathBuf,
}

#[cfg(test)]
impl ScopedLoongHome {
    pub fn new(prefix: &str) -> Self {
        let temp_home = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("create scoped loong home");
        let path = temp_home.path().to_path_buf();
        crate::config::push_default_loong_home_override_for_tests(path.clone());
        crate::tools::reset_runtime_home_state_for_tests();
        Self {
            _temp_home: Some(temp_home),
            path,
        }
    }

    pub fn from_existing(path: PathBuf) -> Self {
        crate::config::push_default_loong_home_override_for_tests(path.clone());
        crate::tools::reset_runtime_home_state_for_tests();
        Self {
            _temp_home: None,
            path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.path().join(relative)
    }
}

#[cfg(test)]
impl Drop for ScopedLoongHome {
    fn drop(&mut self) {
        crate::tools::reset_runtime_home_state_for_tests();
        crate::config::pop_default_loong_home_override_for_tests();
    }
}

/// Monotonic counter for unique harness IDs (avoids temp dir collisions).
static HARNESS_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static TEST_TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(crate) fn unique_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_TEMP_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()))
}

#[cfg(test)]
pub(crate) fn acquire_subprocess_test_guard() -> MutexGuard<'static, ()> {
    subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
fn current_dir_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
pub(crate) struct ScopedCurrentDir {
    original: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl ScopedCurrentDir {
    pub(crate) fn new(path: &Path) -> Self {
        let lock = current_dir_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::current_dir().expect("read current dir");
        std::env::set_current_dir(path).expect("set current dir");
        Self {
            original,
            _lock: lock,
        }
    }
}

#[cfg(test)]
impl Drop for ScopedCurrentDir {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original).expect("restore current dir");
    }
}

#[cfg(test)]
pub(crate) fn durable_memory_flush_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(all(test, unix))]
pub(crate) fn write_executable_script_atomically(
    script_path: &std::path::Path,
    contents: &str,
) -> std::io::Result<()> {
    write_executable_script_atomically_with(script_path, |file| {
        std::io::Write::write_all(file, contents.as_bytes())
    })
}

#[cfg(all(test, unix))]
fn write_executable_script_atomically_with<F>(
    script_path: &std::path::Path,
    writer: F,
) -> std::io::Result<()>
where
    F: FnOnce(&mut std::fs::File) -> std::io::Result<()>,
{
    static NEXT_STAGING_FILE_SEED: AtomicU64 = AtomicU64::new(1);

    let Some(parent) = script_path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "script path `{}` has no parent directory",
                script_path.display()
            ),
        ));
    };
    let Some(file_name) = script_path.file_name().and_then(|name| name.to_str()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "script path `{}` has no UTF-8 file name",
                script_path.display()
            ),
        ));
    };

    let seed = NEXT_STAGING_FILE_SEED.fetch_add(1, Ordering::Relaxed);
    let staged_path = parent.join(format!(".{file_name}.{}.{seed}.tmp", std::process::id()));
    let mut staged_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged_path)?;
    let write_result = writer(&mut staged_file).and_then(|()| staged_file.sync_all());
    drop(staged_file);

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&staged_path);
        return Err(error);
    }

    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(&staged_path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&staged_path, permissions)?;
    if let Err(error) = std::fs::rename(&staged_path, script_path) {
        let _ = std::fs::remove_file(&staged_path);
        return Err(error);
    }

    Ok(())
}

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
/// - An `MvpToolAdapter` with injected `ToolRuntimeConfig` (no OnceLock race)
/// - A real `InMemoryAuditSink` for audit assertions
/// - `max_tool_steps = 1`
#[allow(dead_code)]
pub struct TurnTestHarness {
    pub engine: TurnEngine,
    pub kernel_ctx: KernelContext,
    pub audit: Arc<InMemoryAuditSink>,
    pub temp_dir: PathBuf,
}

pub struct TurnTestHarnessBuilder {
    capabilities: BTreeSet<Capability>,
    tool_config_override: ToolRuntimeConfig,
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

    pub fn builder() -> TurnTestHarnessBuilder {
        TurnTestHarnessBuilder::new()
    }

    /// Construct a harness with a caller-supplied `ToolRuntimeConfig`.
    /// Use this when a test needs specific allow/deny/approval lists rather
    /// than the generic defaults.
    pub fn with_tool_config(
        capabilities: BTreeSet<Capability>,
        tool_config_override: ToolRuntimeConfig,
    ) -> Self {
        TurnTestHarnessBuilder::new()
            .capabilities(capabilities)
            .tool_config(tool_config_override)
            .build()
    }

    fn build_with(
        capabilities: BTreeSet<Capability>,
        tool_config_override: ToolRuntimeConfig,
    ) -> Self {
        let id = HARNESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir =
            std::env::temp_dir().join(format!("loong-integ-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let tool_config = build_harness_tool_config(&temp_dir, tool_config_override);
        let audit = Arc::new(InMemoryAuditSink::default());
        let kernel_ctx = build_harness_kernel_context(capabilities, tool_config, audit.clone());

        Self {
            engine: TurnEngine::new(1),
            kernel_ctx,
            audit,
            temp_dir,
        }
    }

    /// Execute a provider turn through the full TurnEngine path.
    #[allow(dead_code)]
    pub async fn execute(&self, turn: &ProviderTurn) -> TurnResult {
        self.engine.execute_turn(turn, &self.kernel_ctx).await
    }
}

impl TurnTestHarnessBuilder {
    pub fn new() -> Self {
        Self {
            capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
            tool_config_override: ToolRuntimeConfig::default(),
        }
    }

    pub fn capabilities(mut self, capabilities: BTreeSet<Capability>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn tool_config(mut self, tool_config_override: ToolRuntimeConfig) -> Self {
        self.tool_config_override = tool_config_override;
        self
    }

    pub fn build(self) -> TurnTestHarness {
        TurnTestHarness::build_with(self.capabilities, self.tool_config_override)
    }
}

fn build_harness_tool_config(
    temp_dir: &std::path::Path,
    tool_config_override: ToolRuntimeConfig,
) -> ToolRuntimeConfig {
    ToolRuntimeConfig {
        file_root: Some(temp_dir.to_path_buf()),
        config_path: Some(temp_dir.join("loong.toml")),
        ..tool_config_override
    }
}

fn build_harness_kernel_context(
    capabilities: BTreeSet<Capability>,
    tool_config: ToolRuntimeConfig,
    audit: Arc<InMemoryAuditSink>,
) -> KernelContext {
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    register_harness_pack(&mut kernel, capabilities);
    register_harness_tool_adapters(&mut kernel, &tool_config);
    register_harness_memory_adapters(&mut kernel, &tool_config);

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    KernelContext {
        kernel: Arc::new(kernel),
        token,
    }
}

fn register_harness_pack(
    kernel: &mut LoongKernel<StaticPolicyEngine>,
    capabilities: BTreeSet<Capability>,
) {
    let pack = VerticalPackManifest {
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
    };
    kernel.register_pack(pack).expect("register pack");
}

fn register_harness_tool_adapters(
    kernel: &mut LoongKernel<StaticPolicyEngine>,
    tool_config: &ToolRuntimeConfig,
) {
    kernel.register_core_tool_adapter(MvpToolAdapter::with_config(tool_config.clone()));
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .expect("set default adapter");

    kernel.register_policy_extension(
        crate::tools::shell_policy_ext::ToolPolicyExtension::from_config(tool_config),
    );
    kernel.register_policy_extension(crate::tools::file_policy_ext::FilePolicyExtension::new(
        tool_config.file_root.clone(),
    ));
}

fn register_harness_memory_adapters(
    kernel: &mut LoongKernel<StaticPolicyEngine>,
    tool_config: &ToolRuntimeConfig,
) {
    #[cfg(feature = "memory-sqlite")]
    {
        use crate::memory::runtime_config::MemoryRuntimeConfig;

        let sqlite_path = tool_config
            .config_path
            .as_ref()
            .map(|config_path| config_path.with_file_name("memory.sqlite3"))
            .expect("test harness should always set config_path");
        let memory_config = MemoryRuntimeConfig::for_sqlite_path(sqlite_path);
        kernel.register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(
            memory_config,
        ));
        kernel
            .set_default_core_memory_adapter("mvp-memory")
            .expect("set default memory adapter");
    }
}

impl Drop for TurnTestHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::{ScopedEnv, unique_temp_dir};

    #[cfg(unix)]
    use super::{write_executable_script_atomically, write_executable_script_atomically_with};

    #[test]
    fn scoped_env_recovers_after_mutex_poison() {
        let panic_result = std::thread::spawn(|| {
            let _env = ScopedEnv::new();
            panic!("poison env lock for test");
        })
        .join();

        assert!(panic_result.is_err(), "setup thread should poison the lock");

        let recovery = std::panic::catch_unwind(ScopedEnv::new);
        assert!(
            recovery.is_ok(),
            "ScopedEnv::new should recover from a poisoned env lock"
        );
    }

    #[test]
    fn scoped_env_supports_nested_guards_on_one_thread() {
        let mut outer = ScopedEnv::new();
        outer.set("LOONG_HOME", "/tmp/outer");

        let mut inner = ScopedEnv::new();
        inner.set("LOONG_HOME", "/tmp/inner");

        let inner_value = std::env::var_os("LOONG_HOME");
        assert_eq!(inner_value, Some(std::ffi::OsString::from("/tmp/inner")));

        drop(inner);

        let outer_value = std::env::var_os("LOONG_HOME");
        assert_eq!(outer_value, Some(std::ffi::OsString::from("/tmp/outer")));
    }

    #[test]
    fn unique_temp_dir_uses_distinct_paths() {
        let first = unique_temp_dir("loong-test-support");
        let second = unique_temp_dir("loong-test-support");

        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn write_executable_script_atomically_preserves_existing_script_when_write_fails() {
        let root = unique_temp_dir("loong-test-support-script-write-failure");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("fixture-script");

        write_executable_script_atomically(&script_path, "#!/bin/sh\necho old\n")
            .expect("write baseline script");

        let error = write_executable_script_atomically_with(&script_path, |_file| {
            Err(std::io::Error::other("forced write failure"))
        })
        .expect_err("failed staged write should surface an error");
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(
            std::fs::read_to_string(&script_path).expect("baseline script should remain readable"),
            "#!/bin/sh\necho old\n"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
