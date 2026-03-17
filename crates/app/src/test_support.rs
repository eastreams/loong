use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind};
use loongclaw_kernel::{
    FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine, VerticalPackManifest,
};

use crate::context::KernelContext;
use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
use crate::tools::MvpToolAdapter;
use crate::tools::runtime_config::ToolRuntimeConfig;

fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

pub struct ScopedEnv {
    originals: Vec<(&'static str, Option<OsString>)>,
    _guard: MutexGuard<'static, ()>,
}

impl ScopedEnv {
    pub fn new() -> Self {
        let guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            originals: Vec::new(),
            _guard: guard,
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
        }
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
            std::env::temp_dir().join(format!("loongclaw-integ-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        // Merge the caller's overrides with the unique temp dir as file_root.
        let tool_config = ToolRuntimeConfig {
            file_root: Some(temp_dir.clone()),
            config_path: Some(temp_dir.join("loongclaw.toml")),
            ..tool_config_override
        };

        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel =
            LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

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
        kernel.register_core_tool_adapter(MvpToolAdapter::with_config(tool_config.clone()));
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default adapter");

        // Register policy extensions for unified security enforcement.
        // Policy rules come exclusively from the runtime config; no hardcoded
        // lists are injected here.
        kernel.register_policy_extension(
            crate::tools::shell_policy_ext::ToolPolicyExtension::from_config(&tool_config),
        );
        kernel.register_policy_extension(crate::tools::file_policy_ext::FilePolicyExtension::new(
            tool_config.file_root,
        ));

        #[cfg(feature = "memory-sqlite")]
        {
            use crate::memory::runtime_config::MemoryRuntimeConfig;
            let memory_config = MemoryRuntimeConfig {
                sqlite_path: Some(temp_dir.join("memory.sqlite3")),
                ..MemoryRuntimeConfig::default()
            };
            kernel.register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(
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
            token,
        };

        Self {
            engine: TurnEngine::new(1),
            kernel_ctx: ctx,
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
    fn unique_temp_dir_uses_distinct_paths() {
        let first = unique_temp_dir("loongclaw-test-support");
        let second = unique_temp_dir("loongclaw-test-support");

        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn write_executable_script_atomically_preserves_existing_script_when_write_fails() {
        let root = unique_temp_dir("loongclaw-test-support-script-write-failure");
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
