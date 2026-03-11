pub(super) const DEFAULT_WASM_MODULE_CACHE_CAPACITY: usize = 32;
pub(super) const MAX_WASM_MODULE_CACHE_CAPACITY: usize = 4096;
pub(super) const DEFAULT_WASM_MODULE_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MIN_WASM_MODULE_CACHE_MAX_BYTES: usize = 64 * 1024;
pub(super) const MAX_WASM_MODULE_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;

const ENV_WASM_CACHE_CAPACITY: &str = "LOONGCLAW_WASM_CACHE_CAPACITY";
const ENV_WASM_CACHE_MAX_BYTES: &str = "LOONGCLAW_WASM_CACHE_MAX_BYTES";
const ENV_WASM_SIGNALS_BASED_TRAPS: &str = "LOONGCLAW_WASM_SIGNALS_BASED_TRAPS";

pub(super) fn parse_wasm_module_cache_capacity(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_WASM_MODULE_CACHE_CAPACITY))
        .unwrap_or(DEFAULT_WASM_MODULE_CACHE_CAPACITY)
}

pub(super) fn wasm_module_cache_capacity_from_env() -> usize {
    parse_wasm_module_cache_capacity(std::env::var(ENV_WASM_CACHE_CAPACITY).ok().as_deref())
}

pub(super) fn parse_wasm_module_cache_max_bytes(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| {
            value.clamp(
                MIN_WASM_MODULE_CACHE_MAX_BYTES,
                MAX_WASM_MODULE_CACHE_MAX_BYTES,
            )
        })
        .unwrap_or(DEFAULT_WASM_MODULE_CACHE_MAX_BYTES)
}

pub(super) fn wasm_module_cache_max_bytes_from_env() -> usize {
    parse_wasm_module_cache_max_bytes(std::env::var(ENV_WASM_CACHE_MAX_BYTES).ok().as_deref())
}

pub(super) fn default_wasm_signals_based_traps() -> bool {
    !cfg!(target_os = "macos")
}

pub(super) fn parse_wasm_signals_based_traps(raw: Option<&str>) -> bool {
    let Some(value) = raw else {
        return default_wasm_signals_based_traps();
    };
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => true,
        "0" | "false" | "no" | "off" | "disabled" => false,
        _ => default_wasm_signals_based_traps(),
    }
}

pub(super) fn wasm_signals_based_traps_enabled_from_env() -> bool {
    parse_wasm_signals_based_traps(std::env::var(ENV_WASM_SIGNALS_BASED_TRAPS).ok().as_deref())
}
