use std::{collections::BTreeSet, fs, path::PathBuf};

use kernel::{PluginBridgeKind, PluginDescriptor, PluginScanReport};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use wasmparser::{Parser as WasmParser, Payload as WasmPayload};

use crate::spec_runtime::{
    is_process_command_allowed, SecurityFinding, SecurityFindingSeverity, SecurityScanSpec,
    WasmSecurityScanSpec,
};

use super::SecurityScanDelta;

fn build_security_finding(
    severity: SecurityFindingSeverity,
    category: impl Into<String>,
    plugin_id: impl Into<String>,
    source_path: impl Into<String>,
    message: impl Into<String>,
    evidence: Value,
) -> SecurityFinding {
    let category = category.into();
    let plugin_id = plugin_id.into();
    let source_path = source_path.into();
    let message = message.into();
    let correlation_id = security_finding_correlation_id(
        &severity,
        &category,
        &plugin_id,
        &source_path,
        &message,
        &evidence,
    );

    SecurityFinding {
        correlation_id,
        severity,
        category,
        plugin_id,
        source_path,
        message,
        evidence,
    }
}

fn security_finding_correlation_id(
    severity: &SecurityFindingSeverity,
    category: &str,
    plugin_id: &str,
    source_path: &str,
    message: &str,
    evidence: &Value,
) -> String {
    let canonical = json!({
        "severity": severity,
        "category": category,
        "plugin_id": plugin_id,
        "source_path": source_path,
        "message": message,
        "evidence": evidence,
    });
    let Ok(payload) = serde_json::to_vec(&canonical) else {
        return "sf-0000000000000000".to_owned();
    };
    let digest = Sha256::digest(&payload);
    let full = super::hex_lower(&digest);
    format!("sf-{}", &full[..16])
}

pub(super) fn evaluate_plugin_security_scan(
    report: &PluginScanReport,
    policy: &SecurityScanSpec,
    process_allowlist: &BTreeSet<String>,
) -> SecurityScanDelta {
    let mut delta = SecurityScanDelta::default();
    let metadata_keywords =
        super::approval_policy::normalize_signal_list(policy.high_risk_metadata_keywords.clone());
    let blocked_import_prefixes =
        super::approval_policy::normalize_signal_list(policy.wasm.blocked_import_prefixes.clone());
    let allowed_path_prefixes =
        super::normalize_allowed_path_prefixes(&policy.wasm.allowed_path_prefixes);

    for descriptor in &report.descriptors {
        delta.scanned_plugins = delta.scanned_plugins.saturating_add(1);
        let bridge_kind = super::descriptor_bridge_kind(descriptor);
        let metadata_finding = scan_descriptor_metadata_keywords(descriptor, &metadata_keywords);
        accumulate_security_findings(&mut delta, metadata_finding);

        match bridge_kind {
            PluginBridgeKind::ProcessStdio => {
                let findings = scan_process_stdio_security(descriptor, process_allowlist);
                accumulate_security_findings(&mut delta, findings);
            }
            PluginBridgeKind::NativeFfi => {
                let finding = build_security_finding(
                    SecurityFindingSeverity::Medium,
                    "native_ffi_review",
                    descriptor.manifest.plugin_id.clone(),
                    descriptor.path.clone(),
                    "native_ffi plugin requires manual review and stronger sandboxing",
                    json!({
                        "bridge_kind": bridge_kind.as_str(),
                        "recommendation": "prefer wasm_component for untrusted community plugins",
                    }),
                );
                accumulate_security_findings(&mut delta, vec![finding]);
            }
            PluginBridgeKind::WasmComponent if policy.wasm.enabled => {
                let findings = scan_wasm_plugin_security(
                    descriptor,
                    &policy.wasm,
                    &blocked_import_prefixes,
                    &allowed_path_prefixes,
                );
                accumulate_security_findings(&mut delta, findings);
            }
            PluginBridgeKind::HttpJson
            | PluginBridgeKind::McpServer
            | PluginBridgeKind::Unknown
            | PluginBridgeKind::WasmComponent => {}
        }
    }

    if policy.block_on_high && delta.high_findings > 0 {
        delta.block_reason = Some(format!(
            "security scan blocked {} high-risk finding(s)",
            delta.high_findings
        ));
    }

    delta
}

