use std::{collections::BTreeMap, path::Path};

use crate::plugin::{
    PluginCompatibilityShim, PluginContractDialect, PluginManifest, PluginSourceKind,
};

use super::{
    PluginBridgeKind, PluginChannelBridgeContract, PluginChannelBridgeReadiness, PluginIR,
    PluginRuntimeProfile, PluginRuntimeScaffoldDefaults,
};

pub(super) fn infer_runtime_profile(
    language: &str,
    manifest: &PluginManifest,
) -> PluginRuntimeProfile {
    let endpoint = manifest.endpoint.as_deref();
    infer_runtime_profile_from_parts(language, &manifest.metadata, endpoint)
}

fn infer_runtime_profile_from_parts(
    language: &str,
    metadata: &BTreeMap<String, String>,
    endpoint: Option<&str>,
) -> PluginRuntimeProfile {
    let source_language = metadata
        .get("source_language")
        .map(|value| normalize_language(value))
        .filter(|value| value != "unknown")
        .unwrap_or_else(|| normalize_language(language));

    let bridge_kind = metadata
        .get("bridge_kind")
        .and_then(|value| parse_bridge_kind(value))
        .or_else(|| {
            metadata
                .get("protocol")
                .filter(|value| value.eq_ignore_ascii_case("mcp"))
                .map(|_| PluginBridgeKind::McpServer)
        })
        .unwrap_or_else(|| default_bridge_kind(&source_language, endpoint));

    let adapter_family = metadata
        .get("adapter_family")
        .cloned()
        .unwrap_or_else(|| default_adapter_family(&source_language, bridge_kind));

    let entrypoint_hint = metadata
        .get("entrypoint")
        .cloned()
        .or_else(|| default_entrypoint_hint(bridge_kind, endpoint))
        .unwrap_or_else(|| "invoke".to_owned());

    PluginRuntimeProfile {
        source_language,
        bridge_kind,
        adapter_family,
        entrypoint_hint,
    }
}

pub(super) fn derive_channel_bridge_contract(
    manifest: &PluginManifest,
) -> Option<PluginChannelBridgeContract> {
    let channel_id = normalized_optional_value(manifest.channel_id.as_deref());
    let setup_surface = normalized_optional_value(
        manifest
            .setup
            .as_ref()
            .and_then(|setup| setup.surface.as_deref()),
    );
    let transport_family = normalized_manifest_metadata_value(manifest, "transport_family");
    let target_contract = normalized_manifest_metadata_value(manifest, "target_contract");
    let account_scope = normalized_manifest_metadata_value(manifest, "account_scope");
    let runtime_contract = normalized_manifest_metadata_value(manifest, "channel_runtime_contract");
    let runtime_operations =
        normalized_manifest_metadata_string_list(manifest, "channel_runtime_operations_json");
    let mut runtime_metadata_issues = Vec::new();
    let runtime_operations = match runtime_operations {
        Ok(runtime_operations) => runtime_operations,
        Err(issue) => {
            runtime_metadata_issues.push(issue);
            Vec::new()
        }
    };
    let adapter_family = normalized_manifest_metadata_value(manifest, "adapter_family");

    let has_channel_bridge_metadata =
        transport_family.is_some() || target_contract.is_some() || account_scope.is_some();
    let adapter_declares_channel_bridge = adapter_family.as_deref() == Some("channel-bridge");
    // OpenClaw packages can legitimately use setup.surface=channel without being
    // managed channel-bridge plugins. Treat explicit bridge metadata or the
    // sanctioned adapter family as the bridge declaration boundary instead.
    let declares_channel_bridge = has_channel_bridge_metadata || adapter_declares_channel_bridge;

    if !declares_channel_bridge {
        return None;
    }

    let readiness = evaluate_channel_bridge_readiness(
        channel_id.as_deref(),
        setup_surface.as_deref(),
        transport_family.as_deref(),
        target_contract.as_deref(),
    );

    Some(PluginChannelBridgeContract {
        channel_id,
        setup_surface,
        transport_family,
        target_contract,
        account_scope,
        runtime_contract,
        runtime_operations,
        runtime_metadata_issues,
        readiness,
    })
}

