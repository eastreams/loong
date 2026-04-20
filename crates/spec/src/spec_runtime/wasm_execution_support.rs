use super::*;

pub(super) fn wasm_artifact_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{byte:02x}"));
    }
    out
}

#[derive(Debug)]
pub(super) struct WasmArtifactBytes {
    pub(super) bytes: Vec<u8>,
    pub(super) modified_unix_ns: Option<u128>,
    pub(super) file_identity: Option<WasmArtifactFileIdentity>,
}

pub(super) fn read_wasm_artifact_bytes(artifact_path: &Path) -> Result<WasmArtifactBytes, String> {
    let mut artifact_file = fs::File::open(artifact_path)
        .map_err(|error| format!("failed to open wasm artifact: {error}"))?;
    let artifact_metadata = artifact_file
        .metadata()
        .map_err(|error| format!("failed to read wasm artifact metadata: {error}"))?;
    if !artifact_metadata.file_type().is_file() {
        return Err("wasm artifact path must reference a regular file".to_owned());
    }

    let expected_size = artifact_metadata.len().min(8 * 1024 * 1024_u64) as usize;
    let mut bytes = Vec::with_capacity(expected_size);
    artifact_file
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read wasm artifact: {error}"))?;

    Ok(WasmArtifactBytes {
        bytes,
        modified_unix_ns: modified_unix_nanos(&artifact_metadata),
        file_identity: wasm_artifact_file_identity(&artifact_metadata),
    })
}

pub(super) fn compile_wasm_module(
    module_bytes: &[u8],
    fuel_enabled: bool,
    epoch_interruption_enabled: bool,
    artifact_sha256: Option<String>,
) -> Result<CachedWasmModule, String> {
    let mut config = WasmtimeConfig::new();
    // On macOS, default to `false` because Wasmtime's signal-based trap path
    // relies on a global machports handler thread, which has shown intermittent
    // aborts under highly parallel bridge tests.
    config.signals_based_traps(wasm_signals_based_traps_enabled_from_env());
    if fuel_enabled {
        config.consume_fuel(true);
    }
    if epoch_interruption_enabled {
        config.epoch_interruption(true);
    }
    let engine = WasmtimeEngine::new(&config)
        .map_err(|error| format!("failed to initialize wasmtime engine: {error}"))?;
    let module = WasmtimeModule::new(&engine, module_bytes)
        .map_err(|error| format!("failed to compile wasm module: {error}"))?;
    Ok(CachedWasmModule {
        engine,
        module,
        artifact_sha256,
    })
}