fn accumulate_security_findings(delta: &mut SecurityScanDelta, findings: Vec<SecurityFinding>) {
    for finding in findings {
        match finding.severity {
            SecurityFindingSeverity::High => {
                delta.high_findings = delta.high_findings.saturating_add(1)
            }
            SecurityFindingSeverity::Medium => {
                delta.medium_findings = delta.medium_findings.saturating_add(1)
            }
            SecurityFindingSeverity::Low => {
                delta.low_findings = delta.low_findings.saturating_add(1)
            }
        }
        delta.findings.push(finding);
    }
}

fn scan_descriptor_metadata_keywords(
    descriptor: &PluginDescriptor,
    keywords: &[String],
) -> Vec<SecurityFinding> {
    if keywords.is_empty() {
        return Vec::new();
    }

    let mut haystack_parts = Vec::new();
    for (key, value) in &descriptor.manifest.metadata {
        haystack_parts.push(key.to_ascii_lowercase());
        haystack_parts.push(value.to_ascii_lowercase());
    }
    let haystack = haystack_parts.join(" ");

    keywords
        .iter()
        .filter(|keyword| haystack.contains(keyword.as_str()))
        .map(|keyword| {
            build_security_finding(
                SecurityFindingSeverity::Medium,
                "metadata_keyword",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                format!("metadata contains high-risk keyword: {keyword}"),
                json!({
                    "keyword": keyword,
                    "metadata": descriptor.manifest.metadata.clone(),
                }),
            )
        })
        .collect()
}

fn scan_process_stdio_security(
    descriptor: &PluginDescriptor,
    process_allowlist: &BTreeSet<String>,
) -> Vec<SecurityFinding> {
    let mut findings = Vec::new();
    let command = descriptor.manifest.metadata.get("command").cloned();

    match command {
        Some(command) => {
            if !is_process_command_allowed(&command, process_allowlist) {
                findings.push(build_security_finding(
                    SecurityFindingSeverity::High,
                    "process_command_not_allowlisted",
                    descriptor.manifest.plugin_id.clone(),
                    descriptor.path.clone(),
                    format!("process_stdio command {command} is not in runtime allowlist"),
                    json!({
                        "command": command,
                        "allowlist": process_allowlist,
                    }),
                ));
            }
        }
        None => findings.push(build_security_finding(
            SecurityFindingSeverity::Medium,
            "process_command_missing",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "process_stdio plugin does not declare metadata.command",
            json!({
                "recommendation": "declare a fixed command and keep bridge allowlist strict",
            }),
        )),
    }

    findings
}

fn normalize_wasm_sha256_pin(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("wasm sha256 pin must not be empty".to_owned());
    }

    let lowered = trimmed.to_ascii_lowercase();
    let digest = lowered.strip_prefix("sha256:").unwrap_or(&lowered).trim();
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(
            "wasm sha256 pin must be 64 hex characters (optional prefix `sha256:`)".to_owned(),
        );
    }

    Ok(digest.to_owned())
}

