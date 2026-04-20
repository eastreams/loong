use kernel::{PluginBridgeKind, PluginDescriptor};

use crate::spec_runtime::parse_bridge_kind_label;

pub(crate) fn descriptor_bridge_kind(descriptor: &PluginDescriptor) -> PluginBridgeKind {
    if let Some(raw) = descriptor.manifest.metadata.get("bridge_kind")
        && let Some(kind) = parse_bridge_kind_label(raw)
    {
        return kind;
    }

    let language = descriptor.language.trim().to_ascii_lowercase();
    match language.as_str() {
        "wasm" | "wat" => return PluginBridgeKind::WasmComponent,
        "rust" | "go" | "c" | "cpp" | "cxx" => return PluginBridgeKind::NativeFfi,
        "python" | "javascript" | "typescript" | "java" => return PluginBridgeKind::ProcessStdio,
        _ => {}
    }

    if let Some(endpoint) = descriptor.manifest.endpoint.as_deref() {
        let endpoint_lower = endpoint.to_ascii_lowercase();
        if endpoint_lower.starts_with("http://") || endpoint_lower.starts_with("https://") {
            return PluginBridgeKind::HttpJson;
        }
        if endpoint_lower.ends_with(".wasm") {
            return PluginBridgeKind::WasmComponent;
        }
    }

    PluginBridgeKind::Unknown
}