#[derive(Debug)]
pub(super) struct WasmEpochDeadlineController {
    cancel_tx: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl WasmEpochDeadlineController {
    pub(super) fn start(engine: &WasmtimeEngine, timeout_ms: u64) -> Result<Self, String> {
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
        let timeout = Duration::from_millis(timeout_ms);
        let engine = engine.clone();
        let thread_name = "loong-wasm-timeout".to_owned();
        let worker = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let wait_result = cancel_rx.recv_timeout(timeout);
                let timed_out = matches!(wait_result, Err(mpsc::RecvTimeoutError::Timeout));
                if timed_out {
                    engine.increment_epoch();
                }
            })
            .map_err(|error| format!("failed to start wasm timeout watchdog: {error}"))?;
        Ok(Self {
            cancel_tx: Some(cancel_tx),
            worker: Some(worker),
        })
    }

    fn disarm(&mut self) {
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for WasmEpochDeadlineController {
    fn drop(&mut self) {
        self.disarm();
    }
}

pub(super) fn wasm_runtime_failure_reason(
    error: &wasmtime::Error,
    timeout_ms: Option<u64>,
    context: &str,
) -> (String, bool) {
    let is_interrupt_trap = matches!(
        error.downcast_ref::<WasmtimeTrap>(),
        Some(trap) if *trap == WasmtimeTrap::Interrupt
    );

    let Some(timeout_ms) = timeout_ms else {
        return (format!("{context}: {error}"), false);
    };
    if !is_interrupt_trap {
        return (format!("{context}: {error}"), false);
    }

    let timeout_reason = format!("wasm execution timed out after {timeout_ms}ms");
    (timeout_reason, true)
}

pub(super) fn wasm_cache_lookup_disabled(
    cache_capacity: usize,
    cache_max_bytes: usize,
) -> WasmModuleCacheLookup {
    WasmModuleCacheLookup {
        hit: false,
        inserted: false,
        evicted_entries: 0,
        cache_len: 0,
        cache_capacity,
        cache_total_module_bytes: 0,
        cache_max_bytes,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum WasmEntrypointSignature {
    I32,
    Unit,
}

impl WasmEntrypointSignature {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::I32 => "() -> i32",
            Self::Unit => "() -> ()",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct WasmRunEvidence {
    pub(super) entrypoint_signature: Option<&'static str>,
    pub(super) guest_exit_code: Option<i32>,
    pub(super) host_abi: WasmHostAbiSnapshot,
}

#[derive(Debug, Clone)]
pub(super) struct WasmRuntimeExecutionContext {
    pub(super) artifact_path: String,
    pub(super) export_name: String,
    pub(super) operation: String,
    pub(super) payload: Value,
    pub(super) request: Value,
    pub(super) module_size_bytes: usize,
    pub(super) fuel_limit: Option<u64>,
    pub(super) max_output_bytes: Option<usize>,
    pub(super) timeout_ms: Option<u64>,
    pub(super) cache_enabled: bool,
    pub(super) cache_lookup: WasmModuleCacheLookup,
    pub(super) cache_miss: bool,
    pub(super) expected_sha256: Option<String>,
    pub(super) artifact_sha256: Option<String>,
}

#[derive(Debug)]
pub(super) struct WasmRunOutcome {
    pub(super) consumed_fuel: Option<u64>,
    pub(super) timeout_triggered: bool,
    pub(super) evidence: WasmRunEvidence,
}

#[derive(Debug)]
pub(super) struct WasmRunFailure {
    pub(super) reason: String,
    pub(super) timeout_triggered: bool,
    pub(super) consumed_fuel: Option<u64>,
    pub(super) evidence: WasmRunEvidence,
}

pub(super) type WasmRunResult<T> = Result<T, Box<WasmRunFailure>>;

pub(super) fn boxed_wasm_run_failure(
    reason: impl Into<String>,
    timeout_triggered: bool,
    consumed_fuel: Option<u64>,
    evidence: WasmRunEvidence,
) -> Box<WasmRunFailure> {
    Box::new(WasmRunFailure {
        reason: reason.into(),
        timeout_triggered,
        consumed_fuel,
        evidence,
    })
}

pub(super) fn wasm_bridge_request_payload(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
) -> Value {
    json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    })
}

pub(super) fn wasm_runtime_execution_evidence(
    context: &WasmRuntimeExecutionContext,
    timeout_triggered: bool,
    fuel_consumed: Option<u64>,
    evidence: &WasmRunEvidence,
) -> Value {
    let expected_sha256 = context.expected_sha256.clone();
    let artifact_sha256 = context.artifact_sha256.clone();
    let integrity_check_required = expected_sha256.is_some();
    let integrity_check_passed = expected_sha256.is_none() || artifact_sha256.is_some();

    json!({
        "executor": "wasmtime_module",
        "artifact_path": context.artifact_path,
        "export": context.export_name,
        "operation": context.operation,
        "payload": context.payload,
        "request": context.request,
        "module_size_bytes": context.module_size_bytes,
        "fuel_limit": context.fuel_limit,
        "max_output_bytes": evidence.host_abi.max_output_bytes.or(context.max_output_bytes),
        "timeout_ms": context.timeout_ms,
        "timeout_triggered": timeout_triggered,
        "fuel_consumed": fuel_consumed,
        "cache_enabled": context.cache_enabled,
        "cache_hit": context.cache_lookup.hit,
        "cache_miss": context.cache_miss,
        "cache_evicted_entries": context.cache_lookup.evicted_entries,
        "cache_entries": context.cache_lookup.cache_len,
        "cache_capacity": context.cache_lookup.cache_capacity,
        "cache_total_module_bytes": context.cache_lookup.cache_total_module_bytes,
        "cache_max_bytes": context.cache_lookup.cache_max_bytes,
        "cache_inserted": context.cache_lookup.inserted,
        "expected_sha256": expected_sha256,
        "artifact_sha256": artifact_sha256,
        "integrity_check_required": integrity_check_required,
        "integrity_check_passed": integrity_check_passed,
        "host_abi_enabled": evidence.host_abi.host_abi_enabled,
        "entrypoint_signature": evidence.entrypoint_signature,
        "guest_exit_code": evidence.guest_exit_code,
        "guest_logs": evidence.host_abi.guest_logs,
        "guest_logs_truncated": evidence.host_abi.guest_logs_truncated,
        "output_text": evidence.host_abi.output_text,
        "output_json": evidence.host_abi.output_json,
    })
}

pub(super) fn wasm_snapshot_from_store(
    store: &WasmtimeStore<WasmHostAbiStoreData>,
    host_abi_enabled: bool,
) -> WasmHostAbiSnapshot {
    store.data().snapshot(host_abi_enabled)
}