fn scan_wasm_plugin_security(
    descriptor: &PluginDescriptor,
    policy: &WasmSecurityScanSpec,
    blocked_import_prefixes: &[String],
    allowed_path_prefixes: &[PathBuf],
) -> Vec<SecurityFinding> {
    let mut findings = Vec::new();
    let artifact = descriptor_wasm_artifact(descriptor);
    let Some(raw_artifact) = artifact else {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_artifact_missing",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "wasm plugin does not declare metadata.component/metadata.wasm_path/endpoint artifact",
            json!({}),
        ));
        return findings;
    };

    if raw_artifact.starts_with("http://") || raw_artifact.starts_with("https://") {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_remote_artifact",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "remote wasm artifact cannot be statically verified for local hotplug safety",
            json!({
                "artifact": raw_artifact,
            }),
        ));
        return findings;
    }

    let artifact_path = super::resolve_plugin_relative_path(&descriptor.path, &raw_artifact);
    let normalized_artifact_path = super::normalize_path_for_policy(&artifact_path);
    if !allowed_path_prefixes.is_empty()
        && !allowed_path_prefixes
            .iter()
            .any(|prefix| normalized_artifact_path.starts_with(prefix))
    {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_artifact_path_not_allowed",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "wasm artifact path is outside allowed_path_prefixes",
            json!({
                "artifact_path": normalized_artifact_path.display().to_string(),
                "allowed_path_prefixes": allowed_path_prefixes
                    .iter()
                    .map(|prefix| prefix.display().to_string())
                    .collect::<Vec<_>>(),
            }),
        ));
        return findings;
    }

    let bytes = match fs::read(&normalized_artifact_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_artifact_unreadable",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm artifact cannot be read from filesystem",
                json!({
                    "artifact_path": normalized_artifact_path.display().to_string(),
                    "error": error.to_string(),
                }),
            ));
            return findings;
        }
    };

    if bytes.len() > policy.max_module_bytes {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_module_too_large",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            format!(
                "wasm module size {} exceeds max_module_bytes {}",
                bytes.len(),
                policy.max_module_bytes
            ),
            json!({
                "artifact_path": normalized_artifact_path.display().to_string(),
                "module_size_bytes": bytes.len(),
                "max_module_bytes": policy.max_module_bytes,
            }),
        ));
    }

    if !bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_magic_header_invalid",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "artifact does not contain valid wasm magic header",
            json!({
                "artifact_path": normalized_artifact_path.display().to_string(),
            }),
        ));
        return findings;
    }

    let digest = Sha256::digest(&bytes);
    let digest_hex = super::hex_lower(&digest);

    let expected_sha256 = {
        let mut metadata_pins = Vec::new();
        for key in [
            "component_sha256",
            "component_sha256_pin",
            "component_sha256_hex",
        ] {
            let Some(raw_pin) = descriptor.manifest.metadata.get(key) else {
                continue;
            };
            match normalize_wasm_sha256_pin(raw_pin) {
                Ok(pin) => metadata_pins.push((format!("metadata.{key}"), pin)),
                Err(reason) => findings.push(build_security_finding(
                    SecurityFindingSeverity::High,
                    "wasm_sha256_pin_invalid",
                    descriptor.manifest.plugin_id.clone(),
                    descriptor.path.clone(),
                    "wasm sha256 pin format is invalid",
                    json!({
                        "source": format!("metadata.{key}"),
                        "pin": raw_pin,
                        "reason": reason,
                    }),
                )),
            }
        }

        let metadata_pin = if let Some((source, pin)) = metadata_pins.first() {
            if let Some((conflict_source, conflict_pin)) =
                metadata_pins.iter().find(|(_, candidate)| candidate != pin)
            {
                findings.push(build_security_finding(
                    SecurityFindingSeverity::High,
                    "wasm_sha256_pin_conflict",
                    descriptor.manifest.plugin_id.clone(),
                    descriptor.path.clone(),
                    "multiple metadata wasm sha256 pins conflict with each other",
                    json!({
                        "first_source": source,
                        "first_sha256": pin,
                        "conflict_source": conflict_source,
                        "conflict_sha256": conflict_pin,
                    }),
                ));
                None
            } else {
                Some(pin.clone())
            }
        } else {
            None
        };

        let policy_pin = if let Some(raw_pin) = policy
            .required_sha256_by_plugin
            .get(&descriptor.manifest.plugin_id)
        {
            match normalize_wasm_sha256_pin(raw_pin) {
                Ok(pin) => Some(pin),
                Err(reason) => {
                    findings.push(build_security_finding(
                        SecurityFindingSeverity::High,
                        "wasm_sha256_pin_invalid",
                        descriptor.manifest.plugin_id.clone(),
                        descriptor.path.clone(),
                        "wasm sha256 pin format is invalid",
                        json!({
                            "source": "security_scan.wasm.required_sha256_by_plugin",
                            "pin": raw_pin,
                            "reason": reason,
                        }),
                    ));
                    None
                }
            }
        } else {
            None
        };

        match (metadata_pin, policy_pin) {
            (Some(metadata_pin), Some(policy_pin)) => {
                if metadata_pin != policy_pin {
                    findings.push(build_security_finding(
                        SecurityFindingSeverity::High,
                        "wasm_sha256_pin_conflict",
                        descriptor.manifest.plugin_id.clone(),
                        descriptor.path.clone(),
                        "metadata wasm sha256 pin conflicts with required_sha256_by_plugin pin",
                        json!({
                            "metadata_sha256": metadata_pin,
                            "policy_sha256": policy_pin,
                        }),
                    ));
                    None
                } else {
                    Some(policy_pin)
                }
            }
            (Some(metadata_pin), None) => Some(metadata_pin),
            (None, Some(policy_pin)) => Some(policy_pin),
            (None, None) => {
                if policy.require_hash_pin {
                    findings.push(build_security_finding(
                        SecurityFindingSeverity::High,
                        "wasm_sha256_pin_missing",
                        descriptor.manifest.plugin_id.clone(),
                        descriptor.path.clone(),
                        "wasm hash pin is required but missing for plugin",
                        json!({
                            "required_sha256_by_plugin": policy.required_sha256_by_plugin,
                        }),
                    ));
                }
                None
            }
        }
    };

    if let Some(expected) = expected_sha256 {
        if !expected.eq_ignore_ascii_case(&digest_hex) {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_sha256_mismatch",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm sha256 does not match required pin",
                json!({
                    "expected_sha256": expected,
                    "actual_sha256": digest_hex,
                }),
            ));
        }
    }

    let imports = match parse_wasm_import_modules(&bytes) {
        Ok(imports) => imports,
        Err(error) => {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_parse_failed",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm parser failed while reading module imports",
                json!({
                    "artifact_path": normalized_artifact_path.display().to_string(),
                    "error": error,
                }),
            ));
            return findings;
        }
    };

    for module_name in &imports {
        let module_name_lower = module_name.to_ascii_lowercase();
        if !policy.allow_wasi && module_name_lower.starts_with("wasi") {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_wasi_import_blocked",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasi import is blocked by wasm security policy",
                json!({
                    "import_module": module_name,
                }),
            ));
        }
        if blocked_import_prefixes
            .iter()
            .any(|prefix| module_name_lower.starts_with(prefix))
        {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_import_prefix_blocked",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm import module matched blocked prefix",
                json!({
                    "import_module": module_name,
                    "blocked_import_prefixes": blocked_import_prefixes,
                }),
            ));
        }
    }

    findings.push(build_security_finding(
        SecurityFindingSeverity::Low,
        "wasm_digest_observed",
        descriptor.manifest.plugin_id.clone(),
        descriptor.path.clone(),
        "wasm artifact digest captured for audit",
        json!({
            "artifact_path": normalized_artifact_path.display().to_string(),
            "sha256": digest_hex,
            "imports": imports,
        }),
    ));

    findings
}

fn parse_wasm_import_modules(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut modules = Vec::new();
    for payload in WasmParser::new(0).parse_all(bytes) {
        match payload {
            Ok(WasmPayload::ImportSection(section)) => {
                for import in section.into_imports() {
                    let import = import.map_err(|error| error.to_string())?;
                    modules.push(import.module.to_owned());
                }
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(modules)
}

fn descriptor_wasm_artifact(descriptor: &PluginDescriptor) -> Option<String> {
    descriptor
        .manifest
        .metadata
        .get("component")
        .cloned()
        .or_else(|| descriptor.manifest.metadata.get("wasm_path").cloned())
        .or_else(|| {
            descriptor
                .manifest
                .endpoint
                .clone()
                .filter(|value| value.to_ascii_lowercase().ends_with(".wasm"))
        })
}
