use std::collections::BTreeMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::spec_runtime::*;

pub(super) fn bridge_support_policy_checksum(bridge: &BridgeSupportSpec) -> String {
    let encoded = bridge_support_policy_canonical_json(bridge);
    super::fnv1a64_hex(encoded.as_bytes())
}

pub fn bridge_support_policy_sha256(bridge: &BridgeSupportSpec) -> String {
    let encoded = bridge_support_policy_canonical_json(bridge);
    let digest = Sha256::digest(encoded.as_bytes());
    super::hex_lower(&digest)
}

fn bridge_support_policy_canonical_json(bridge: &BridgeSupportSpec) -> String {
    let mut bridges = bridge.supported_bridges.clone();
    bridges.sort();

    let mut adapter_families = bridge.supported_adapter_families.clone();
    adapter_families.sort();
    let mut allowed_commands = bridge.allowed_process_commands.clone();
    allowed_commands.sort();
    let security_scan = canonical_security_scan_value(bridge.security_scan.as_ref());

    let canonical = json!({
        "supported_bridges": bridges,
        "supported_adapter_families": adapter_families,
        "enforce_supported": bridge.enforce_supported,
        "execute_process_stdio": bridge.execute_process_stdio,
        "execute_http_json": bridge.execute_http_json,
        "allowed_process_commands": allowed_commands,
        "enforce_execution_success": bridge.enforce_execution_success,
        "security_scan": security_scan,
    });

    serde_json::to_string(&canonical).unwrap_or_default()
}

fn canonical_security_scan_value(security_scan: Option<&SecurityScanSpec>) -> Value {
    let Some(scan) = security_scan else {
        return Value::Null;
    };

    let mut keywords = scan.high_risk_metadata_keywords.clone();
    keywords.sort();

    let mut blocked_import_prefixes = scan.wasm.blocked_import_prefixes.clone();
    blocked_import_prefixes.sort();

    let mut allowed_path_prefixes = scan.wasm.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    let required_sha256_by_plugin = scan
        .wasm
        .required_sha256_by_plugin
        .iter()
        .map(|(plugin, digest)| (plugin.clone(), digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let profile_signature =
        canonical_security_scan_profile_signature_value(scan.profile_signature.as_ref());
    let siem_export = canonical_security_scan_siem_export_value(scan.siem_export.as_ref());
    let runtime = canonical_security_scan_runtime_value(&scan.runtime);

    json!({
        "enabled": scan.enabled,
        "block_on_high": scan.block_on_high,
        "profile_path": scan.profile_path,
        "profile_sha256": scan.profile_sha256,
        "profile_signature": profile_signature,
        "siem_export": siem_export,
        "runtime": runtime,
        "high_risk_metadata_keywords": keywords,
        "wasm": {
            "enabled": scan.wasm.enabled,
            "max_module_bytes": scan.wasm.max_module_bytes,
            "allow_wasi": scan.wasm.allow_wasi,
            "blocked_import_prefixes": blocked_import_prefixes,
            "allowed_path_prefixes": allowed_path_prefixes,
            "require_hash_pin": scan.wasm.require_hash_pin,
            "required_sha256_by_plugin": required_sha256_by_plugin,
        },
    })
}

fn canonical_security_scan_profile_signature_value(
    signature: Option<&SecurityProfileSignatureSpec>,
) -> Value {
    let Some(signature) = signature else {
        return Value::Null;
    };
    json!({
        "algorithm": signature.algorithm.trim().to_ascii_lowercase(),
        "public_key_base64": signature.public_key_base64,
        "signature_base64": signature.signature_base64,
    })
}

fn canonical_security_scan_siem_export_value(export: Option<&SecuritySiemExportSpec>) -> Value {
    let Some(export) = export else {
        return Value::Null;
    };
    json!({
        "enabled": export.enabled,
        "path": export.path,
        "include_findings": export.include_findings,
        "max_findings_per_record": export.max_findings_per_record,
        "fail_on_error": export.fail_on_error,
    })
}

fn canonical_security_scan_runtime_value(runtime: &SecurityRuntimeExecutionSpec) -> Value {
    let mut allowed_path_prefixes = runtime.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    json!({
        "execute_wasm_component": runtime.execute_wasm_component,
        "allowed_path_prefixes": allowed_path_prefixes,
        "max_component_bytes": runtime.max_component_bytes,
        "fuel_limit": runtime.fuel_limit,
    })
}

fn canonical_security_scan_profile_value(profile: &SecurityScanProfile) -> Value {
    let mut keywords = profile.high_risk_metadata_keywords.clone();
    keywords.sort();

    let mut blocked_import_prefixes = profile.wasm.blocked_import_prefixes.clone();
    blocked_import_prefixes.sort();

    let mut allowed_path_prefixes = profile.wasm.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    let required_sha256_by_plugin = profile
        .wasm
        .required_sha256_by_plugin
        .iter()
        .map(|(plugin, digest)| (plugin.clone(), digest.clone()))
        .collect::<BTreeMap<_, _>>();

    json!({
        "high_risk_metadata_keywords": keywords,
        "wasm": {
            "enabled": profile.wasm.enabled,
            "max_module_bytes": profile.wasm.max_module_bytes,
            "allow_wasi": profile.wasm.allow_wasi,
            "blocked_import_prefixes": blocked_import_prefixes,
            "allowed_path_prefixes": allowed_path_prefixes,
            "require_hash_pin": profile.wasm.require_hash_pin,
            "required_sha256_by_plugin": required_sha256_by_plugin,
        }
    })
}

pub fn security_scan_profile_sha256(profile: &SecurityScanProfile) -> String {
    let encoded = security_scan_profile_message(profile);
    let digest = Sha256::digest(&encoded);
    super::hex_lower(&digest)
}

pub fn security_scan_profile_message(profile: &SecurityScanProfile) -> Vec<u8> {
    let canonical = canonical_security_scan_profile_value(profile);
    serde_json::to_vec(&canonical).unwrap_or_default()
}