fn evaluate_channel_bridge_readiness(
    channel_id: Option<&str>,
    setup_surface: Option<&str>,
    transport_family: Option<&str>,
    target_contract: Option<&str>,
) -> PluginChannelBridgeReadiness {
    let mut missing_fields = Vec::new();

    if channel_id.is_none() {
        missing_fields.push("channel_id".to_owned());
    }

    if setup_surface != Some("channel") {
        missing_fields.push("setup.surface".to_owned());
    }

    if transport_family.is_none() {
        missing_fields.push("metadata.transport_family".to_owned());
    }

    if target_contract.is_none() {
        missing_fields.push("metadata.target_contract".to_owned());
    }

    let ready = missing_fields.is_empty();

    PluginChannelBridgeReadiness {
        ready,
        missing_fields,
    }
}

fn normalized_manifest_metadata_value(manifest: &PluginManifest, key: &str) -> Option<String> {
    let value = manifest.metadata.get(key);
    let value = value.map(String::as_str);
    normalized_optional_value(value)
}

fn normalized_manifest_metadata_string_list(
    manifest: &PluginManifest,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(raw_value) = manifest.metadata.get(key) else {
        return Ok(Vec::new());
    };

    let parsed_values = serde_json::from_str::<Vec<String>>(raw_value)
        .map_err(|error| format!("metadata.{key} must be valid json string array: {error}"))?;

    let mut normalized_values = Vec::new();
    for parsed_value in parsed_values {
        let trimmed_value = parsed_value.trim();
        if trimmed_value.is_empty() {
            continue;
        }

        let normalized_value = trimmed_value.to_owned();
        normalized_values.push(normalized_value);
    }

    Ok(normalized_values)
}

fn normalized_optional_value(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_owned())
}

pub fn plugin_runtime_scaffold_defaults(
    bridge_kind: PluginBridgeKind,
    source_language: Option<&str>,
) -> Result<PluginRuntimeScaffoldDefaults, String> {
    if matches!(bridge_kind, PluginBridgeKind::Unknown) {
        return Err("plugin scaffold does not support bridge_kind `unknown`".to_owned());
    }

    let normalized_source_language = source_language
        .map(normalize_language)
        .filter(|value| value != "unknown" && value != "manifest");

    let source_language_is_required = matches!(
        bridge_kind,
        PluginBridgeKind::ProcessStdio | PluginBridgeKind::NativeFfi
    );

    if source_language_is_required && normalized_source_language.is_none() {
        return Err(format!(
            "plugin scaffold requires an explicit source language for bridge_kind `{}`",
            bridge_kind.as_str()
        ));
    }

    let adapter_language = normalized_source_language.as_deref().unwrap_or("unknown");
    let adapter_family = default_adapter_family(adapter_language, bridge_kind);
    let entrypoint_hint =
        default_entrypoint_hint(bridge_kind, None).unwrap_or_else(|| "invoke".to_owned());

    Ok(PluginRuntimeScaffoldDefaults {
        source_language: normalized_source_language,
        bridge_kind,
        adapter_family,
        entrypoint_hint,
    })
}

pub(super) fn legacy_plugin_ir_dialect(source_kind: PluginSourceKind) -> PluginContractDialect {
    match source_kind {
        PluginSourceKind::PackageManifest => PluginContractDialect::LoongPackageManifest,
        PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongEmbeddedSource,
    }
}

pub(super) fn legacy_plugin_ir_runtime_profile(
    source_path: &str,
    source_kind: PluginSourceKind,
    metadata: &BTreeMap<String, String>,
    endpoint: Option<&str>,
) -> PluginRuntimeProfile {
    let source_language = legacy_plugin_ir_source_language(source_path, source_kind);
    infer_runtime_profile_from_parts(&source_language, metadata, endpoint)
}

fn legacy_plugin_ir_source_language(source_path: &str, source_kind: PluginSourceKind) -> String {
    if source_kind == PluginSourceKind::PackageManifest {
        return "unknown".to_owned();
    }

    let extension = Path::new(source_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    normalize_language(extension)
}

pub(super) fn normalize_language(language: &str) -> String {
    match language.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust".to_owned(),
        "py" => "python".to_owned(),
        "js" => "javascript".to_owned(),
        "ts" => "typescript".to_owned(),
        "go" => "go".to_owned(),
        "wasm" => "wasm".to_owned(),
        "" => "unknown".to_owned(),
        other => other.to_owned(),
    }
}

pub(super) fn parse_bridge_kind(raw: &str) -> Option<PluginBridgeKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "http_json" | "http" => Some(PluginBridgeKind::HttpJson),
        "process_stdio" | "stdio" => Some(PluginBridgeKind::ProcessStdio),
        "native_ffi" | "ffi" => Some(PluginBridgeKind::NativeFfi),
        "wasm_component" | "wasm" => Some(PluginBridgeKind::WasmComponent),
        "mcp_server" | "mcp" => Some(PluginBridgeKind::McpServer),
        "acp_bridge" | "acp" => Some(PluginBridgeKind::AcpBridge),
        "acp_runtime" | "acpx" => Some(PluginBridgeKind::AcpRuntime),
        "unknown" => Some(PluginBridgeKind::Unknown),
        _ => None,
    }
}

