use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loong_contracts::ToolCoreOutcome;
use loong_kernel::{
    BridgeSupportMatrix, PluginActivationStatus, PluginScanReport, PluginScanner,
    PluginSetupReadinessContext, PluginTranslationReport, PluginTranslator,
};
use serde::Serialize;
use serde_json::{Value, json};
use wasmtime::{Engine, Instance, Module, Store};

use crate::config::ToolConfig;

const DEFAULT_PLUGIN_ID: &str = "demo-wasm-bridge";
const DEFAULT_OPERATION: &str = "demo";
const DEFAULT_RUNTIME_PLUGIN_ROOT: &str = "runtime-plugins";
const WASM_EXECUTOR: &str = "app_runtime_plugin_wasmtime_json_abi";
const WASM_JSON_ABI: &str = "loong_json_abi_v1";
const WASM_RESPONSE_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct WasmInvocationOutcome {
    response_payload: Value,
    response_json: Value,
    response_text: String,
    module_path: String,
    request_json: Value,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimePluginRuntimeEvidence {
    executor: &'static str,
    plugin_id: String,
    module_path: String,
    abi: &'static str,
    operation: String,
    request: Value,
    response_text: String,
    response_json: Value,
}

pub(crate) fn execute_runtime_plugin_tool_with_config(
    request: loong_contracts::ToolCoreRequest,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "plugin payload must be an object".to_owned())?;
    let plugin_id = payload
        .get("plugin_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PLUGIN_ID)
        .to_owned();
    let operation = payload
        .get("operation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OPERATION)
        .to_owned();
    let request_payload = payload.get("payload").cloned().unwrap_or_else(|| json!({}));
    let scan_root = resolve_runtime_plugin_scan_root(tool_config, payload.get("root"))?;
    let (scan_report, translation, activation) = scan_runtime_plugins(&scan_root)?;
    let selected = select_runtime_plugin(&translation, &activation, plugin_id.as_str())?;

    let bridge_kind = selected.runtime.bridge_kind;
    if bridge_kind != loong_kernel::PluginBridgeKind::WasmComponent {
        return Err(format!(
            "plugin `{}` uses unsupported bridge kind `{}` for TUI plugin tool; expected `wasm_component`",
            selected.plugin_id,
            bridge_kind.as_str()
        ));
    }

    let endpoint = resolve_plugin_endpoint(selected)?;
    let request_json = json!({
        "plugin_id": selected.plugin_id,
        "provider_id": selected.provider_id,
        "channel_id": selected.channel_id,
        "operation": operation,
        "payload": request_payload,
    });
    let invocation = execute_wasm_runtime_plugin(selected, endpoint.as_path(), request_json)?;
    let runtime = RuntimePluginRuntimeEvidence {
        executor: WASM_EXECUTOR,
        plugin_id: selected.plugin_id.clone(),
        module_path: invocation.module_path,
        abi: WASM_JSON_ABI,
        operation,
        request: invocation.request_json,
        response_text: invocation.response_text,
        response_json: invocation.response_json,
    };

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "plugin_id": selected.plugin_id,
            "provider_id": selected.provider_id,
            "channel_id": selected.channel_id,
            "bridge_kind": bridge_kind.as_str(),
            "source_path": selected.source_path,
            "package_root": selected.package_root,
            "scan_root": scan_root.display().to_string(),
            "scan_summary": {
                "scanned_files": scan_report.scanned_files,
                "matched_plugins": scan_report.matched_plugins,
                "translated_plugins": translation.translated_plugins,
                "ready_plugins": activation.ready_plugins,
                "blocked_plugins": activation.blocked_plugins,
            },
            "result": invocation.response_payload,
            "runtime": serde_json::to_value(runtime)
                .map_err(|error| format!("serialize plugin runtime evidence failed: {error}"))?,
        }),
    })
}

fn resolve_runtime_plugin_scan_root(
    tool_config: &ToolConfig,
    raw_root: Option<&Value>,
) -> Result<PathBuf, String> {
    let base_root = tool_config
        .configured_runtime_workspace_root()
        .or_else(|| tool_config.configured_file_root())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let explicit_root = raw_root
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);

    let root = match explicit_root {
        Some(root) if root.is_absolute() => root,
        Some(root) => base_root.join(root),
        None => base_root.join(DEFAULT_RUNTIME_PLUGIN_ROOT),
    };

    Ok(dunce::canonicalize(&root).unwrap_or(root))
}

fn scan_runtime_plugins(
    scan_root: &Path,
) -> Result<
    (
        PluginScanReport,
        PluginTranslationReport,
        loong_kernel::PluginActivationPlan,
    ),
    String,
