use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use kernel::{ChannelConfig, ProviderConfig};
use serde_json::Value;
use wasmtime::{
    Caller, Extern, Linker as WasmtimeLinker, Module as WasmtimeModule, Result as WasmtimeResult,
};

const WASM_HOST_ABI_IMPORT_MODULE: &str = "loongclaw";
const WASM_HOST_ABI_IMPORT_INPUT_LEN: &str = "input_len";
const WASM_HOST_ABI_IMPORT_READ_INPUT: &str = "read_input";
const WASM_HOST_ABI_IMPORT_CONFIG_LEN: &str = "config_len";
const WASM_HOST_ABI_IMPORT_READ_CONFIG: &str = "read_config";
const WASM_HOST_ABI_IMPORT_WRITE_OUTPUT: &str = "write_output";
const WASM_HOST_ABI_IMPORT_LOG: &str = "log";
const WASM_HOST_ABI_IMPORT_ABORT: &str = "abort";
const WASM_HOST_ABI_ERROR_CODE: i32 = -1;
const WASM_HOST_ABI_LOG_DROPPED_CODE: i32 = 0;
const WASM_HOST_ABI_CONFIG_UNAVAILABLE_CODE: i32 = -2;
const WASM_HOST_ABI_BUFFER_TOO_SMALL_CODE: i32 = -3;
const DEFAULT_WASM_HOST_ABI_MAX_OUTPUT_BYTES: usize = 256 * 1024;
const WASM_HOST_ABI_MAX_CONFIG_KEY_BYTES: usize = 1024;
const WASM_HOST_ABI_MAX_LOG_ENTRY_BYTES: usize = 4 * 1024;
const WASM_HOST_ABI_MAX_LOG_TOTAL_BYTES: usize = 16 * 1024;
const WASM_HOST_ABI_MAX_LOG_ENTRIES: usize = 64;

pub(crate) const WASM_GUEST_CONFIG_PROVIDER_PREFIX: &str = "provider.";
pub(crate) const WASM_GUEST_CONFIG_CHANNEL_PREFIX: &str = "channel.";

#[derive(Debug, Clone, Default)]
pub(super) struct WasmHostAbiSnapshot {
    pub(super) host_abi_enabled: bool,
    pub(super) guest_logs: Vec<String>,
    pub(super) guest_logs_truncated: bool,
    pub(super) max_output_bytes: Option<usize>,
    pub(super) output_text: Option<String>,
    pub(super) output_json: Option<Value>,
}

#[derive(Debug, Clone)]
pub(super) struct WasmHostAbiStoreData {
    input_bytes: Vec<u8>,
    guest_config: BTreeMap<String, Vec<u8>>,
    guest_logs: Vec<String>,
    guest_logs_bytes: usize,
    guest_logs_truncated: bool,
    max_output_bytes: usize,
    output_bytes: Option<Vec<u8>>,
    pub(super) output_error: Option<String>,
    pub(super) abort_code: Option<i32>,
}

impl WasmHostAbiStoreData {
    pub(super) fn try_new(
        input_bytes: Vec<u8>,
        guest_config: BTreeMap<String, Vec<u8>>,
        max_output_bytes: Option<usize>,
    ) -> Result<Self, String> {
        let input_len = input_bytes.len();
        let max_len = i32::MAX as usize;
        if input_len > max_len {
            return Err(format!(
                "wasm host ABI input exceeds supported length: {input_len} bytes"
            ));
        }

        for (key, value_bytes) in &guest_config {
            let value_len = value_bytes.len();
            if value_len > max_len {
                return Err(format!(
                    "wasm guest config value exceeds supported length for key `{key}`: {value_len} bytes"
                ));
            }
        }

        let resolved_max_output_bytes = resolve_wasm_host_abi_max_output_bytes(max_output_bytes)?;

        Ok(Self {
            input_bytes,
            guest_config,
            guest_logs: Vec::new(),
            guest_logs_bytes: 0,
            guest_logs_truncated: false,
            max_output_bytes: resolved_max_output_bytes,
            output_bytes: None,
            output_error: None,
            abort_code: None,
        })
    }

    pub(super) fn snapshot(&self, host_abi_enabled: bool) -> WasmHostAbiSnapshot {
        let output_text = self
            .output_bytes
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());

        WasmHostAbiSnapshot {
            host_abi_enabled,
            guest_logs: self.guest_logs.clone(),
            guest_logs_truncated: self.guest_logs_truncated,
            max_output_bytes: Some(self.max_output_bytes),
            output_text,
            output_json: None,
        }
    }

    pub(super) fn parse_output_json(&self) -> Result<Option<Value>, String> {
        let Some(output_bytes) = self.output_bytes.as_ref() else {
            return Ok(None);
        };

        let output_json = serde_json::from_slice::<Value>(output_bytes)
            .map_err(|error| format!("wasm guest output is not valid JSON: {error}"))?;

        Ok(Some(output_json))
    }
}

