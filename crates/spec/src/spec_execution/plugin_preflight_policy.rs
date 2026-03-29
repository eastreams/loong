use std::collections::BTreeSet;
use std::fs;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ed25519_dalek::{Signature as Ed25519Signature, Verifier, VerifyingKey};
use semver::VersionReq;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::BUNDLED_PLUGIN_PREFLIGHT_POLICY;
use crate::spec_runtime::{
    PluginPreflightPolicyException, PluginPreflightPolicyProfile, SecurityProfileSignatureSpec,
    default_security_profile_signature_algorithm,
};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedPluginPreflightPolicy {
    pub profile: PluginPreflightPolicyProfile,
    pub source: String,
    pub checksum: String,
    pub sha256: String,
}

pub(super) fn resolve_plugin_preflight_policy(
    policy_path: Option<&str>,
    policy_sha256: Option<&str>,
    policy_signature: Option<&SecurityProfileSignatureSpec>,
) -> Result<ResolvedPluginPreflightPolicy, String> {
    validate_plugin_preflight_policy_request(policy_path, policy_sha256, policy_signature)?;

    if let Some(path) = policy_path {
        let policy = load_plugin_preflight_policy_from_path(path).map_err(|error| {
            format!("failed to load plugin preflight policy at {path}: {error}")
        })?;
        let actual_sha256 = plugin_preflight_policy_sha256(&policy);
        if let Some(expected_sha256) = policy_sha256
            && !expected_sha256.eq_ignore_ascii_case(&actual_sha256)
        {
            return Err(format!(
                "plugin preflight policy sha256 mismatch for {path}: expected {expected_sha256}, actual {actual_sha256}",
            ));
        }
        if let Some(signature) = policy_signature {
            verify_plugin_preflight_policy_signature(&policy, signature).map_err(|error| {
                format!("plugin preflight policy signature verification failed for {path}: {error}")
            })?;
        }
        return Ok(ResolvedPluginPreflightPolicy {
            checksum: plugin_preflight_policy_checksum(&policy),
            sha256: actual_sha256,
            profile: policy,
            source: path.to_owned(),
        });
    }

    let bundled = bundled_plugin_preflight_policy()?;
    Ok(ResolvedPluginPreflightPolicy {
        checksum: plugin_preflight_policy_checksum(&bundled),
        sha256: plugin_preflight_policy_sha256(&bundled),
        profile: bundled,
        source: "bundled:plugin-preflight-medium-balanced.json".to_owned(),
    })
}

fn validate_plugin_preflight_policy_request(
    policy_path: Option<&str>,
    policy_sha256: Option<&str>,
    policy_signature: Option<&SecurityProfileSignatureSpec>,
) -> Result<(), String> {
    if policy_sha256.is_some() && policy_path.is_none() {
        return Err(
            "plugin preflight policy_sha256 requires plugin_preflight.policy_path to be set"
                .to_owned(),
        );
    }
    if policy_signature.is_some() && policy_path.is_none() {
        return Err(
            "plugin preflight policy_signature requires plugin_preflight.policy_path to be set"
                .to_owned(),
        );
    }
    if let Some(signature) = policy_signature {
        if signature.public_key_base64.trim().is_empty() {
            return Err(
                "plugin preflight policy_signature.public_key_base64 cannot be empty".to_owned(),
            );
        }
        if signature.signature_base64.trim().is_empty() {
            return Err(
                "plugin preflight policy_signature.signature_base64 cannot be empty".to_owned(),
            );
        }
    }
    Ok(())
}

pub fn load_plugin_preflight_policy_from_path(
    path: &str,
) -> Result<PluginPreflightPolicyProfile, String> {
    let content =
        fs::read_to_string(path).map_err(|error| format!("read policy failed: {error}"))?;
    let policy = serde_json::from_str::<PluginPreflightPolicyProfile>(&content)
        .map_err(|error| format!("parse policy failed: {error}"))?;
    validate_plugin_preflight_policy_profile(&policy)?;
    Ok(policy)
}