> {
    let scanner = PluginScanner::new();
    let scan_report = scanner.scan_path(scan_root).map_err(|error| {
        format!(
            "runtime plugin scan failed for {}: {error}",
            scan_root.display()
        )
    })?;
    let translator = PluginTranslator::new();
    let translation = translator.translate_scan_report(&scan_report);
    let readiness_context = plugin_setup_readiness_context();
    let activation = translator.plan_activation(
        &translation,
        &BridgeSupportMatrix::default(),
        &readiness_context,
    );

    Ok((scan_report, translation, activation))
}

fn plugin_setup_readiness_context() -> PluginSetupReadinessContext {
    let verified_env_vars = std::env::vars_os()
        .filter_map(|(key, value)| {
            let value = value.to_string_lossy();
            let trimmed_value = value.trim();
            if trimmed_value.is_empty() {
                return None;
            }
            Some(key.to_string_lossy().to_string())
        })
        .collect::<BTreeSet<_>>();

    PluginSetupReadinessContext {
        verified_env_vars,
        verified_config_keys: BTreeSet::new(),
    }
}

fn select_runtime_plugin<'a>(
    translation: &'a PluginTranslationReport,
    activation: &'a loong_kernel::PluginActivationPlan,
    plugin_id: &str,
) -> Result<&'a loong_kernel::PluginIR, String> {
    let matches = translation
        .entries
        .iter()
        .filter(|entry| entry.plugin_id == plugin_id)
        .collect::<Vec<_>>();

    if matches.is_empty() {
        let available = translation
            .entries
            .iter()
            .map(|entry| entry.plugin_id.clone())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "runtime plugin `{plugin_id}` was not found; available plugins: {available}"
        ));
    }

    if matches.len() > 1 {
        return Err(format!(
            "runtime plugin `{plugin_id}` is ambiguous across scan roots"
        ));
    }

    let selected = matches
        .into_iter()
        .next()
        .ok_or_else(|| format!("runtime plugin `{plugin_id}` selection unexpectedly failed"))?;
    let candidate = activation
        .candidate_for(&selected.source_path, &selected.plugin_id)
        .ok_or_else(|| {
            format!(
                "runtime plugin `{plugin_id}` is missing activation metadata for {}",
                selected.source_path
            )
        })?;
    if candidate.status != PluginActivationStatus::Ready {
        return Err(format!(
            "runtime plugin `{plugin_id}` is not execution-ready: {} ({})",
            candidate.status.as_str(),
            candidate.reason
        ));
    }

    Ok(selected)
}

fn resolve_plugin_endpoint(plugin: &loong_kernel::PluginIR) -> Result<PathBuf, String> {
    let endpoint = plugin
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "runtime plugin `{}` is missing manifest endpoint",
                plugin.plugin_id
            )
        })?;
    let endpoint_path = Path::new(endpoint);
    if endpoint_path.is_absolute() {
        return Ok(endpoint_path.to_path_buf());
    }

    Ok(Path::new(&plugin.package_root).join(endpoint_path))
}

fn execute_wasm_runtime_plugin(
    plugin: &loong_kernel::PluginIR,
    endpoint: &Path,
    request_json: Value,
) -> Result<WasmInvocationOutcome, String> {
    let request_bytes = serde_json::to_vec(&request_json)
        .map_err(|error| format!("serialize plugin request failed: {error}"))?;
    let request_len = i32::try_from(request_bytes.len()).map_err(|error| {
        format!(
            "plugin request is too large: {} bytes: {error}",
            request_bytes.len()
        )
    })?;

    let engine = Engine::default();
    let module = load_wasm_module(&engine, endpoint)?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).map_err(|error| {
        format!(
            "instantiate runtime plugin `{}` failed: {error}",
            plugin.plugin_id
        )
    })?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| format!("runtime plugin `{}` must export memory", plugin.plugin_id))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "loong_alloc")
        .map_err(|error| {
            format!(
                "runtime plugin `{}` must export loong_alloc(len): {error}",
                plugin.plugin_id
            )
        })?;
    let invoke = instance
        .get_typed_func::<(i32, i32), i64>(&mut store, "loong_invoke")
        .map_err(|error| {
            format!(
                "runtime plugin `{}` must export loong_invoke(ptr,len): {error}",
                plugin.plugin_id
            )
        })?;
    let free = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "loong_free")
        .ok();

    let request_ptr = alloc.call(&mut store, request_len).map_err(|error| {
        format!(
            "runtime plugin `{}` loong_alloc failed: {error}",
            plugin.plugin_id
        )
    })?;
    write_guest_memory(&mut store, &memory, request_ptr, &request_bytes)?;

    let packed_response = invoke
        .call(&mut store, (request_ptr, request_len))
        .map_err(|error| {
            format!(
                "runtime plugin `{}` loong_invoke failed: {error}",
                plugin.plugin_id
            )
        })?;
    let (response_ptr, response_len) = unpack_wasm_response_ptr_len(packed_response);
    if response_len > WASM_RESPONSE_MAX_BYTES {
        return Err(format!(
            "runtime plugin `{}` response is too large: {response_len} bytes",
            plugin.plugin_id
        ));
    }

    let response_bytes = read_guest_memory(&mut store, &memory, response_ptr, response_len)?;
    if let Some(free) = free.as_ref() {
        free.call(&mut store, (request_ptr, request_len))
            .map_err(|error| {
                format!(
                    "runtime plugin `{}` loong_free request failed: {error}",
                    plugin.plugin_id
                )
            })?;
        if response_len > 0 {
            let response_ptr_i32 = usize_to_wasm_i32(response_ptr)?;
            let response_len_i32 = i32::try_from(response_len).map_err(|error| {
                format!(
                    "runtime plugin `{}` response length is out of i32 range: {response_len}: {error}",
                    plugin.plugin_id
                )
            })?;
            free.call(&mut store, (response_ptr_i32, response_len_i32))
                .map_err(|error| {
                    format!(
                        "runtime plugin `{}` loong_free response failed: {error}",
                        plugin.plugin_id
                    )
                })?;
        }
    }

    let response_text = String::from_utf8(response_bytes).map_err(|error| {
        format!(
            "runtime plugin `{}` response is not utf-8: {error}",
            plugin.plugin_id
        )
    })?;
    let response_json = serde_json::from_str::<Value>(&response_text).map_err(|error| {
        format!(
            "runtime plugin `{}` response is not valid json: {error}",
            plugin.plugin_id
        )
    })?;
    let response_payload = extract_response_payload(&response_json);

    Ok(WasmInvocationOutcome {
        response_payload,
        response_json,
        response_text,
        module_path: endpoint.display().to_string(),
        request_json,
    })
}

