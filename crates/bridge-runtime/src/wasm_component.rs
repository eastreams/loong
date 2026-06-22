use std::path::{Path, PathBuf};

use loong_kernel as kernel;
use serde::Serialize;
use serde_json::{Value, json};
use wasmtime::{Engine, Instance, Module, Store};

use crate::http_json::{BridgeExecutionFailure, BridgeExecutionSuccess};
use crate::policy::BridgeExecutionPolicy;
use crate::protocol::{
    BridgeProtocolRuntimeContext, ConnectorProtocolContext, authorize_connector_protocol_context,
    serialize_runtime_evidence,
};

const WASM_COMPONENT_EXECUTOR: &str = "wasm_component_wasmtime_json_abi";
const WASM_JSON_ABI: &str = "loong_json_abi_v1";
const WASM_RESPONSE_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct WasmComponentInvocationOutcome {
    response_payload: Value,
    response_json: Value,
    module_path: String,
    request_json: Value,
    response_text: String,
}

#[derive(Debug, Clone, Serialize)]
struct WasmComponentRuntimeBase {
    executor: &'static str,
    module_path: String,
    abi: &'static str,
    max_response_bytes: usize,
    #[serde(flatten)]
    protocol: BridgeProtocolRuntimeContext,
}

#[derive(Debug, Serialize)]
struct WasmComponentRuntimeExecution {
    #[serde(flatten)]
    base: WasmComponentRuntimeBase,
    request: Value,
    response_text: String,
    response_json: Value,
}

#[derive(Debug)]
enum WasmComponentRuntimeEvidenceKind {
    BaseOnly,
    Execution {
        request: Value,
        response_text: String,
        response_json: Value,
    },
}

pub async fn execute_wasm_component_bridge_call(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &kernel::ConnectorCommand,
    runtime_policy: &BridgeExecutionPolicy,
) -> Result<BridgeExecutionSuccess, BridgeExecutionFailure> {
    if !runtime_policy.execute_wasm_component {
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason: "wasm_component execution is disabled by runtime policy".to_owned(),
            runtime_evidence: Value::Null,
        });
    }

    let mut protocol_context =
        ConnectorProtocolContext::from_connector_command(provider, channel, command);
    let authorized = authorize_connector_protocol_context(&mut protocol_context);
    if let Err(reason) = authorized {
        let runtime_evidence = wasm_component_runtime_evidence(
            &protocol_context,
            &channel.endpoint,
            WasmComponentRuntimeEvidenceKind::BaseOnly,
        );
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason: format!("wasm_component {reason}"),
            runtime_evidence,
        });
    }

    let request_payload = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let provider_for_worker = provider.clone();
    let channel_for_worker = channel.clone();
    let request_for_worker = request_payload;
    let run = tokio::task::spawn_blocking(move || {
        run_wasm_component_json_abi(
            &provider_for_worker,
            &channel_for_worker,
            request_for_worker,
        )
    })
    .await;

    match run {
        Ok(Ok(outcome)) => {
            let runtime_evidence = wasm_component_runtime_evidence(
                &protocol_context,
                &outcome.module_path,
                WasmComponentRuntimeEvidenceKind::Execution {
                    request: outcome.request_json,
                    response_text: outcome.response_text,
                    response_json: outcome.response_json,
                },
            );
            Ok(BridgeExecutionSuccess {
                response_payload: outcome.response_payload,
                runtime_evidence,
            })
        }
        Ok(Err(reason)) => {
            let runtime_evidence = wasm_component_runtime_evidence(
                &protocol_context,
                &channel.endpoint,
                WasmComponentRuntimeEvidenceKind::BaseOnly,
            );
            Err(BridgeExecutionFailure {
                blocked: false,
                reason,
                runtime_evidence,
            })
        }
        Err(error) => {
            let runtime_evidence = wasm_component_runtime_evidence(
                &protocol_context,
                &channel.endpoint,
                WasmComponentRuntimeEvidenceKind::BaseOnly,
            );
            Err(BridgeExecutionFailure {
                blocked: false,
                reason: format!("wasm_component bridge worker task failed: {error}"),
                runtime_evidence,
            })
        }
    }
}

fn run_wasm_component_json_abi(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    request_json: Value,
) -> Result<WasmComponentInvocationOutcome, String> {
    let module_path = resolve_wasm_component_module_path(provider, &channel.endpoint)?;
    let request_bytes = serde_json::to_vec(&request_json)
        .map_err(|error| format!("serialize wasm_component request failed: {error}"))?;
    let request_len = i32::try_from(request_bytes.len()).map_err(|error| {
        format!(
            "wasm_component request is too large: {} bytes: {error}",
            request_bytes.len()
        )
    })?;

    let engine = Engine::default();
    let module = load_wasm_component_module(&engine, &module_path)?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .map_err(|error| format!("instantiate wasm_component module failed: {error}"))?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| "wasm_component module must export memory".to_owned())?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "loong_alloc")
        .map_err(|error| format!("wasm_component module must export loong_alloc(len): {error}"))?;
    let invoke = instance
        .get_typed_func::<(i32, i32), i64>(&mut store, "loong_invoke")
        .map_err(|error| {
            format!("wasm_component module must export loong_invoke(ptr,len): {error}")
        })?;
    let free = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "loong_free")
        .ok();

    let request_ptr = alloc
        .call(&mut store, request_len)
        .map_err(|error| format!("wasm_component loong_alloc failed: {error}"))?;
    write_guest_memory(&mut store, &memory, request_ptr, &request_bytes)?;

    let packed_response = invoke
        .call(&mut store, (request_ptr, request_len))
        .map_err(|error| format!("wasm_component loong_invoke failed: {error}"))?;
    let (response_ptr, response_len) = unpack_wasm_response_ptr_len(packed_response);
    if response_len > WASM_RESPONSE_MAX_BYTES {
        return Err(format!(
            "wasm_component response is too large: {response_len} bytes"
        ));
    }

    let response_bytes = read_guest_memory(&mut store, &memory, response_ptr, response_len)?;
    if let Some(free) = free.as_ref() {
        free.call(&mut store, (request_ptr, request_len))
            .map_err(|error| format!("wasm_component loong_free request failed: {error}"))?;
        if response_len > 0 {
            let response_ptr_i32 = usize_to_wasm_i32(response_ptr)?;
            let response_len_i32 = i32::try_from(response_len).map_err(|error| {
                format!(
                    "wasm_component response length is out of i32 range: {response_len}: {error}"
                )
            })?;
            free.call(&mut store, (response_ptr_i32, response_len_i32))
                .map_err(|error| format!("wasm_component loong_free response failed: {error}"))?;
        }
    }

    let response_text = String::from_utf8(response_bytes)
        .map_err(|error| format!("wasm_component response is not utf-8: {error}"))?;
    let response_json = serde_json::from_str::<Value>(&response_text)
        .map_err(|error| format!("wasm_component response is not valid json: {error}"))?;
    let response_payload = extract_wasm_component_response_payload(&response_json);

    Ok(WasmComponentInvocationOutcome {
        response_payload,
        response_json,
        module_path: module_path.display().to_string(),
        request_json,
        response_text,
    })
}