pub(super) fn module_uses_wasm_host_abi(module: &WasmtimeModule) -> bool {
    for import in module.imports() {
        if import.module() == WASM_HOST_ABI_IMPORT_MODULE {
            return true;
        }
    }

    false
}

pub(super) fn module_requires_wasm_host_abi_memory(module: &WasmtimeModule) -> bool {
    for import in module.imports() {
        let import_module = import.module();
        if import_module != WASM_HOST_ABI_IMPORT_MODULE {
            continue;
        }

        let import_name = import.name();
        if wasm_host_abi_import_uses_guest_memory(import_name) {
            return true;
        }
    }

    false
}

pub(super) fn link_wasm_host_abi(
    linker: &mut WasmtimeLinker<WasmHostAbiStoreData>,
) -> Result<(), String> {
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_INPUT_LEN,
            wasm_host_input_len,
        )
        .map_err(|error| format!("failed to define wasm host function input_len: {error}"))?;
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_READ_INPUT,
            wasm_host_read_input,
        )
        .map_err(|error| format!("failed to define wasm host function read_input: {error}"))?;
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_CONFIG_LEN,
            wasm_host_config_len,
        )
        .map_err(|error| format!("failed to define wasm host function config_len: {error}"))?;
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_READ_CONFIG,
            wasm_host_read_config,
        )
        .map_err(|error| format!("failed to define wasm host function read_config: {error}"))?;
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_WRITE_OUTPUT,
            wasm_host_write_output,
        )
        .map_err(|error| format!("failed to define wasm host function write_output: {error}"))?;
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_LOG,
            wasm_host_log,
        )
        .map_err(|error| format!("failed to define wasm host function log: {error}"))?;
    linker
        .func_wrap(
            WASM_HOST_ABI_IMPORT_MODULE,
            WASM_HOST_ABI_IMPORT_ABORT,
            wasm_host_abort,
        )
        .map_err(|error| format!("failed to define wasm host function abort: {error}"))?;

    Ok(())
}

fn wasm_host_abi_import_uses_guest_memory(import_name: &str) -> bool {
    matches!(
        import_name,
        WASM_HOST_ABI_IMPORT_READ_INPUT
            | WASM_HOST_ABI_IMPORT_CONFIG_LEN
            | WASM_HOST_ABI_IMPORT_READ_CONFIG
            | WASM_HOST_ABI_IMPORT_WRITE_OUTPUT
            | WASM_HOST_ABI_IMPORT_LOG
    )
}

fn resolve_wasm_host_abi_max_output_bytes(
    max_output_bytes: Option<usize>,
) -> Result<usize, String> {
    let Some(limit) = max_output_bytes else {
        return Ok(DEFAULT_WASM_HOST_ABI_MAX_OUTPUT_BYTES);
    };

    if limit == 0 {
        return Err("wasm host ABI max_output_bytes must be greater than zero".to_owned());
    }

    let max_supported_limit = i32::MAX as usize;
    if limit > max_supported_limit {
        return Err(format!(
            "wasm host ABI max_output_bytes exceeds supported limit: {limit}"
        ));
    }

    Ok(limit)
}

fn wasm_host_input_len(caller: Caller<'_, WasmHostAbiStoreData>) -> i32 {
    let input_len = caller.data().input_bytes.len();
    input_len as i32
}

fn wasm_host_read_input(mut caller: Caller<'_, WasmHostAbiStoreData>, ptr: i32, len: i32) -> i32 {
    let Some(requested_len) = checked_len(len) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    let Ok(memory) = guest_memory(&mut caller) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    let (memory_bytes, store_data) = memory.data_and_store_mut(caller);
    let input_len = store_data.input_bytes.len();
    if requested_len < input_len {
        return WASM_HOST_ABI_ERROR_CODE;
    }

    let Some(memory_range) = checked_memory_range(memory_bytes.len(), ptr, input_len) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    let input_slice = store_data.input_bytes.as_slice();
    let Some(target_slice) = memory_bytes.get_mut(memory_range) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };
    target_slice.copy_from_slice(input_slice);
    input_len as i32
}

fn wasm_host_config_len(
    mut caller: Caller<'_, WasmHostAbiStoreData>,
    key_ptr: i32,
    key_len: i32,
) -> i32 {
    let config_key = match copy_guest_config_key(&mut caller, key_ptr, key_len) {
        Ok(key) => key,
        Err(_) => {
            return WASM_HOST_ABI_ERROR_CODE;
        }
    };

    let config_value = caller.data().guest_config.get(config_key.as_str());
    let Some(config_value) = config_value else {
        return WASM_HOST_ABI_CONFIG_UNAVAILABLE_CODE;
    };

    let config_len = config_value.len();
    config_len as i32
}