fn load_wasm_module(engine: &Engine, module_path: &Path) -> Result<Module, String> {
    let is_wat = module_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wat"));

    if is_wat {
        let wat_text = std::fs::read_to_string(module_path).map_err(|error| {
            format!("read WAT module {} failed: {error}", module_path.display())
        })?;
        let wasm_bytes = wat::parse_str(&wat_text).map_err(|error| {
            format!("parse WAT module {} failed: {error}", module_path.display())
        })?;
        return Module::new(engine, wasm_bytes).map_err(|error| {
            format!(
                "compile WAT module {} failed: {error}",
                module_path.display()
            )
        });
    }

    Module::from_file(engine, module_path)
        .map_err(|error| format!("load WASM module {} failed: {error}", module_path.display()))
}

fn write_guest_memory(
    store: &mut Store<()>,
    memory: &wasmtime::Memory,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), String> {
    let offset = wasm_i32_to_usize(ptr);
    let end = offset
        .checked_add(bytes.len())
        .ok_or_else(|| "runtime plugin request memory range overflowed".to_owned())?;
    let memory_data = memory.data_mut(store);
    let destination = memory_data.get_mut(offset..end).ok_or_else(|| {
        format!("runtime plugin request memory range {offset}..{end} is out of bounds")
    })?;
    destination.copy_from_slice(bytes);
    Ok(())
}

fn read_guest_memory(
    store: &mut Store<()>,
    memory: &wasmtime::Memory,
    ptr: usize,
    len: usize,
) -> Result<Vec<u8>, String> {
    let end = ptr
        .checked_add(len)
        .ok_or_else(|| "runtime plugin response memory range overflowed".to_owned())?;
    let memory_data = memory.data(store);
    let source = memory_data.get(ptr..end).ok_or_else(|| {
        format!("runtime plugin response memory range {ptr}..{end} is out of bounds")
    })?;
    Ok(source.to_vec())
}

fn wasm_i32_to_usize(value: i32) -> usize {
    let raw = u32::from_ne_bytes(value.to_ne_bytes());
    raw as usize
}

fn usize_to_wasm_i32(value: usize) -> Result<i32, String> {
    let raw = u32::try_from(value).map_err(|error| {
        format!("runtime plugin pointer is out of wasm32 range: {value}: {error}")
    })?;
    Ok(i32::from_ne_bytes(raw.to_ne_bytes()))
}

fn unpack_wasm_response_ptr_len(value: i64) -> (usize, usize) {
    let raw = u64::from_ne_bytes(value.to_ne_bytes());
    let ptr_raw = (raw >> 32) as u32;
    let len_raw = (raw & u64::from(u32::MAX)) as u32;
    (ptr_raw as usize, len_raw as usize)
}

fn extract_response_payload(response_body: &Value) -> Value {
    let response_object = response_body.as_object();
    let Some(response_object) = response_object else {
        return response_body.clone();
    };
    let payload = response_object.get("payload");
    let Some(payload) = payload else {
        return response_body.clone();
    };
    payload.clone()
}