fn default_bridge_kind(language: &str, endpoint: Option<&str>) -> PluginBridgeKind {
    match language {
        "rust" | "go" | "c" | "cpp" | "cxx" => PluginBridgeKind::NativeFfi,
        "python" | "javascript" | "typescript" | "java" => PluginBridgeKind::ProcessStdio,
        "wasm" | "wat" => PluginBridgeKind::WasmComponent,
        _ => {
            if let Some(endpoint) = endpoint
                && (endpoint.starts_with("http://") || endpoint.starts_with("https://"))
            {
                return PluginBridgeKind::HttpJson;
            }
            PluginBridgeKind::Unknown
        }
    }
}

fn default_adapter_family(language: &str, bridge_kind: PluginBridgeKind) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => "http-adapter".to_owned(),
        PluginBridgeKind::ProcessStdio => format!("{language}-stdio-adapter"),
        PluginBridgeKind::NativeFfi => format!("{language}-ffi-adapter"),
        PluginBridgeKind::WasmComponent => "wasm-component-adapter".to_owned(),
        PluginBridgeKind::McpServer => "mcp-adapter".to_owned(),
        PluginBridgeKind::AcpBridge => "acp-bridge-adapter".to_owned(),
        PluginBridgeKind::AcpRuntime => "acp-runtime-adapter".to_owned(),
        PluginBridgeKind::Unknown => format!("{language}-unknown-adapter"),
    }
}

fn default_entrypoint_hint(
    bridge_kind: PluginBridgeKind,
    endpoint: Option<&str>,
) -> Option<String> {
    match bridge_kind {
        PluginBridgeKind::HttpJson => {
            Some(endpoint.unwrap_or("https://localhost/invoke").to_owned())
        }
        PluginBridgeKind::ProcessStdio => Some("stdin/stdout::invoke".to_owned()),
        PluginBridgeKind::NativeFfi => Some("lib::invoke".to_owned()),
        PluginBridgeKind::WasmComponent => Some("component::run".to_owned()),
        PluginBridgeKind::McpServer => Some("mcp::stdio".to_owned()),
        PluginBridgeKind::AcpBridge => Some("acp::bridge".to_owned()),
        PluginBridgeKind::AcpRuntime => Some("acp::turn".to_owned()),
        PluginBridgeKind::Unknown => None,
    }
}

pub(super) fn bootstrap_hint(ir: &PluginIR) -> String {
    let compatibility_prefix = PluginCompatibilityShim::for_mode(ir.compatibility_mode)
        .map(|shim| {
            format!(
                "enable compatibility shim `{}` ({}) and then ",
                shim.shim_id, shim.family
            )
        })
        .unwrap_or_default();

    match ir.runtime.bridge_kind {
        PluginBridgeKind::HttpJson => format!(
            "{}register http connector adapter for {} at {}",
            compatibility_prefix,
            ir.connector_name,
            ir.endpoint.as_deref().unwrap_or("https://localhost/invoke")
        ),
        PluginBridgeKind::ProcessStdio => format!(
            "{}spawn {} worker and bind stdio bridge {}",
            compatibility_prefix, ir.runtime.source_language, ir.runtime.entrypoint_hint
        ),
        PluginBridgeKind::NativeFfi => format!(
            "{}load native library adapter {} with symbol {}",
            compatibility_prefix, ir.runtime.adapter_family, ir.runtime.entrypoint_hint
        ),
        PluginBridgeKind::WasmComponent => {
            format!(
                "{}load wasm component and invoke {}",
                compatibility_prefix, ir.runtime.entrypoint_hint
            )
        }
        PluginBridgeKind::McpServer => format!(
            "{}register MCP server bridge and handshake capability schema",
            compatibility_prefix
        ),
        PluginBridgeKind::AcpBridge => format!(
            "{}register ACP bridge surface and bind the external gateway/runtime contract",
            compatibility_prefix
        ),
        PluginBridgeKind::AcpRuntime => {
            format!(
                "{}register ACP runtime backend and bind a session-aware control plane",
                compatibility_prefix
            )
        }
        PluginBridgeKind::Unknown => format!(
            "{}inspect plugin metadata and define explicit bridge_kind override",
            compatibility_prefix
        ),
    }
}