fn wasm_host_read_config(
    mut caller: Caller<'_, WasmHostAbiStoreData>,
    key_ptr: i32,
    key_len: i32,
    ptr: i32,
    len: i32,
) -> i32 {
    let config_key = match copy_guest_config_key(&mut caller, key_ptr, key_len) {
        Ok(key) => key,
        Err(_) => {
            return WASM_HOST_ABI_ERROR_CODE;
        }
    };

    let Some(requested_len) = checked_len(len) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    let Ok(memory) = guest_memory(&mut caller) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    let (memory_bytes, store_data) = memory.data_and_store_mut(caller);
    let config_value = store_data.guest_config.get(config_key.as_str());
    let Some(config_value) = config_value else {
        return WASM_HOST_ABI_CONFIG_UNAVAILABLE_CODE;
    };

    let config_len = config_value.len();
    if requested_len < config_len {
        return WASM_HOST_ABI_BUFFER_TOO_SMALL_CODE;
    }

    let Some(memory_range) = checked_memory_range(memory_bytes.len(), ptr, config_len) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    let Some(target_slice) = memory_bytes.get_mut(memory_range) else {
        return WASM_HOST_ABI_ERROR_CODE;
    };

    target_slice.copy_from_slice(config_value.as_slice());
    config_len as i32
}

fn wasm_host_write_output(mut caller: Caller<'_, WasmHostAbiStoreData>, ptr: i32, len: i32) -> i32 {
    let output_limit = caller.data().max_output_bytes;
    let output_bytes = match copy_guest_bytes_with_limit(
        &mut caller,
        ptr,
        len,
        output_limit,
        "wasm guest output exceeds host ABI limit of",
    ) {
        Ok(bytes) => bytes,
        Err(reason) => {
            let store_data = caller.data_mut();
            store_data.output_error = Some(reason);
            return WASM_HOST_ABI_ERROR_CODE;
        }
    };

    let output_len = output_bytes.len();
    let store_data = caller.data_mut();
    if store_data.output_bytes.is_some() {
        store_data.output_error =
            Some("wasm guest attempted to write output more than once".to_owned());
        return WASM_HOST_ABI_ERROR_CODE;
    }

    store_data.output_bytes = Some(output_bytes);
    store_data.output_error = None;
    output_len as i32
}

fn wasm_host_log(mut caller: Caller<'_, WasmHostAbiStoreData>, ptr: i32, len: i32) -> i32 {
    let log_bytes = match copy_guest_bytes_with_limit(
        &mut caller,
        ptr,
        len,
        WASM_HOST_ABI_MAX_LOG_ENTRY_BYTES,
        "wasm guest log exceeds host ABI limit of",
    ) {
        Ok(bytes) => bytes,
        Err(_) => {
            return WASM_HOST_ABI_ERROR_CODE;
        }
    };

    let log_entry = match String::from_utf8(log_bytes) {
        Ok(entry) => entry,
        Err(_) => {
            return WASM_HOST_ABI_ERROR_CODE;
        }
    };

    let next_entry_count = caller.data().guest_logs.len().saturating_add(1);
    let next_total_bytes = caller
        .data()
        .guest_logs_bytes
        .saturating_add(log_entry.len());
    if next_entry_count > WASM_HOST_ABI_MAX_LOG_ENTRIES {
        let store_data = caller.data_mut();
        store_data.guest_logs_truncated = true;
        return WASM_HOST_ABI_LOG_DROPPED_CODE;
    }
    if next_total_bytes > WASM_HOST_ABI_MAX_LOG_TOTAL_BYTES {
        let store_data = caller.data_mut();
        store_data.guest_logs_truncated = true;
        return WASM_HOST_ABI_LOG_DROPPED_CODE;
    }

    let log_len = log_entry.len();
    let store_data = caller.data_mut();
    store_data.guest_logs.push(log_entry);
    store_data.guest_logs_bytes = next_total_bytes;
    log_len as i32
}

fn wasm_host_abort(mut caller: Caller<'_, WasmHostAbiStoreData>, code: i32) -> WasmtimeResult<()> {
    let store_data = caller.data_mut();
    store_data.abort_code = Some(code);
    wasmtime::bail!("wasm guest aborted with code {code}");
}

fn guest_memory(caller: &mut Caller<'_, WasmHostAbiStoreData>) -> Result<wasmtime::Memory, String> {
    let Some(memory_export) = caller.get_export("memory") else {
        return Err("wasm host ABI requires exported memory".to_owned());
    };

    let Extern::Memory(memory) = memory_export else {
        return Err("wasm export `memory` is not a memory".to_owned());
    };

    Ok(memory)
}