fn resolve_wasm_component_module_path(
    provider: &kernel::ProviderConfig,
    endpoint: &str,
) -> Result<PathBuf, String> {
    let trimmed_endpoint = endpoint.trim();
    if trimmed_endpoint.is_empty() {
        return Err("wasm_component execution requires a non-empty channel endpoint".to_owned());
    }

    let endpoint_path = Path::new(trimmed_endpoint);
    if endpoint_path.is_absolute() {
        return Ok(endpoint_path.to_path_buf());
    }

    let package_root = provider
        .metadata
        .get("package_root")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match package_root {
        Some(package_root) => Ok(Path::new(package_root).join(endpoint_path)),
        None => Ok(endpoint_path.to_path_buf()),
    }
}

fn load_wasm_component_module(engine: &Engine, module_path: &Path) -> Result<Module, String> {
    let is_wat = module_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wat"));

    if is_wat {
        let wat_text = std::fs::read_to_string(module_path).map_err(|error| {
            format!(
                "read wasm_component wat module {} failed: {error}",
                module_path.display()
            )
        })?;
        let wasm_bytes = wat::parse_str(&wat_text).map_err(|error| {
            format!(
                "parse wasm_component wat module {} failed: {error}",
                module_path.display()
            )
        })?;
        return Module::new(engine, wasm_bytes).map_err(|error| {
            format!(
                "compile wasm_component wat module {} failed: {error}",
                module_path.display()
            )
        });
    }

    Module::from_file(engine, module_path).map_err(|error| {
        format!(
            "load wasm_component module {} failed: {error}",
            module_path.display()
        )
    })
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
        .ok_or_else(|| "wasm_component request memory range overflowed".to_owned())?;
    let memory_data = memory.data_mut(store);
    let destination = memory_data.get_mut(offset..end).ok_or_else(|| {
        format!("wasm_component request memory range {offset}..{end} is out of bounds")
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
        .ok_or_else(|| "wasm_component response memory range overflowed".to_owned())?;
    let memory_data = memory.data(store);
    let source = memory_data.get(ptr..end).ok_or_else(|| {
        format!("wasm_component response memory range {ptr}..{end} is out of bounds")
    })?;

    Ok(source.to_vec())
}

fn wasm_i32_to_usize(value: i32) -> usize {
    let raw = u32::from_ne_bytes(value.to_ne_bytes());
    raw as usize
}

fn usize_to_wasm_i32(value: usize) -> Result<i32, String> {
    let raw = u32::try_from(value).map_err(|error| {
        format!("wasm_component pointer is out of wasm32 range: {value}: {error}")
    })?;
    Ok(i32::from_ne_bytes(raw.to_ne_bytes()))
}

fn unpack_wasm_response_ptr_len(value: i64) -> (usize, usize) {
    let raw = u64::from_ne_bytes(value.to_ne_bytes());
    let ptr_raw = (raw >> 32) as u32;
    let len_raw = (raw & u64::from(u32::MAX)) as u32;

    (ptr_raw as usize, len_raw as usize)
}

fn extract_wasm_component_response_payload(response_body: &Value) -> Value {
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

fn wasm_component_runtime_evidence(
    context: &ConnectorProtocolContext,
    module_path: &str,
    evidence_kind: WasmComponentRuntimeEvidenceKind,
) -> Value {
    let base = WasmComponentRuntimeBase {
        executor: WASM_COMPONENT_EXECUTOR,
        module_path: module_path.to_owned(),
        abi: WASM_JSON_ABI,
        max_response_bytes: WASM_RESPONSE_MAX_BYTES,
        protocol: BridgeProtocolRuntimeContext::from_connector_context(context),
    };

    match evidence_kind {
        WasmComponentRuntimeEvidenceKind::BaseOnly => {
            serialize_runtime_evidence(WASM_COMPONENT_EXECUTOR, &base)
        }
        WasmComponentRuntimeEvidenceKind::Execution {
            request,
            response_text,
            response_json,
        } => {
            let runtime = WasmComponentRuntimeExecution {
                base,
                request,
                response_text,
                response_json,
            };
            serialize_runtime_evidence(WASM_COMPONENT_EXECUTOR, &runtime)
        }
    }
}