fn bundled_plugin_preflight_policy() -> Result<PluginPreflightPolicyProfile, String> {
    BUNDLED_PLUGIN_PREFLIGHT_POLICY
        .get_or_init(|| {
            let raw = include_str!("../../config/plugin-preflight-medium-balanced.json");
            let policy: PluginPreflightPolicyProfile =
                serde_json::from_str(raw).map_err(|error| {
                    format!("bundled plugin preflight policy should parse: {error}")
                })?;
            validate_plugin_preflight_policy_profile(&policy)
                .map(|_| policy)
                .map_err(|error| {
                    format!("bundled plugin preflight policy should be valid: {error}")
                })
        })
        .clone()
}

pub fn plugin_preflight_policy_checksum(profile: &PluginPreflightPolicyProfile) -> String {
    let encoded = plugin_preflight_policy_canonical_json(profile);
    super::fnv1a64_hex(encoded.as_bytes())
}

pub fn plugin_preflight_policy_sha256(profile: &PluginPreflightPolicyProfile) -> String {
    let encoded = plugin_preflight_policy_message(profile);
    let digest = Sha256::digest(&encoded);
    super::hex_lower(&digest)
}

pub fn plugin_preflight_policy_message(profile: &PluginPreflightPolicyProfile) -> Vec<u8> {
    let canonical = canonical_plugin_preflight_policy_value(profile);
    serde_json::to_vec(&canonical).unwrap_or_default()
}

fn plugin_preflight_policy_canonical_json(profile: &PluginPreflightPolicyProfile) -> String {
    let canonical = canonical_plugin_preflight_policy_value(profile);
    serde_json::to_string(&canonical).unwrap_or_default()
}