fn copy_guest_config_key(
    caller: &mut Caller<'_, WasmHostAbiStoreData>,
    ptr: i32,
    len: i32,
) -> Result<String, String> {
    let key_bytes = copy_guest_bytes_with_limit(
        caller,
        ptr,
        len,
        WASM_HOST_ABI_MAX_CONFIG_KEY_BYTES,
        "wasm guest config key exceeds host ABI limit of",
    )?;

    String::from_utf8(key_bytes)
        .map_err(|_error| "wasm guest config key must be valid UTF-8".to_owned())
}

fn copy_guest_bytes(
    caller: &mut Caller<'_, WasmHostAbiStoreData>,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, String> {
    let requested_len =
        checked_len(len).ok_or_else(|| "wasm guest provided invalid length".to_owned())?;
    let memory = guest_memory(caller)?;
    let memory_bytes = memory.data(&mut *caller);
    let memory_range = checked_memory_range(memory_bytes.len(), ptr, requested_len)
        .ok_or_else(|| "wasm guest memory access is out of bounds".to_owned())?;
    let guest_slice = memory_bytes
        .get(memory_range)
        .ok_or_else(|| "wasm guest memory access is out of bounds".to_owned())?;
    Ok(guest_slice.to_vec())
}

fn copy_guest_bytes_with_limit(
    caller: &mut Caller<'_, WasmHostAbiStoreData>,
    ptr: i32,
    len: i32,
    byte_limit: usize,
    limit_subject: &str,
) -> Result<Vec<u8>, String> {
    let requested_len =
        checked_len(len).ok_or_else(|| "wasm guest provided invalid length".to_owned())?;
    if requested_len > byte_limit {
        return Err(format!("{limit_subject} {byte_limit} bytes"));
    }

    copy_guest_bytes(caller, ptr, len)
}

fn checked_len(len: i32) -> Option<usize> {
    if len < 0 {
        return None;
    }

    Some(len as usize)
}

fn checked_memory_range(memory_len: usize, ptr: i32, len: usize) -> Option<Range<usize>> {
    if ptr < 0 {
        return None;
    }

    let offset = ptr as usize;
    let end = offset.checked_add(len)?;
    if end > memory_len {
        return None;
    }

    Some(offset..end)
}

pub(crate) fn wasm_guest_config_key_is_supported(raw_key: &str) -> bool {
    let provider_key =
        prefixed_wasm_guest_config_metadata_key(raw_key, WASM_GUEST_CONFIG_PROVIDER_PREFIX);
    if provider_key.is_some() {
        return true;
    }

    let channel_key =
        prefixed_wasm_guest_config_metadata_key(raw_key, WASM_GUEST_CONFIG_CHANNEL_PREFIX);
    channel_key.is_some()
}

pub(super) fn build_wasm_guest_config(
    provider: &ProviderConfig,
    channel: &ChannelConfig,
    guest_readable_config_keys: &BTreeSet<String>,
) -> BTreeMap<String, Vec<u8>> {
    let mut guest_config = BTreeMap::new();

    for key in guest_readable_config_keys {
        let resolved_value = resolve_wasm_guest_config_value(provider, channel, key.as_str());
        let Some(resolved_value) = resolved_value else {
            continue;
        };

        let value_bytes = resolved_value.as_bytes().to_vec();
        guest_config.insert(key.clone(), value_bytes);
    }

    guest_config
}

fn resolve_wasm_guest_config_value<'a>(
    provider: &'a ProviderConfig,
    channel: &'a ChannelConfig,
    raw_key: &str,
) -> Option<&'a str> {
    let provider_key =
        prefixed_wasm_guest_config_metadata_key(raw_key, WASM_GUEST_CONFIG_PROVIDER_PREFIX);
    if let Some(provider_key) = provider_key {
        let value = provider.metadata.get(provider_key)?;
        return Some(value.as_str());
    }

    let channel_key =
        prefixed_wasm_guest_config_metadata_key(raw_key, WASM_GUEST_CONFIG_CHANNEL_PREFIX);
    if let Some(channel_key) = channel_key {
        let value = channel.metadata.get(channel_key)?;
        return Some(value.as_str());
    }

    None
}

fn prefixed_wasm_guest_config_metadata_key<'a>(raw_key: &'a str, prefix: &str) -> Option<&'a str> {
    let metadata_key = raw_key.strip_prefix(prefix)?;
    let trimmed_key = metadata_key.trim();
    let has_outer_whitespace = trimmed_key.len() != metadata_key.len();
    let has_inner_whitespace = trimmed_key
        .chars()
        .any(|character| character.is_whitespace());
    if trimmed_key.is_empty() || has_outer_whitespace || has_inner_whitespace {
        return None;
    }

    Some(trimmed_key)
}