fn canonical_plugin_preflight_policy_value(profile: &PluginPreflightPolicyProfile) -> Value {
    let mut exceptions = profile.exceptions.clone();
    exceptions.sort_by(|left, right| {
        left.exception_id
            .cmp(&right.exception_id)
            .then_with(|| left.plugin_id.cmp(&right.plugin_id))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    json!({
        "policy_version": profile.policy_version,
        "runtime_activation": canonical_plugin_preflight_rule_value(&profile.runtime_activation),
        "sdk_release": canonical_plugin_preflight_rule_value(&profile.sdk_release),
        "marketplace_submission": canonical_plugin_preflight_rule_value(&profile.marketplace_submission),
        "exceptions": exceptions
            .iter()
            .map(canonical_plugin_preflight_exception_value)
            .collect::<Vec<_>>(),
    })
}

fn canonical_plugin_preflight_rule_value(
    profile: &crate::spec_runtime::PluginPreflightRuleProfile,
) -> Value {
    json!({
        "block_on_activation_blocked": profile.block_on_activation_blocked,
        "block_on_blocking_diagnostics": profile.block_on_blocking_diagnostics,
        "warn_on_advisory_diagnostics": profile.warn_on_advisory_diagnostics,
        "block_on_invalid_runtime_attestation": profile.block_on_invalid_runtime_attestation,
        "block_on_foreign_dialect_contract": profile.block_on_foreign_dialect_contract,
        "block_on_legacy_openclaw_contract": profile.block_on_legacy_openclaw_contract,
        "block_on_compatibility_shim_required": profile.block_on_compatibility_shim_required,
        "block_on_compatibility_shim_profile_mismatch": profile
            .block_on_compatibility_shim_profile_mismatch,
        "block_on_embedded_source_contract": profile.block_on_embedded_source_contract,
        "block_on_legacy_metadata_version": profile.block_on_legacy_metadata_version,
        "block_on_shadowed_embedded_source": profile.block_on_shadowed_embedded_source,
    })
}

fn canonical_plugin_preflight_exception_value(exception: &PluginPreflightPolicyException) -> Value {
    let mut profiles = exception.profiles.clone();
    profiles.sort_by_key(|profile| profile.as_str());

    let mut waive_policy_flags = exception.waive_policy_flags.clone();
    waive_policy_flags.sort();
    waive_policy_flags.dedup();

    let mut waive_diagnostic_codes = exception.waive_diagnostic_codes.clone();
    waive_diagnostic_codes.sort();
    waive_diagnostic_codes.dedup();

    json!({
        "exception_id": exception.exception_id,
        "plugin_id": exception.plugin_id,
        "plugin_version_req": exception.plugin_version_req,
        "profiles": profiles,
        "waive_policy_flags": waive_policy_flags,
        "waive_diagnostic_codes": waive_diagnostic_codes,
        "reason": exception.reason,
        "ticket_ref": exception.ticket_ref,
        "approved_by": exception.approved_by,
        "expires_at": exception.expires_at,
    })
}

fn validate_plugin_preflight_policy_profile(
    profile: &PluginPreflightPolicyProfile,
) -> Result<(), String> {
    let mut seen_exception_ids = BTreeSet::new();
    for exception in &profile.exceptions {
        validate_plugin_preflight_exception(exception, &mut seen_exception_ids)?;
    }
    Ok(())
}

fn validate_plugin_preflight_exception(
    exception: &PluginPreflightPolicyException,
    seen_exception_ids: &mut BTreeSet<String>,
) -> Result<(), String> {
    let exception_id = exception.exception_id.trim();
    if exception_id.is_empty() {
        return Err("plugin preflight exception_id cannot be empty".to_owned());
    }
    if !seen_exception_ids.insert(exception_id.to_owned()) {
        return Err(format!(
            "duplicate plugin preflight exception_id `{exception_id}`"
        ));
    }

    if exception.plugin_id.trim().is_empty() {
        return Err(format!(
            "plugin preflight exception `{exception_id}` plugin_id cannot be empty"
        ));
    }
    if exception.reason.trim().is_empty() {
        return Err(format!(
            "plugin preflight exception `{exception_id}` reason cannot be empty"
        ));
    }
    if exception.ticket_ref.trim().is_empty() {
        return Err(format!(
            "plugin preflight exception `{exception_id}` ticket_ref cannot be empty"
        ));
    }
    if exception.approved_by.trim().is_empty() {
        return Err(format!(
            "plugin preflight exception `{exception_id}` approved_by cannot be empty"
        ));
    }
    if let Some(plugin_version_req) = exception.plugin_version_req.as_deref() {
        let normalized = plugin_version_req.trim();
        if normalized.is_empty() {
            return Err(format!(
                "plugin preflight exception `{exception_id}` plugin_version_req cannot be empty when provided"
            ));
        }
        VersionReq::parse(normalized).map_err(|error| {
            format!(
                "plugin preflight exception `{exception_id}` has invalid plugin_version_req `{normalized}`: {error}"
            )
        })?;
    }
    if exception.waive_policy_flags.is_empty() && exception.waive_diagnostic_codes.is_empty() {
        return Err(format!(
            "plugin preflight exception `{exception_id}` must waive at least one policy flag or diagnostic code"
        ));
    }

    for flag in &exception.waive_policy_flags {
        let normalized = flag.trim();
        if normalized.is_empty() {
            return Err(format!(
                "plugin preflight exception `{exception_id}` contains an empty waive_policy_flags entry"
            ));
        }
        if !is_supported_plugin_preflight_waivable_flag(normalized) {
            return Err(format!(
                "plugin preflight exception `{exception_id}` cannot waive policy flag `{normalized}` because it is outside the contract-drift waiver lane"
            ));
        }
    }

    for code in &exception.waive_diagnostic_codes {
        let normalized = code.trim();
        if normalized.is_empty() {
            return Err(format!(
                "plugin preflight exception `{exception_id}` contains an empty waive_diagnostic_codes entry"
            ));
        }
        if !is_supported_plugin_preflight_waivable_diagnostic_code(normalized) {
            return Err(format!(
                "plugin preflight exception `{exception_id}` cannot waive diagnostic code `{normalized}` because it may hide a runtime fail-closed boundary"
            ));
        }
    }

    Ok(())
}

fn is_supported_plugin_preflight_waivable_flag(flag: &str) -> bool {
    matches!(
        flag,
        "foreign_dialect_contract"
            | "legacy_openclaw_contract"
            | "embedded_source_contract"
            | "legacy_metadata_version"
            | "shadowed_embedded_source"
    )
}

fn is_supported_plugin_preflight_waivable_diagnostic_code(code: &str) -> bool {
    matches!(
        code,
        "foreign_dialect_contract"
            | "legacy_openclaw_contract"
            | "embedded_source_legacy_contract"
            | "legacy_metadata_version"
            | "shadowed_embedded_source"
    )
}

fn verify_plugin_preflight_policy_signature(
    profile: &PluginPreflightPolicyProfile,
    signature: &SecurityProfileSignatureSpec,
) -> Result<(), String> {
    let algorithm = if signature.algorithm.trim().is_empty() {
        default_security_profile_signature_algorithm()
    } else {
        signature.algorithm.trim().to_ascii_lowercase()
    };
    if algorithm != "ed25519" {
        return Err(format!(
            "unsupported profile signature algorithm: {algorithm} (expected ed25519)"
        ));
    }

    let public_key_bytes = BASE64_STANDARD
        .decode(signature.public_key_base64.trim())
        .map_err(|error| format!("invalid public_key_base64: {error}"))?;
    let public_key_bytes: [u8; 32] = public_key_bytes
        .as_slice()
        .try_into()
        .map_err(|_err| "invalid ed25519 public key length (expected 32 bytes)".to_owned())?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|error| format!("invalid ed25519 public key bytes: {error}"))?;

    let signature_bytes = BASE64_STANDARD
        .decode(signature.signature_base64.trim())
        .map_err(|error| format!("invalid signature_base64: {error}"))?;
    let signature_bytes: [u8; 64] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_err| "invalid ed25519 signature length (expected 64 bytes)".to_owned())?;
    let signature = Ed25519Signature::from_bytes(&signature_bytes);

    let message = plugin_preflight_policy_message(profile);
    verifying_key
        .verify(&message, &signature)
        .map_err(|error| format!("ed25519 verification failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::spec_runtime::{PluginPreflightProfile, PluginPreflightRuleProfile};

    fn unique_temp_policy_path(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("{prefix}-{nanos}.json"))
            .display()
            .to_string()
    }

    #[test]
    fn resolve_plugin_preflight_policy_accepts_custom_path_with_matching_sha() {
        let path = unique_temp_policy_path("loongclaw-plugin-preflight-policy");
        let policy = PluginPreflightPolicyProfile {
            policy_version: Some("custom".to_owned()),
            runtime_activation: PluginPreflightRuleProfile {
                block_on_embedded_source_contract: true,
                ..PluginPreflightRuleProfile::default()
            },
            ..PluginPreflightPolicyProfile::default()
        };
        fs::write(
            &path,
            serde_json::to_string_pretty(&policy).expect("encode policy"),
        )
        .expect("write policy");

        let resolved = resolve_plugin_preflight_policy(
            Some(path.as_str()),
            Some(plugin_preflight_policy_sha256(&policy).as_str()),
            None,
        )
        .expect("resolve policy");

        assert_eq!(resolved.source, path);
        assert_eq!(resolved.profile.policy_version.as_deref(), Some("custom"));
    }

    #[test]
    fn resolve_plugin_preflight_policy_rejects_sha_mismatch() {
        let path = unique_temp_policy_path("loongclaw-plugin-preflight-policy-mismatch");
        fs::write(
            &path,
            serde_json::to_string_pretty(&PluginPreflightPolicyProfile::default())
                .expect("encode policy"),
        )
        .expect("write policy");

        let error = resolve_plugin_preflight_policy(Some(path.as_str()), Some("deadbeef"), None)
            .expect_err("sha mismatch should fail");
        assert!(error.contains("sha256 mismatch"));
    }

    #[test]
    fn bundled_plugin_preflight_policy_has_stable_sha() {
        let bundled = bundled_plugin_preflight_policy()
            .unwrap_or_else(|error| panic!("expected bundled policy to load: {error}"));
        assert!(!plugin_preflight_policy_checksum(&bundled).is_empty());
        assert_eq!(plugin_preflight_policy_sha256(&bundled).len(), 64);
        assert_eq!(
            bundled.policy_version.as_deref(),
            Some("medium-balanced-2026-03-26")
        );
    }

    #[test]
    fn resolve_plugin_preflight_policy_rejects_fail_closed_waivers() {
        let path = unique_temp_policy_path("loongclaw-plugin-preflight-policy-invalid-waiver");
        let policy = PluginPreflightPolicyProfile {
            policy_version: Some("invalid-waiver".to_owned()),
            exceptions: vec![PluginPreflightPolicyException {
                exception_id: "bad-runtime-waiver".to_owned(),
                plugin_id: "search-sdk".to_owned(),
                plugin_version_req: None,
                profiles: vec![PluginPreflightProfile::RuntimeActivation],
                waive_policy_flags: vec!["activation_blocked".to_owned()],
                waive_diagnostic_codes: Vec::new(),
                reason: "should not hide runtime activation blockers".to_owned(),
                ticket_ref: "SEC-123".to_owned(),
                approved_by: "platform-security".to_owned(),
                expires_at: None,
            }],
            ..PluginPreflightPolicyProfile::default()
        };
        fs::write(
            &path,
            serde_json::to_string_pretty(&policy).expect("encode policy"),
        )
        .expect("write policy");

        let error = resolve_plugin_preflight_policy(Some(path.as_str()), None, None)
            .expect_err("invalid waiver should fail");
        assert!(error.contains("outside the contract-drift waiver lane"));
    }

    #[test]
    fn resolve_plugin_preflight_policy_rejects_exception_without_approval_metadata() {
        let path = unique_temp_policy_path("loongclaw-plugin-preflight-policy-missing-approval");
        let policy = PluginPreflightPolicyProfile {
            policy_version: Some("missing-approval".to_owned()),
            exceptions: vec![PluginPreflightPolicyException {
                exception_id: "missing-approval".to_owned(),
                plugin_id: "search-sdk".to_owned(),
                plugin_version_req: None,
                profiles: vec![PluginPreflightProfile::SdkRelease],
                waive_policy_flags: vec!["embedded_source_contract".to_owned()],
                waive_diagnostic_codes: Vec::new(),
                reason: "temporary migration window".to_owned(),
                ticket_ref: String::new(),
                approved_by: String::new(),
                expires_at: None,
            }],
            ..PluginPreflightPolicyProfile::default()
        };
        fs::write(
            &path,
            serde_json::to_string_pretty(&policy).expect("encode policy"),
        )
        .expect("write policy");

        let error = resolve_plugin_preflight_policy(Some(path.as_str()), None, None)
            .expect_err("missing approval metadata should fail");
        assert!(error.contains("ticket_ref cannot be empty"));
    }

    #[test]
    fn resolve_plugin_preflight_policy_rejects_invalid_plugin_version_req() {
        let path = unique_temp_policy_path("loongclaw-plugin-preflight-policy-bad-version-req");
        let policy = PluginPreflightPolicyProfile {
            policy_version: Some("bad-version-req".to_owned()),
            exceptions: vec![PluginPreflightPolicyException {
                exception_id: "bad-version-req".to_owned(),
                plugin_id: "search-sdk".to_owned(),
                plugin_version_req: Some("=>1.0".to_owned()),
                profiles: vec![PluginPreflightProfile::SdkRelease],
                waive_policy_flags: vec!["embedded_source_contract".to_owned()],
                waive_diagnostic_codes: Vec::new(),
                reason: "temporary migration window".to_owned(),
                ticket_ref: "SEC-456".to_owned(),
                approved_by: "platform-security".to_owned(),
                expires_at: None,
            }],
            ..PluginPreflightPolicyProfile::default()
        };
        fs::write(
            &path,
            serde_json::to_string_pretty(&policy).expect("encode policy"),
        )
        .expect("write policy");

        let error = resolve_plugin_preflight_policy(Some(path.as_str()), None, None)
            .expect_err("invalid version req should fail");
        assert!(error.contains("invalid plugin_version_req"));
    }

    #[test]
    fn validate_plugin_preflight_policy_request_allows_empty_signature_algorithm() {
        let result = validate_plugin_preflight_policy_request(
            Some("/tmp/policy.json"),
            None,
            Some(&SecurityProfileSignatureSpec {
                algorithm: String::new(),
                public_key_base64: "cHVibGljLWtleQ==".to_owned(),
                signature_base64: "c2lnbmF0dXJl".to_owned(),
            }),
        );

        assert!(result.is_ok());
    }
}
